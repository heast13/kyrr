//! Push-to-talk key. Wayland has no way to read keys of other windows, so the
//! key is caught through the compositor: the XDG GlobalShortcuts portal
//! (GNOME 48+, KDE Plasma, Hyprland), or on an X11 session a passive key grab
//! on the root window. Without either, `kyrr --dictate` (or
//! `--dictate-start` and `--dictate-stop`) goes on a key in the desktop's
//! own settings, by kyrr ([`crate::binds`], [`crate::gnome`]) or by hand;
//! that start of kyrr hands it to the running one, which calls [`toggle`].
//!
//! With the portal the compositor, not kyrr, owns the key: kyrr only suggests
//! it when the shortcut is bound. GNOME and KDE show their own dialog or
//! settings page where the user may change it, so [`set_key`] after [`start`]
//! has no effect there. On X11 the key follows [`set_key`] and is only grabbed
//! while [`set_active`] is on, so it reaches apps normally otherwise.
//!
//! Hyprland (xdg-desktop-portal-hyprland) offers no trigger of its own: a
//! `bind = , F9, global, io.github.heast13.kyrr:dictate` has to be in its
//! config, which [`crate::binds`] puts there. The portal forwards the compositor's key press and key
//! release of that bind as `Activated` and `Deactivated`, because Hyprland
//! releases "global" binds as soon as the key goes up, whatever modifiers are
//! held by then. The shortcut names show in `hyprctl globalshortcuts`.
//! Sources: `src/portals/GlobalShortcuts.cpp` of xdg-desktop-portal-hyprland
//! and <https://wiki.hyprland.org/Configuring/Binds/>.
//!
//! A second key starts and ends a voice memo ([`set_memo_key`]), caught the
//! same ways as shortcut "memo" (`kyrr --memo` without them, or
//! `--memo-start` and `--memo-stop` on a key's press and release). Press and
//! release are both reported; the app decides whether they mean tap or hold.

use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{mpsc, Mutex, OnceLock, PoisonError};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use ashpd::desktop::Session;
use futures_lite::{future, stream, StreamExt};
use kyrr_common::keys;
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
    /// The voice memo key went down.
    MemoPressed,
    /// The voice memo key came up.
    MemoReleased,
}

/// Where binding the key through the portal stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Not asked yet: [`bind_again`] asks.
    NotAsked,
    /// The desktop is asking the user (GNOME and KDE show a dialog).
    Asking,
    Bound,
    /// The user cancelled the dialog, or binding failed.
    Refused,
}

impl Binding {
    fn get() -> Binding {
        match BINDING.load(Ordering::Relaxed) {
            0 => Binding::NotAsked,
            1 => Binding::Asking,
            2 => Binding::Bound,
            _ => Binding::Refused,
        }
    }

    fn set(self) {
        BINDING.store(self as u8, Ordering::Relaxed);
    }
}

/// How the key is caught, so the UI can tell the user what to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The GlobalShortcuts portal; the compositor decides the key.
    Portal,
    /// A key grab on the X server.
    X11,
    /// Not caught; `kyrr --dictate` goes on a key in the desktop's own
    /// settings ([`crate::binds`], [`crate::gnome`], or by hand).
    None,
}

/// The id of the shortcut in the portal, and the app id the portal knows kyrr by.
const SHORTCUT: &str = "dictate";
const MEMO_SHORTCUT: &str = "memo";
const APP_ID: &str = "io.github.heast13.kyrr";

static KEY: AtomicU32 = AtomicU32::new(keys::DICTATION_DEFAULT);
/// The voice memo key as a Windows virtual-key code; 0 for none (memos off).
static MEMO_KEY: AtomicU32 = AtomicU32::new(0);
static MEMO_HELD: AtomicBool = AtomicBool::new(false);
/// The description of the memo shortcut in the desktop's settings.
static MEMO_DESCRIPTION: Mutex<String> = Mutex::new(String::new());
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// A key field in the settings waits for a key: nothing is grabbed, and no
/// press starts anything.
static CAPTURING: AtomicBool = AtomicBool::new(false);
static HELD: AtomicBool = AtomicBool::new(false);
/// Dictation is tapped (one press starts, the next ends), not held.
static TAP: AtomicBool = AtomicBool::new(false);
/// Where binding the shortcut stands, a [`Binding`] as u8.
static BINDING: AtomicU8 = AtomicU8::new(Binding::NotAsked as u8);
/// Presses of the key seen, whether dictation was ready or not, so the
/// setup can show that the key arrives.
static PRESSES: AtomicU32 = AtomicU32::new(0);
/// Asks the portal thread to bind the shortcut again.
static REBIND: OnceLock<async_channel::Sender<()>> = OnceLock::new();
static SENDER: OnceLock<mpsc::Sender<KeyEvent>> = OnceLock::new();
static BACKEND: OnceLock<Backend> = OnceLock::new();
static X11: OnceLock<X11> = OnceLock::new();

