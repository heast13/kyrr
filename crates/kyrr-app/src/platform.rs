//! Everything the app needs from the operating system, behind one interface:
//! Windows (the filter is an APO in the audio service) and Linux (the filter
//! runs in kyrr on a virtual PipeWire microphone).

use kyrr_common::FilterSettings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    /// Full endpoint id (what Windows' audio API takes); the node name on
    /// Linux.
    pub id: String,
    pub guid: String,
    pub name: String,
    /// The one Windows uses unless an app asks for another.
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum FilterStatus {
    /// The PC cannot run the filter (Windows on ARM).
    Unsupported,
    NotInstalled,
    NotAttached,
    /// Attached to this microphone (endpoint GUID); the APO reported nothing yet.
    Waiting(String),
    Running(String, String),
    Passthrough(String, String),
    /// Attached, but the effect slot no longer holds kyrr (a Windows or
    /// driver update reset it).
    Detached(String),
}

/// The microphone level the audio filter publishes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    /// Changes while an app uses the microphone.
    pub heartbeat: u32,
    /// Peak before and after kyrr, linear.
    pub before: f32,
    pub after: f32,
    /// What the automatic volume adds right now, in dB.
    pub gain_db: f32,
}

/// The same take before and after kyrr, mono.
#[derive(Debug)]
pub struct Take {
    pub rate: u32,
    pub before: Vec<f32>,
    pub after: Vec<f32>,
}

/// What the user has to do so the dictation key reaches kyrr.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(windows, allow(dead_code))]
pub enum KeyNote {
    /// Nothing: kyrr catches the key itself.
    None,
    /// The desktop owns the key (its shortcut portal) and can change it.
    Desktop(Desktop),
    /// The desktop could own the key, but has not bound it to kyrr: the user
    /// cancelled its dialog, or it failed. [`bind_key_again`] asks again.
    Unbound,
    /// Hyprland binds the key and kyrr could not add the lines itself: they
    /// belong in this config file.
    Hyprland { file: String, lines: String },
    /// Another bind of the user's in this config file already has the key.
    HyprlandTaken { file: String, key: String },
    /// Hyprland without the shortcut portal: lines that run kyrr on the
    /// key's press and release, for this config file.
    HyprlandCommands { file: String, lines: String },
    /// Nothing catches keys: bind this command to a key.
    Command(String),
}

/// Where getting the desktop's OK for the key stands (GNOME, KDE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub enum KeyBinding {
    NotAsked,
    Asking,
    Bound,
    Refused,
}

/// Desktops whose own settings change the key, so the note can say where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub enum Desktop {
    Gnome,
    Kde,
    Other,
}

#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum SetupError {
    Declined,
    /// The microphone to move to is gone (unplugged while the permission
    /// prompt was open): nothing was changed.
    Gone,
    Failed(String),
}

#[cfg(windows)]
mod imp {
    use super::*;
    use kyrr_windows::{autostart, devices, elevate, filter, hotkey, setup, typing};

    pub fn microphones() -> Vec<Microphone> {
        devices::microphones()
            .unwrap_or_default()
            .into_iter()
            .map(|m| Microphone {
                id: m.id,
                guid: m.guid,
                name: m.name,
                is_default: m.is_default,
            })
            .collect()
    }

    /// Whether Windows has this microphone muted (full endpoint id).
    pub fn microphone_muted(id: &str) -> Option<bool> {
        devices::is_muted(id)
    }

    /// Takes Windows' mute off the microphone.
    pub fn unmute_microphone(id: &str) -> bool {
        devices::unmute(id)
    }

    pub fn filter_status() -> FilterStatus {
        if !setup::filter_supported() {
            return FilterStatus::Unsupported;
        }
        if !setup::is_installed() {
            return FilterStatus::NotInstalled;
        }
        let Some(guid) = setup::attached().into_iter().next() else {
            return FilterStatus::NotAttached;
        };
        if !setup::slot_holds_kyrr(&guid) {
            return FilterStatus::Detached(guid);
        }
        match filter::status() {
            Some(s) if s.state == "running" => FilterStatus::Running(guid, format!("{}, {}", s.engine, s.format)),
            Some(s) if s.state.starts_with("passthrough") => FilterStatus::Passthrough(guid, s.state),
            _ => FilterStatus::Waiting(guid),
        }
    }

