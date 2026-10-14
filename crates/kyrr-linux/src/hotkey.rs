//! Push-to-talk key. Wayland has no way to read keys of other windows, so the
//! key is caught through the compositor: the XDG GlobalShortcuts portal
//! (GNOME 48+, KDE Plasma, Hyprland), or on an X11 session a passive key grab
//! on the root window. Without either, the user binds `kyrr --dictate` to a
//! key in the desktop settings; that start of kyrr hands it to the running
//! one, which calls [`toggle`].
//!
//! With the portal the compositor, not kyrr, owns the key: kyrr only suggests
//! it when the shortcut is bound. GNOME and KDE show their own dialog or
//! settings page where the user may change it, so [`set_key`] after [`start`]
//! has no effect there. On X11 the key follows [`set_key`] and is only grabbed
//! while [`set_active`] is on, so it reaches apps normally otherwise.
//!
//! Hyprland (xdg-desktop-portal-hyprland) offers no trigger of its own: the
//! user adds `bind = , F9, global, io.github.heast13.kyrr:dictate` to
//! hyprland.conf. The portal forwards the compositor's key press and key
//! release of that bind as `Activated` and `Deactivated`, because Hyprland
//! releases "global" binds as soon as the key goes up, whatever modifiers are
//! held by then. The shortcut names show in `hyprctl globalshortcuts`.
//! Sources: `src/portals/GlobalShortcuts.cpp` of xdg-desktop-portal-hyprland
//! and <https://wiki.hyprland.org/Configuring/Binds/>.

use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex, OnceLock, PoisonError};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use ashpd::desktop::Session;
use futures_lite::{future, stream, StreamExt};
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xkb::{self, BoolCtrl, PerClientFlag};
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask, Window};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed,
    Released,
}

/// How the key is caught, so the UI can tell the user what to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The GlobalShortcuts portal; the compositor decides the key.
    Portal,
    /// A key grab on the X server.
    X11,
    /// Not caught; the user binds `kyrr --dictate` themselves.
    None,
}

/// The id of the shortcut in the portal, and the app id the portal knows kyrr by.
const SHORTCUT: &str = "dictate";
const APP_ID: &str = "io.github.heast13.kyrr";

/// The keys the app offers: Windows virtual-key code, X11 keysym and the
/// portal's name for it.
const KEYS: [(u32, u32, &str); 4] = [
    (0x77, 0xFFC5, "F8"),
    (0x78, 0xFFC6, "F9"),
    (0x13, 0xFF13, "Pause"),
    (0xA3, 0xFFE4, "Control_R"),
];

static KEY: AtomicUsize = AtomicUsize::new(1);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELD: AtomicBool = AtomicBool::new(false);
/// The portal session came up but binding the shortcut failed, or the user
/// cancelled the desktop's dialog.
static UNBOUND: AtomicBool = AtomicBool::new(false);
/// Asks the portal thread to bind the shortcut again.
static REBIND: OnceLock<async_channel::Sender<()>> = OnceLock::new();
static SENDER: OnceLock<mpsc::Sender<KeyEvent>> = OnceLock::new();
static BACKEND: OnceLock<Backend> = OnceLock::new();
static X11: OnceLock<X11> = OnceLock::new();

/// The push-to-talk key (a Windows virtual-key code, e.g. 0x78 for F9).
/// Codes the app does not offer are ignored.
pub fn set_key(vk: u32) {
    if let Some(index) = KEYS.iter().position(|k| k.0 == vk) {
        KEY.store(index, Ordering::Relaxed);
        if let Some(x) = X11.get() {
            let _ = x.rebind();
        }
    }
}

/// Whether the key is captured. When inactive, key events are ignored and,
/// on X11, the key reaches apps normally.
pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Relaxed);
    if let Some(x) = X11.get() {
        let _ = x.rebind();
    }
}

/// The key's name for the portal and for Hyprland's binds, e.g. "F9".
pub fn key_name() -> &'static str {
    KEYS[KEY.load(Ordering::Relaxed)].2
}

/// Which way the key is caught; [`Backend::None`] until [`start`] succeeds.
pub fn backend() -> Backend {
    match BACKEND.get().copied() {
        Some(Backend::Portal) if UNBOUND.load(Ordering::Relaxed) => Backend::None,
        Some(backend) => backend,
        None => Backend::None,
    }
}

fn send(event: KeyEvent) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(event);
    }
}