/// The push-to-talk key (a Windows virtual-key code, e.g. 0x78 for F9).
/// Codes the app does not offer are ignored.
pub fn set_key(vk: u32) {
    if keys::allowed(vk) {
        KEY.store(vk, Ordering::Relaxed);
        if let Some(x) = X11.get() {
            let _ = x.rebind();
        }
    }
}

/// Whether dictation is tapped rather than held: `kyrr --dictate` then
/// reports each call as a press.
pub fn set_tap(tap: bool) {
    TAP.store(tap, Ordering::Relaxed);
}

/// The voice memo key (a Windows virtual-key code, e.g. 0x77 for F8), 0
/// for none; `description` names it in the desktop's settings. With the
/// portal the desktop owns the key: a change counts from the next binding
/// ([`bind_again`], or the next start).
pub fn set_memo_key(vk: u32, description: &str) {
    MEMO_KEY.store(if keys::allowed(vk) { vk } else { 0 }, Ordering::Relaxed);
    *MEMO_DESCRIPTION.lock().unwrap_or_else(PoisonError::into_inner) = description.to_string();
    if let Some(x) = X11.get() {
        let _ = x.rebind();
    }
}

/// The voice memo key's name for the portal and for the compositors' binds.
pub fn memo_key_name() -> Option<String> {
    keys::xkb_name(MEMO_KEY.load(Ordering::Relaxed))
}