    pub fn filter_settings() -> FilterSettings {
        filter::settings()
    }

    pub fn set_filter_values(values: &[(&str, u32)]) -> Result<(), String> {
        filter::set_values(values)
    }

    /// Moves the filter to microphone `guid`, or puts it back there: one
    /// permission prompt, one short audio restart.
    pub fn attach(guid: &str) -> Result<(), SetupError> {
        run(&["--setup", "move", guid])
    }

    fn run(args: &[&str]) -> Result<(), SetupError> {
        match elevate::run_self_elevated(args) {
            Ok(0) => Ok(()),
            Ok(code) if code == kyrr_windows::EXIT_GONE as u32 => Err(SetupError::Gone),
            Ok(code) => Err(SetupError::Failed(format!("setup failed ({code})"))),
            Err(elevate::ElevateError::Declined) => Err(SetupError::Declined),
            Err(elevate::ElevateError::Failed(e)) => Err(SetupError::Failed(e)),
        }
    }

    pub fn paste_text(text: &str) -> bool {
        typing::paste_text(text)
    }

    pub fn copy_to_clipboard(text: &str) -> bool {
        typing::copy_to_clipboard(text)
    }

    pub fn set_hotkey_key(vk: u32) {
        hotkey::set_key(vk);
    }

    /// Shows the license notices that ship in the install folder.
    pub fn open_licenses() {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()))
        {
            let _ = std::process::Command::new("notepad.exe")
                .arg(dir.join("THIRD_PARTY.md"))
                .spawn();
        }
    }

    /// Opens a web page in the default browser.
    pub fn open_url(url: &str) {
        let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
    }

    /// Push-to-talk events; `None` when the key could not be hooked.
    pub fn start_hotkey(_description: &str, _setup_done: bool) -> Option<std::sync::mpsc::Receiver<hotkey::KeyEvent>> {
        hotkey::start().ok()
    }

    /// The hook needs no one's OK.
    pub fn key_binding() -> KeyBinding {
        KeyBinding::Bound
    }

    /// Not counted: the setup shows no key test on Windows.
    pub fn key_presses() -> u32 {
        0
    }

    /// Typing needs no one's OK.
    pub fn allow_typing() -> bool {
        true
    }

    /// The hook needs no one's OK.
    pub fn bind_key_again() {}

    /// kyrr's own settings change the key.
    pub fn open_key_settings() -> bool {
        false
    }

    pub fn can_open_key_settings() -> bool {
        false
    }

    /// Typing needs no one's OK.
    pub fn typing_needs_permission() -> bool {
        false
    }

    pub fn set_hotkey_active(active: bool) {
        hotkey::set_active(active);
    }

    pub fn set_key_wanted(_wanted: bool) {}

    pub fn enable_autostart() {
        if !autostart::is_enabled() {
            let _ = autostart::enable();
        }
    }

    pub fn autostart_enabled() -> bool {
        autostart::is_enabled()
    }

    pub fn set_autostart(on: bool) {
        let _ = if on { autostart::enable() } else { autostart::disable() };
    }

    /// Runs a downloaded kyrr installer as an update: no wizard, only its
    /// progress window, and kyrr starts again when it is done. Windows asks
    /// for permission. Blocks: the installer ends this kyrr on its way, so
    /// coming back means nothing was installed (permission declined).
    pub fn run_installer(path: &std::path::Path) -> Result<(), SetupError> {
        match elevate::run(path, &["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/restart=1"]) {
            Ok(_) => Ok(()),
            Err(elevate::ElevateError::Declined) => Err(SetupError::Declined),
            Err(elevate::ElevateError::Failed(e)) => Err(SetupError::Failed(e)),
        }
    }

    /// Puts the window in front of everything and makes it the active one.
    pub fn bring_to_front(window: &slint::Window) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = window.window_handle();
        if let Ok(h) = handle.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                kyrr_windows::window::bring_to_front(w.hwnd.get());
            }
        }
    }

    /// Left, top, right and bottom of the free screen at a point, in
    /// physical pixels.
    pub fn work_area(window: &slint::Window) -> Option<(i32, i32, i32, i32)> {
        let pos = window.position();
        kyrr_windows::screen::work_area(pos.x, pos.y)
    }

    pub use kyrr_windows::hotkey::KeyEvent;

    /// The filter lives in the audio service; nothing to stop.
    pub fn shutdown() {}

    pub fn has_tray() -> bool {
        true
    }

    pub fn can_minimize() -> bool {
        true
    }

    pub fn key_note() -> KeyNote {
        KeyNote::None
    }

    /// The live monitor the audio filter shares; `None` until it ran once.
    pub struct Monitor(kyrr_windows::monitor::Monitor);

    impl Monitor {
        pub fn open() -> Option<Monitor> {
            kyrr_windows::monitor::Monitor::open().map(Monitor)
        }

        pub fn levels(&self) -> Levels {
            let l = self.0.levels();
            Levels {
                heartbeat: l.heartbeat,
                before: l.before,
                after: l.after,
                gain_db: l.gain_db,
            }
        }

        pub fn request_probe(&self) -> u32 {
            self.0.request_probe()
        }

        pub fn probe(&self, id: u32) -> Option<Take> {
            self.0.probe(id).map(|p| Take {
                rate: p.rate,
                before: p.before,
                after: p.after,
            })
        }
    }
}