fn press() {
    if ACTIVE.load(Ordering::Relaxed) && !HELD.swap(true, Ordering::Relaxed) {
        send(KeyEvent::Pressed);
    }
}

/// Reported after any press we took, even if dictation stopped meanwhile, so
/// the next press is seen again.
fn release() {
    if HELD.swap(false, Ordering::Relaxed) {
        send(KeyEvent::Released);
    }
}

/// Starts catching the key on its own threads and returns the receiver for
/// key events. Call once. `description` names the shortcut in the desktop's
/// dialog and settings. Without a way to catch the key the receiver still
/// gets what [`toggle`] sends.
pub fn start(description: &str) -> Result<mpsc::Receiver<KeyEvent>, String> {
    let (tx, rx) = mpsc::channel();
    SENDER.set(tx).map_err(|_| "hotkey already started")?;
    let x11_session = std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some();
    let backend = match start_portal(description.to_string()) {
        Ok(()) => Backend::Portal,
        Err(_) if x11_session && start_x11().is_ok() => Backend::X11,
        Err(_) => Backend::None,
    };
    let _ = BACKEND.set(backend);
    Ok(rx)
}

/// Whether the portal is there but the key is not bound to kyrr, e.g. because
/// the user cancelled the desktop's dialog. [`bind_again`] asks once more.
pub fn unbound() -> bool {
    BACKEND.get() == Some(&Backend::Portal) && UNBOUND.load(Ordering::Relaxed)
}

/// Asks the desktop again to bind the key, with its dialog where it has one.
pub fn bind_again() {
    if let Some(rebind) = REBIND.get() {
        let _ = rebind.try_send(());
    }
}

/// `kyrr --dictate`: starts dictation, or ends it when it runs.
pub fn toggle() {
    if HELD.load(Ordering::Relaxed) {
        release();
    } else {
        press();
    }
}

/// `kyrr --dictate-start` and `--dictate-stop`, bound to a key's press and
/// release where the desktop can (Hyprland's `bind` and `bindr`): hold to
/// talk without the portal.
pub fn remote_press() {
    press();
}

pub fn remote_release() {
    release();
}

/// Returns once the session exists; whether the shortcut gets bound is up to
/// the user, who may be asked and may take a while.
fn start_portal(description: String) -> Result<(), String> {
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kyrr-hotkey".into())
        .spawn(move || future::block_on(portal(ready_tx, description)))
        .map_err(|e| e.to_string())?;
    ready_rx.recv().map_err(|e| e.to_string())?
}