/// While a key field in the settings waits for a key, kyrr lets go of its
/// keys (on X11 they are not grabbed, so the window sees them) and takes
/// no press for dictation or a memo.
pub fn set_capture(on: bool) {
    if on {
        // A key still down would lose its release to the ungrab.
        release();
        memo_up();
    }
    CAPTURING.store(on, Ordering::Relaxed);
    if let Some(x) = X11.get() {
        let _ = x.rebind();
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
pub fn key_name() -> String {
    keys::xkb_name(KEY.load(Ordering::Relaxed)).unwrap_or_default()
}

/// Which way the key is caught; [`Backend::None`] until [`start`] succeeds.
pub fn backend() -> Backend {
    match BACKEND.get().copied() {
        Some(Backend::Portal) if Binding::get() == Binding::Refused => Backend::None,
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
    PRESSES.fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::Relaxed) && !CAPTURING.load(Ordering::Relaxed) && !HELD.swap(true, Ordering::Relaxed) {
        send(KeyEvent::Pressed);
    }
}

/// `kyrr --memo`: one press, as a key without a release would give.
pub fn memo_press() {
    if !CAPTURING.load(Ordering::Relaxed) {
        send(KeyEvent::MemoPressed);
    }
}

/// The memo key from the portal or the X server: a key held down
/// (auto-repeat) is one press.
pub fn memo_down() {
    if !CAPTURING.load(Ordering::Relaxed) && !MEMO_HELD.swap(true, Ordering::Relaxed) {
        send(KeyEvent::MemoPressed);
    }
}

/// `kyrr --memo-start`: a press whatever came before, so a release that
/// never came (a locked screen) does not leave the key dead; the app ends a
/// memo that still runs.
pub fn memo_remote_start() {
    if !CAPTURING.load(Ordering::Relaxed) {
        MEMO_HELD.store(true, Ordering::Relaxed);
        send(KeyEvent::MemoPressed);
    }
}

/// The memo key's release, and `kyrr --memo-stop`; only after a press taken.
pub fn memo_up() {
    if MEMO_HELD.swap(false, Ordering::Relaxed) {
        send(KeyEvent::MemoReleased);
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
/// dialog and settings. Without `ask_now` the portal is not asked to bind
/// the key until [`bind_again`], so its dialog comes when the user expects
/// it. Without a way to catch the key the receiver still gets what
/// [`toggle`] sends.
pub fn start(description: &str, ask_now: bool) -> Result<mpsc::Receiver<KeyEvent>, String> {
    let (tx, rx) = mpsc::channel();
    SENDER.set(tx).map_err(|_| "hotkey already started")?;
    let x11_session = std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some();
    let backend = match start_portal(description.to_string(), ask_now) {
        Ok(()) => Backend::Portal,
        Err(_) if x11_session && start_x11().is_ok() => Backend::X11,
        Err(_) => Backend::None,
    };
    let _ = BACKEND.set(backend);
    Ok(rx)
}

/// Whether the portal is there but the key is not bound to kyrr: not asked
/// yet, or the user cancelled the desktop's dialog. [`bind_again`] asks.
pub fn unbound() -> bool {
    BACKEND.get() == Some(&Backend::Portal) && matches!(Binding::get(), Binding::NotAsked | Binding::Refused)
}

/// Where binding the key through the portal stands; meaningless without it.
pub fn binding() -> Binding {
    Binding::get()
}

/// How often the key was pressed so far, also while dictation is not ready.
pub fn presses() -> u32 {
    PRESSES.load(Ordering::Relaxed)
}

/// Asks the desktop again to bind the key, with its dialog where it has one.
pub fn bind_again() {
    if let Some(rebind) = REBIND.get() {
        if rebind.try_send(()).is_ok() {
            Binding::Asking.set();
        }
    }
}

/// `kyrr --dictate`: starts dictation, or ends it when it runs. Tapped,
/// every call is a press, as the app takes them; held, a call is a press
/// and the next its release.
pub fn toggle() {
    if TAP.load(Ordering::Relaxed) {
        PRESSES.fetch_add(1, Ordering::Relaxed);
        if ACTIVE.load(Ordering::Relaxed) && !CAPTURING.load(Ordering::Relaxed) {
            HELD.store(false, Ordering::Relaxed);
            send(KeyEvent::Pressed);
        }
    } else if HELD.load(Ordering::Relaxed) {
        release();
    } else {
        press();
    }
}

/// `kyrr --dictate-start` and `--dictate-stop`, bound to a key's press and
/// release where the desktop can (Hyprland's `bind` and `bindr`): hold to
/// talk without the portal. A press whatever came before, as for the memo
/// key: a release that never came (a locked screen) does not leave the key
/// dead.
pub fn remote_press() {
    PRESSES.fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::Relaxed) && !CAPTURING.load(Ordering::Relaxed) {
        HELD.store(true, Ordering::Relaxed);
        send(KeyEvent::Pressed);
    }
}

pub fn remote_release() {
    release();
}

/// Returns once the session exists; whether the shortcut gets bound is up to
/// the user, who may be asked and may take a while.
fn start_portal(description: String, ask_now: bool) -> Result<(), String> {
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kyrr-hotkey".into())
        .spawn(move || future::block_on(portal(ready_tx, description, ask_now)))
        .map_err(|e| e.to_string())?;
    // A portal that hangs (a broken backend: each call waits 25 s for D-Bus
    // to give up) must not keep the window from opening. Should it answer
    // later after all, its key presses still arrive.
    match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err("the shortcut portal does not answer".into()),
        Err(e) => Err(e.to_string()),
    }
}

async fn portal(ready: mpsc::Sender<Result<(), String>>, description: String, ask_now: bool) {
    // The portal cannot tell which app this is; it matters for the
    // shortcut's name in the compositor. Older portals lack the call.
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
        let mut fresh = true;
        if ask_now {
            bind(&shortcuts, &session, &description).await;
            fresh = false;
        }
        // GNOME turns down a second request in a session whose dialog was
        // cancelled: ask in a new one.
        while rebind_rx.recv().await.is_ok() {
            if !fresh {
                match shortcuts.create_session(Default::default()).await {
                    Ok(new) => {
                        let _ = session.close().await;
                        session = new;
                    }
                    Err(e) => eprintln!("kyrr: shortcut portal: {e}"),
                }
            }
            fresh = false;
            bind(&shortcuts, &session, &description).await;
        }
    };
    // (shortcut, pressed)
    let pressed = pressed.map(|a| (a.shortcut_id().to_string(), true));
    let released = released.map(|d| (d.shortcut_id().to_string(), false));
    let events = async {
        let mut events = pin!(stream::or(pressed, released));
        while let Some((id, down)) = events.next().await {
            match (id.as_str(), down) {
                (SHORTCUT, true) => press(),
                (SHORTCUT, false) => release(),
                (MEMO_SHORTCUT, true) => memo_down(),
                (MEMO_SHORTCUT, false) => memo_up(),
                _ => {}
            }
        }
    };
    future::or(binding, events).await;
    // Only when the portal's connection is gone (it restarted): the key
    // does nothing until kyrr starts again. Said in the log at least.
    eprintln!("kyrr: shortcut portal: the connection ended");
}

/// Asks the desktop to bind the key (and the memo key) to kyrr; GNOME and
/// KDE show a dialog.
async fn bind(shortcuts: &GlobalShortcuts, session: &Session<GlobalShortcuts>, description: &str) {
    let trigger = key_name();
    let mut wanted = vec![NewShortcut::new(SHORTCUT, description).preferred_trigger(trigger.as_str())];
    if let Some(memo) = memo_key_name() {
        let memo_description = MEMO_DESCRIPTION.lock().unwrap_or_else(PoisonError::into_inner).clone();
        wanted.push(NewShortcut::new(MEMO_SHORTCUT, memo_description.as_str()).preferred_trigger(memo.as_str()));
    }
    Binding::Asking.set();
    let mut result = bind_these(shortcuts, session, &wanted).await;
    // The memo key must not cost the dictation key: refused for another
    // reason than the user saying no, dictation is asked for alone.
    let cancelled = matches!(
        result,
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled))
    );
    if result.is_err() && !cancelled && wanted.len() > 1 {
        eprintln!("kyrr: binding both shortcuts failed, dictation alone");
        wanted.truncate(1);
        result = bind_these(shortcuts, session, &wanted).await;
    }
    match result {
        Ok(_) => Binding::Bound.set(),
        Err(e) => {
            eprintln!("kyrr: binding the dictation shortcut: {e}");
            Binding::Refused.set();
        }
    }
}