#[cfg(not(windows))]
mod imp {
    //! Linux: the filter runs inside kyrr on the virtual microphone "kyrr"
    //! (see `engine`); there is nothing to install, so "attaching" starts it
    //! on the chosen microphone and remembers that.

    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use kyrr_linux::hyprland;
    use kyrr_linux::mic::Status;
    use serde::{Deserialize, Serialize};

    use crate::engine::Engine;

    pub use kyrr_linux::hotkey::KeyEvent;

    /// What the filter needs across restarts.
    #[derive(Default, Serialize, Deserialize)]
    #[serde(default)]
    struct Saved {
        /// PipeWire node name of the microphone "kyrr" records from; none
        /// until the first setup.
        microphone: Option<String>,
        /// The filter settings as their stored values.
        filter: BTreeMap<String, u32>,
        /// Lets the keyboard portal type without asking again.
        portal_token: Option<String>,
        /// In a Flatpak: the Background portal starts kyrr at login. The
        /// portal's own record is out of the sandbox's reach.
        autostart: bool,
    }

    impl Saved {
        fn file() -> PathBuf {
            crate::config::config_dir().join("filter.json")
        }

        /// A file that does not read is kept as filter.json.bak and kyrr
        /// starts afresh: the setup asks for the microphone again.
        fn load() -> Saved {
            let path = Saved::file();
            let Ok(bytes) = std::fs::read(&path) else {
                return Saved::default();
            };
            match serde_json::from_slice(&bytes) {
                Ok(saved) => saved,
                Err(e) => {
                    eprintln!("kyrr: {} does not read ({e}); kept as filter.json.bak", path.display());
                    let _ = std::fs::rename(&path, path.with_extension("json.bak"));
                    Saved::default()
                }
            }
        }

        fn save(&self) {
            let path = Saved::file();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Only the user may read it: it holds the keyboard portal's token.
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            if let Ok(json) = serde_json::to_vec_pretty(self) {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&path);
                if let Ok(mut file) = file {
                    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
                    let _ = file.write_all(&json);
                }
            }
        }