async fn portal(ready: mpsc::Sender<Result<(), String>>, description: String) {
    // Outside Flatpak the portal cannot tell which app this is; it matters
    // for the shortcut's name in the compositor. Older portals lack the call.
    // The portal finds the app by its launcher; one an AppImage has just
    // written may take the portal a moment to notice.
    if let Ok(id) = ashpd::AppID::try_from(APP_ID) {
        let mut registered = ashpd::register_host_app(id.clone()).await;
        if registered.as_ref().is_err_and(|e| e.to_string().contains("not found")) {
            std::thread::sleep(std::time::Duration::from_secs(1));
            registered = ashpd::register_host_app(id).await;
        }
        if let Err(e) = registered {
            eprintln!("kyrr: registering with the desktop portal: {e}");
        }
    }
    let setup = async {
        let shortcuts = GlobalShortcuts::new().await?;
        let session = shortcuts.create_session(Default::default()).await?;
        // Listen before binding so no early press is lost.
        let pressed = shortcuts.receive_activated().await?;
        let released = shortcuts.receive_deactivated().await?;
        Ok::<_, ashpd::Error>((shortcuts, session, pressed, released))
    };
    let (shortcuts, session, pressed, released) = match setup.await {
        Ok(setup) => setup,
        Err(e) => {
            eprintln!("kyrr: shortcut portal: {e}");
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let (rebind_tx, rebind_rx) = async_channel::bounded(1);
    let _ = REBIND.set(rebind_tx);
    let binding = async {
        let mut session = session;
        bind(&shortcuts, &session, &description).await;
        // GNOME turns down a second request in a session whose dialog was
        // cancelled: ask in a new one.
        while rebind_rx.recv().await.is_ok() {
            match shortcuts.create_session(Default::default()).await {
                Ok(new) => {
                    let _ = session.close().await;
                    session = new;
                }
                Err(e) => eprintln!("kyrr: shortcut portal: {e}"),
            }
            bind(&shortcuts, &session, &description).await;
        }
    };
    let pressed = pressed.filter_map(|a| (a.shortcut_id() == SHORTCUT).then_some(KeyEvent::Pressed));
    let released = released.filter_map(|d| (d.shortcut_id() == SHORTCUT).then_some(KeyEvent::Released));
    let events = async {
        let mut events = pin!(stream::or(pressed, released));
        while let Some(event) = events.next().await {
            match event {
                KeyEvent::Pressed => press(),
                KeyEvent::Released => release(),
            }
        }
    };
    future::or(binding, events).await;
}

/// Asks the desktop to bind the key to kyrr; GNOME and KDE show a dialog.
async fn bind(shortcuts: &GlobalShortcuts, session: &Session<GlobalShortcuts>, description: &str) {
    let trigger = KEYS[KEY.load(Ordering::Relaxed)].2;
    let shortcut = NewShortcut::new(SHORTCUT, description).preferred_trigger(trigger);
    let bound = async {
        shortcuts
            .bind_shortcuts(session, &[shortcut], None, Default::default())
            .await?
            .response()
    };
    match bound.await {
        Ok(_) => UNBOUND.store(false, Ordering::Relaxed),
        Err(e) => {
            eprintln!("kyrr: binding the dictation shortcut: {e}");
            UNBOUND.store(true, Ordering::Relaxed);
        }
    }
}

/// One connection serves the event thread and the requests of [`set_key`] and
/// [`set_active`]; `RustConnection` allows both at once.
struct X11 {
    conn: RustConnection,
    root: Window,
    /// The keycode currently grabbed.
    grabbed: Mutex<Option<u8>>,
}

impl X11 {
    /// Grabs the key while active, ungrabs whatever was grabbed before. A
    /// press already in progress still reports its release.
    fn rebind(&self) -> Result<(), ReplyError> {
        let mut grabbed = self.grabbed.lock().unwrap_or_else(PoisonError::into_inner);
        let locks = [
            ModMask::from(0u16),
            ModMask::LOCK,
            ModMask::M2,
            ModMask::LOCK | ModMask::M2,
        ];
        if let Some(code) = grabbed.take() {
            for modifiers in locks {
                self.conn.ungrab_key(code, self.root, modifiers)?;
            }
        }
        if ACTIVE.load(Ordering::Relaxed) {
            if let Some(code) = self.keycode(KEYS[KEY.load(Ordering::Relaxed)].1)? {
                // Caps Lock and Num Lock count as modifiers to the grab; Ctrl,
                // Alt and Shift are left out on purpose, the key is then the
                // focused app's.
                *grabbed = Some(code);
                let cookies = locks
                    .map(|m| {
                        self.conn
                            .grab_key(false, self.root, m, code, GrabMode::ASYNC, GrabMode::ASYNC)
                    })
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?;
                for cookie in cookies {
                    cookie.check()?;
                }
            }
        }
        self.conn.flush()?;
        Ok(())
    }

    fn keycode(&self, keysym: u32) -> Result<Option<u8>, ReplyError> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let per_key = usize::from(map.keysyms_per_keycode).max(1);
        Ok(map
            .keysyms
            .chunks(per_key)
            .position(|keysyms| keysyms.first() == Some(&keysym))
            .map(|i| min + i as u8))
    }
}

fn start_x11() -> Result<(), String> {
    let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn.setup().roots[screen].root;
    // Holding the key must not read as a release and press pair on every
    // auto-repeat.
    let repeat = PerClientFlag::DETECTABLE_AUTO_REPEAT;
    let detectable = || -> Result<bool, ReplyError> {
        xkb::use_extension(&conn, 1, 0)?.reply()?;
        let reply = xkb::per_client_flags(
            &conn,
            xkb::ID::USE_CORE_KBD.into(),
            repeat,
            repeat,
            BoolCtrl::default(),
            BoolCtrl::default(),
            BoolCtrl::default(),
        )?
        .reply()?;
        Ok(u32::from(reply.value) & u32::from(repeat) != 0)
    };
    if !detectable().map_err(|e| e.to_string())? {
        return Err("the X server cannot report auto-repeat".into());
    }
    let x = X11.get_or_init(|| X11 {
        conn,
        root,
        grabbed: Mutex::new(None),
    });
    std::thread::Builder::new()
        .name("kyrr-hotkey".into())
        .spawn(move || {
            while let Ok(event) = x.conn.wait_for_event() {
                match event {
                    Event::KeyPress(_) => press(),
                    Event::KeyRelease(_) => release(),
                    _ => {}
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