/// One request to the portal for `wanted`.
async fn bind_these(
    shortcuts: &GlobalShortcuts,
    session: &Session<GlobalShortcuts>,
    wanted: &[NewShortcut],
) -> ashpd::Result<()> {
    shortcuts
        .bind_shortcuts(session, wanted, None, Default::default())
        .await?
        .response()
        .map(drop)
}

/// One connection serves the event thread and the requests of [`set_key`] and
/// [`set_active`]; `RustConnection` allows both at once.
struct X11 {
    conn: RustConnection,
    root: Window,
    /// The keycodes currently grabbed: dictation's and the memo key's.
    grabbed: Mutex<[Option<u8>; 2]>,
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
        for code in grabbed.iter_mut().filter_map(Option::take) {
            for modifiers in locks {
                self.conn.ungrab_key(code, self.root, modifiers)?;
            }
        }
        let grab = !CAPTURING.load(Ordering::Relaxed);
        let dictate = (grab && ACTIVE.load(Ordering::Relaxed))
            .then(|| keys::keysym(KEY.load(Ordering::Relaxed)))
            .flatten();
        let memo = grab.then(|| keys::keysym(MEMO_KEY.load(Ordering::Relaxed))).flatten();
        // Each key on its own: one that another program has grabbed already
        // costs only itself.
        let mut result = Ok(());
        for (slot, keysym) in [dictate, memo].into_iter().enumerate() {
            let Some(code) = keysym.map(|k| self.keycode(k)).transpose()?.flatten() else {
                continue;
            };
            // Caps Lock and Num Lock count as modifiers to the grab; Ctrl,
            // Alt and Shift are left out on purpose, the key is then the
            // focused app's.
            let grab = || -> Result<(), ReplyError> {
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
                Ok(())
            };
            match grab() {
                Ok(()) => grabbed[slot] = Some(code),
                Err(e) => {
                    eprintln!("kyrr: the key (keycode {code}) is taken by another program: {e}");
                    // What the failed grab got through is let go again.
                    for modifiers in locks {
                        let _ = self.conn.ungrab_key(code, self.root, modifiers);
                    }
                    result = Err(e);
                }
            }
        }
        self.conn.flush()?;
        result
    }

    /// Whether `code` is the memo key's (not dictation's).
    fn is_memo(&self, code: u8) -> bool {
        let grabbed = self.grabbed.lock().unwrap_or_else(PoisonError::into_inner);
        grabbed[1] == Some(code) && grabbed[0] != Some(code)
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
        grabbed: Mutex::new([None; 2]),
    });
    std::thread::Builder::new()
        .name("kyrr-hotkey".into())
        .spawn(move || {
            while let Ok(event) = x.conn.wait_for_event() {
                match event {
                    Event::KeyPress(e) if x.is_memo(e.detail) => memo_down(),
                    Event::KeyRelease(e) if x.is_memo(e.detail) => memo_up(),
                    Event::KeyPress(_) => press(),
                    Event::KeyRelease(_) => release(),
                    _ => {}
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