        fn settings(&self) -> FilterSettings {
            FilterSettings::from_dwords(|name| self.filter.get(name).copied())
        }
    }

    enum Filter {
        Off,
        Starting,
        On(Engine),
        Failed(String),
    }

    struct State {
        saved: Saved,
        filter: Filter,
    }

    fn state() -> MutexGuard<'static, State> {
        static STATE: OnceLock<Mutex<State>> = OnceLock::new();
        let state = STATE.get_or_init(|| {
            let saved = Saved::load();
            kyrr_linux::typing::set_restore_token(saved.portal_token.clone());
            let start = saved.microphone.clone();
            let state = Mutex::new(State {
                saved,
                filter: Filter::Off,
            });
            if let Some(mic) = start {
                std::thread::spawn(move || start_filter(mic));
            }
            state
        });
        state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Starts the filter on `microphone`; loading the noise model takes a
    /// moment, so this runs off the UI thread.
    fn start_filter(microphone: String) {
        let settings = {
            let mut s = state();
            if matches!(s.filter, Filter::Starting | Filter::On(_)) {
                return;
            }
            s.filter = Filter::Starting;
            s.saved.settings()
        };
        let filter = match Engine::start(Some(microphone), &settings) {
            Ok(engine) => Filter::On(engine),
            Err(e) => {
                eprintln!("kyrr: the noise filter did not start: {e}");
                Filter::Failed(e)
            }
        };
        state().filter = filter;
    }

    pub fn microphones() -> Vec<Microphone> {
        match &state().filter {
            // "kyrr" itself is the system's default; the filter follows the
            // microphone the system would use without it.
            Filter::On(engine) => {
                let default = engine.system_default();
                engine
                    .microphones()
                    .into_iter()
                    .map(|m| Microphone {
                        is_default: default.as_ref() == Some(&m.name),
                        id: m.name.clone(),
                        guid: m.name,
                        name: m.description,
                    })
                    .collect()
            }
            // Before the first setup: the microphones PipeWire offers, read
            // without starting anything.
            _ => kyrr_linux::mic::list()
                .unwrap_or_default()
                .into_iter()
                .map(|m| Microphone {
                    id: m.name.clone(),
                    guid: m.name,
                    name: m.description,
                    is_default: false,
                })
                .collect(),
        }
    }

    /// Whether the microphone (node name) is muted in the sound settings,
    /// asked of WirePlumber's `wpctl`; `None` where that is out of reach
    /// (the Flatpak sandbox) or the filter does not run.
    pub fn microphone_muted(name: &str) -> Option<bool> {
        let id = mic_node(name)?;
        let out = std::process::Command::new("wpctl")
            .args(["get-volume", &id.to_string()])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).contains("[MUTED]"))
    }

    pub fn unmute_microphone(name: &str) -> bool {
        let Some(id) = mic_node(name) else {
            return false;
        };
        std::process::Command::new("wpctl")
            .args(["set-mute", &id.to_string(), "0"])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn mic_node(name: &str) -> Option<u32> {
        if flatpak() {
            return None;
        }
        match &state().filter {
            Filter::On(engine) => engine.node_id(name),
            _ => None,
        }
    }

    pub fn filter_status() -> FilterStatus {
        let s = state();
        let wanted = s.saved.microphone.clone().unwrap_or_default();
        match &s.filter {
            // Nothing chosen yet: the setup asks for a microphone.
            Filter::Off if s.saved.microphone.is_none() => FilterStatus::NotAttached,
            Filter::Off | Filter::Starting => FilterStatus::Waiting(wanted),
            Filter::Failed(_) => FilterStatus::Unsupported,
            Filter::On(engine) => match engine.status() {
                Status::Starting => FilterStatus::Waiting(wanted),
                // Unplugged: shown as "disconnected" until it is back.
                Status::NoMicrophone => FilterStatus::Waiting(wanted),
                Status::Failed(e) => FilterStatus::Passthrough(wanted, e),
                Status::Running(mic) => {
                    let label = engine.engine();
                    if label.starts_with("passthrough") {
                        FilterStatus::Passthrough(mic, label)
                    } else {
                        FilterStatus::Running(mic, format!("{label}, 48 kHz, 1 ch"))
                    }
                }
            },
        }
    }

    pub fn filter_settings() -> FilterSettings {
        state().saved.settings()
    }

    pub fn set_filter_values(values: &[(&str, u32)]) -> Result<(), String> {
        let mut s = state();
        for (name, value) in values {
            s.saved.filter.insert(name.to_string(), *value);
        }
        s.saved.save();
        let settings = s.saved.settings();
        if let Filter::On(engine) = &s.filter {
            engine.set_settings(&settings);
        }
        Ok(())
    }

    /// Records from microphone `guid` (a PipeWire node name) from now on;
    /// starts the filter the first time.
    pub fn attach(guid: &str) -> Result<(), SetupError> {
        {
            let mut s = state();
            // Pulled out meanwhile: stay on the microphone in use, which
            // keeps working, instead of waiting for one that is gone.
            if let Filter::On(engine) = &s.filter {
                if !engine.microphones().iter().any(|m| m.name == guid) {
                    return Err(SetupError::Gone);
                }
            }
            s.saved.microphone = Some(guid.to_string());
            s.saved.save();
            if let Filter::On(engine) = &s.filter {
                engine.use_microphone(guid);
                return Ok(());
            }
            if matches!(s.filter, Filter::Failed(_)) {
                s.filter = Filter::Off;
            }
        }
        start_filter(guid.to_string());
        match &state().filter {
            Filter::Failed(e) => Err(SetupError::Failed(e.clone())),
            _ => Ok(()),
        }
    }

    /// Gives the default microphone back before kyrr ends.
    pub fn shutdown() {
        state().filter = Filter::Off;
    }

    /// Whether the desktop shows kyrr's tray icon. Not from a Flatpak: the
    /// tray (Slint's, through ksni) claims a D-Bus name the sandbox may not
    /// own, so it never registers there.
    pub fn has_tray() -> bool {
        !flatpak() && kyrr_linux::tray::available()
    }

    /// Whether the desktop minimizes windows. Tiling compositors (Hyprland,
    /// sway, niri) ignore the request, so only desktops known to have a
    /// taskbar or dock to minimize to count.
    pub fn can_minimize() -> bool {
        const MINIMIZING: [&str; 12] = [
            "gnome", "kde", "cinnamon", "xfce", "mate", "budgie", "unity", "pantheon", "lxqt", "lxde", "deepin",
            "cosmic",
        ];
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase();
        desktop.split(':').any(|d| MINIMIZING.iter().any(|m| d.contains(m)))
    }

    pub fn paste_text(text: &str) -> bool {
        let typed = kyrr_linux::typing::type_text(text);
        let token = kyrr_linux::typing::restore_token();
        let mut s = state();
        if token != s.saved.portal_token {
            s.saved.portal_token = token;
            s.saved.save();
        }
        typed
    }

    pub fn copy_to_clipboard(text: &str) -> bool {
        kyrr_linux::typing::copy_to_clipboard(text)
    }

    pub fn set_hotkey_key(vk: u32) {
        kyrr_linux::hotkey::set_key(vk);
    }

    /// `description` names the shortcut in the desktop's dialog and settings.
    /// Before the setup is done, GNOME and KDE are asked for the key only
    /// when the setup gets to it, not out of the blue at the start.
    pub fn start_hotkey(description: &str, setup_done: bool) -> Option<std::sync::mpsc::Receiver<KeyEvent>> {
        integrate_appimage();
        kyrr_linux::hotkey::start(description, setup_done || desktop() == Desktop::Other).ok()
    }

    pub fn key_binding() -> KeyBinding {
        use kyrr_linux::hotkey::Binding;
        match kyrr_linux::hotkey::binding() {
            Binding::NotAsked => KeyBinding::NotAsked,
            Binding::Asking => KeyBinding::Asking,
            Binding::Bound => KeyBinding::Bound,
            Binding::Refused => KeyBinding::Refused,
        }
    }

    pub fn key_presses() -> u32 {
        kyrr_linux::hotkey::presses()
    }

    /// Asks for the permission to type now (GNOME and KDE show a dialog) and
    /// keeps the answer. Blocks until answered.
    pub fn allow_typing() -> bool {
        let allowed = kyrr_linux::typing::prepare();
        let token = kyrr_linux::typing::restore_token();
        let mut s = state();
        if token != s.saved.portal_token {
            s.saved.portal_token = token;
            s.saved.save();
        }
        allowed
    }

    pub fn bind_key_again() {
        kyrr_linux::hotkey::bind_again();
    }

    /// Opens the page of the desktop's settings where kyrr's key is changed.
    /// False where kyrr cannot open it (Flatpak, other desktops).
    pub fn open_key_settings() -> bool {
        let command: &[&str] = match desktop() {
            Desktop::Gnome => &["gnome-control-center", "applications", "io.github.heast13.kyrr"],
            Desktop::Kde => &["systemsettings", "kcm_keys"],
            Desktop::Other => return false,
        };
        !flatpak()
            && std::process::Command::new(command[0])
                .args(&command[1..])
                .spawn()
                .is_ok()
    }

    /// Whether [`open_key_settings`] can work, to offer it at all.
    pub fn can_open_key_settings() -> bool {
        let program = match desktop() {
            Desktop::Gnome => "gnome-control-center",
            Desktop::Kde => "systemsettings",
            Desktop::Other => return false,
        };
        !flatpak()
            && std::env::var_os("PATH")
                .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    }

    /// Whether the first dictation will bring up the desktop's "Remote
    /// Desktop" dialog (GNOME, KDE), which kyrr needs to type there. Once
    /// granted, the portal's token keeps the answer.
    pub fn typing_needs_permission() -> bool {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty());
        wayland && desktop() != Desktop::Other && state().saved.portal_token.is_none()
    }

    fn desktop() -> Desktop {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if desktop.contains("gnome") {
            Desktop::Gnome
        } else if desktop.contains("kde") {
            Desktop::Kde
        } else {
            Desktop::Other
        }
    }

    /// Gives an AppImage the launcher a package would install. Without it the
    /// desktop portal does not know kyrr's app id and refuses the shortcut;
    /// it also puts kyrr in the app menu. Rewritten when the AppImage moves,
    /// and hidden by the menu once the file is gone (TryExec).
    fn integrate_appimage() {
        if flatpak() {
            return;
        }
        let (Some(appimage), Some(appdir)) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR")) else {
            return;
        };
        let appimage = appimage.to_string_lossy().into_owned();
        let usr = PathBuf::from(appdir).join("usr/share");
        let data = crate::config::data_home();
        let id = "io.github.heast13.kyrr";
        let Ok(bundled) = std::fs::read_to_string(usr.join(format!("applications/{id}.desktop"))) else {
            return;
        };
        let mut entry = String::with_capacity(bundled.len() + 2 * appimage.len());
        for line in bundled.lines() {
            if line.starts_with("Exec=") {
                entry.push_str(&format!("TryExec={}\n", appimage.replace('\\', "\\\\")));
                entry.push_str(&format!("Exec=\"{}\"\n", exec_quoted(&appimage)));
            } else {
                entry.push_str(line);
                entry.push('\n');
            }
        }
        for icon in [
            "256x256/apps/io.github.heast13.kyrr.png",
            "scalable/apps/io.github.heast13.kyrr.svg",
        ] {
            write_if_changed(
                &data.join("icons/hicolor").join(icon),
                std::fs::read(usr.join("icons/hicolor").join(icon)).ok(),
            );
        }
        write_if_changed(
            &data.join(format!("applications/{id}.desktop")),
            Some(entry.into_bytes()),
        );
        // The start at sign-in follows a moved AppImage too.
        if autostart_enabled() {
            set_autostart(true);
        }
    }

    fn write_if_changed(path: &std::path::Path, contents: Option<Vec<u8>>) {
        let Some(contents) = contents else {
            return;
        };
        if std::fs::read(path).is_ok_and(|old| old == contents) {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, contents) {
            eprintln!("kyrr: writing {}: {e}", path.display());
        }
    }

    pub fn set_hotkey_active(active: bool) {
        kyrr_linux::hotkey::set_active(active);
    }

    pub fn key_note() -> KeyNote {
        use kyrr_linux::hotkey::{self, Backend};
        let hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
            || std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_lowercase().contains("hyprland"));
        let backend = hotkey::backend();
        if !hyprland && hotkey::unbound() {
            return KeyNote::Unbound;
        }
        if !hyprland || backend == Backend::X11 {
            return match backend {
                Backend::X11 => KeyNote::None,
                Backend::Portal => KeyNote::Desktop(desktop()),
                Backend::None => KeyNote::Command(dictate_command("--dictate")),
            };
        }
        let keys = hyprland::Keys {
            key: hotkey::key_name().to_string(),
            portal: backend == Backend::Portal,
            start: dictate_command("--dictate-start"),
            stop: dictate_command("--dictate-stop"),
        };
        let dir = hyprland::config_dir();
        // Dictation is off: nothing to bind, so nothing to say.
        let Some(outcome) = hyprland_outcome(&dir, &keys) else {
            return KeyNote::None;
        };
        match outcome {
            hyprland::Outcome::Bound { .. } => KeyNote::None,
            hyprland::Outcome::Taken { file } => KeyNote::HyprlandTaken {
                file: hyprland::display(&file),
                key: keys.key,
            },
            hyprland::Outcome::Failed => {
                let (file, syntax) = match hyprland::target(&dir) {
                    Some(t) => (t.path, t.syntax),
                    None => (dir.join("hyprland.conf"), hyprland::Syntax::Conf),
                };
                let (file, lines) = (hyprland::display(&file), hyprland::lines(syntax, &keys));
                if keys.portal {
                    KeyNote::Hyprland { file, lines }
                } else {
                    KeyNote::HyprlandCommands { file, lines }
                }
            }
        }
    }

    /// Whether dictation is on, so kyrr may set its key up in Hyprland.
    pub fn set_key_wanted(wanted: bool) {
        KEY_WANTED.store(wanted, std::sync::atomic::Ordering::Relaxed);
    }

    static KEY_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static HYPRLAND: std::sync::Mutex<Option<(hyprland::Keys, hyprland::Outcome)>> = std::sync::Mutex::new(None);

    /// Puts the key into Hyprland's config while dictation is on. Done once
    /// per key and way of catching it: the answer stands until one changes, so
    /// a key freed in the config is noticed after the next start.
    fn hyprland_outcome(dir: &std::path::Path, keys: &hyprland::Keys) -> Option<hyprland::Outcome> {
        if !KEY_WANTED.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let mut memo = HYPRLAND.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((seen, outcome)) = memo.as_ref() {
            if seen == keys {
                return Some(outcome.clone());
            }
        }
        let outcome = hyprland::ensure(dir, keys);
        if matches!(outcome, hyprland::Outcome::Bound { written: true, .. }) && !flatpak() {
            // Hyprland sees the change by itself; this only makes it quicker.
            std::thread::spawn(|| {
                let _ = std::process::Command::new("hyprctl").arg("reload").status();
            });
        }
        *memo = Some((keys.clone(), outcome.clone()));
        Some(outcome)
    }

    /// How this kyrr is started with `flag`: through Flatpak, the
    /// AppImage's path, or the program's own.
    fn dictate_command(flag: &str) -> String {
        if let Ok(id) = std::env::var("FLATPAK_ID") {
            return format!("flatpak run {id} {flag}");
        }
        let program = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .filter(|p| !p.starts_with("/usr/bin"))
            .map_or_else(|| "kyrr".to_string(), |p| p.display().to_string());
        format!("{program} {flag}")
    }

    /// Shows the license notices that ship with kyrr.
    pub fn open_licenses() {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()));
        let found = exe_dir
            .iter()
            .flat_map(|d| [d.join("../share/doc/kyrr/THIRD_PARTY.md"), d.join("THIRD_PARTY.md")])
            .find(|p| p.is_file());
        match found {
            Some(path) => open_url(&path.to_string_lossy()),
            None => open_url("https://github.com/heast13/kyrr/blob/main/THIRD_PARTY.md"),
        }
    }

    pub fn open_url(url: &str) {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }

    fn autostart_file() -> PathBuf {
        let config = crate::config::config_dir();
        config
            .parent()
            .map(|d| d.join("autostart"))
            .unwrap_or_default()
            .join("io.github.heast13.kyrr.desktop")
    }

    pub fn enable_autostart() {
        if !autostart_enabled() {
            set_autostart(true);
        }
    }

    /// A path for inside the quotes of a desktop entry's Exec: the quoting
    /// escapes `"`, `` ` ``, `$` and `\` with a backslash, the value's own
    /// escaping doubles that backslash, and `%` is doubled for field codes.
    fn exec_quoted(path: &str) -> String {
        let mut out = String::with_capacity(path.len());
        for c in path.chars() {
            match c {
                '"' | '`' | '$' => {
                    out.push_str("\\\\");
                    out.push(c);
                }
                '\\' => out.push_str("\\\\\\\\"),
                '%' => out.push_str("%%"),
                c => out.push(c),
            }
        }
        out
    }

    fn flatpak() -> bool {
        std::env::var_os("FLATPAK_ID").is_some()
    }

    pub fn autostart_enabled() -> bool {
        if flatpak() {
            return state().saved.autostart;
        }
        std::fs::read_to_string(autostart_file()).is_ok_and(|entry| !autostart_switched_off(&entry))
    }

    /// The desktop's own settings (GNOME Tweaks, KDE's) switch a start at
    /// sign-in off by marking the file instead of deleting it.
    fn autostart_switched_off(entry: &str) -> bool {
        entry
            .lines()
            .any(|l| matches!(l.trim(), "Hidden=true" | "X-GNOME-Autostart-enabled=false"))
    }

    #[cfg(test)]
    #[test]
    fn autostart_switched_off_by_the_desktop() {
        let ours = "[Desktop Entry]\nExec=kyrr --background\nX-GNOME-Autostart-enabled=true\n";
        assert!(!autostart_switched_off(ours));
        assert!(autostart_switched_off(&ours.replace("enabled=true", "enabled=false")));
        assert!(autostart_switched_off(&format!("{ours}Hidden=true\n")));
    }

    pub fn set_autostart(on: bool) {
        if flatpak() {
            // Taken as asked; the desktop may ask the user first, and its
            // answer lands in the settings the next time they are shown.
            {
                let mut s = state();
                s.saved.autostart = on;
                s.saved.save();
            }
            std::thread::spawn(move || {
                if let Ok(enabled) = kyrr_linux::background::request_autostart(on) {
                    let mut s = state();
                    s.saved.autostart = enabled;
                    s.saved.save();
                }
            });
            return;
        }
        let file = autostart_file();
        if !on {
            let _ = std::fs::remove_file(file);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // An AppImage runs from a temporary mount; start the AppImage itself.
        // TryExec: a deleted AppImage is skipped at sign-in instead of failing.
        let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or(exe);
        let path = exe.display().to_string();
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=kyrr\nTryExec={}\nExec=\"{}\" --background\nIcon=io.github.heast13.kyrr\nX-GNOME-Autostart-enabled=true\n",
            path.replace('\\', "\\\\"),
            exec_quoted(&path)
        );
        write_if_changed(&file, Some(entry.into_bytes()));
    }

    /// The screen the window is on (panels included; a Wayland app cannot
    /// learn where they are), once the window exists.
    pub fn work_area(window: &slint::Window) -> Option<(i32, i32, i32, i32)> {
        use slint::winit_030::WinitWindowAccessor;
        window
            .with_winit_window(|w| {
                let monitor = w.current_monitor()?;
                let (pos, size) = (monitor.position(), monitor.size());
                Some((pos.x, pos.y, pos.x + size.width as i32, pos.y + size.height as i32))
            })
            .flatten()
    }

    pub fn bring_to_front(_: &slint::Window) {}

    pub fn run_installer(_: &std::path::Path) -> Result<(), SetupError> {
        Err(SetupError::Failed("updates come from your package manager".into()))
    }

    /// The filter's levels and before/after take, straight from the engine.
    pub struct Monitor;

    impl Monitor {
        pub fn open() -> Option<Monitor> {
            matches!(state().filter, Filter::On(_)).then_some(Monitor)
        }

        pub fn levels(&self) -> Levels {
            match &state().filter {
                Filter::On(engine) => engine.levels(),
                _ => Levels::default(),
            }
        }

        pub fn request_probe(&self) -> u32 {
            match &state().filter {
                Filter::On(engine) => engine.request_probe(),
                _ => 0,
            }
        }

        pub fn probe(&self, id: u32) -> Option<Take> {
            match &state().filter {
                Filter::On(engine) => engine.probe(id),
                _ => None,
            }
        }
    }
}

pub use imp::*;
