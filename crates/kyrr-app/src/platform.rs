//! Everything the app needs from the operating system, behind one interface:
//! Windows (the filter is an APO in the audio service) and Linux (the filter
//! runs in kyrr on a virtual PipeWire microphone).

use kyrr_common::FilterSettings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    pub guid: String,
    pub name: String,
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
    Desktop,
    /// Hyprland binds the key: this line belongs in hyprland.conf.
    Hyprland(String),
    /// Nothing catches keys: bind this command to a key.
    Command(String),
}

#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum SetupError {
    Declined,
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
                guid: m.guid,
                name: m.name,
            })
            .collect()
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
    pub fn start_hotkey() -> Option<std::sync::mpsc::Receiver<hotkey::KeyEvent>> {
        hotkey::start().ok()
    }

    pub fn set_hotkey_active(active: bool) {
        hotkey::set_active(active);
    }

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
    pub fn work_area(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
        kyrr_windows::screen::work_area(x, y)
    }

    pub use kyrr_windows::hotkey::KeyEvent;

    /// The filter lives in the audio service; nothing to stop.
    pub fn shutdown() {}

    pub fn has_tray() -> bool {
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
    }

    impl Saved {
        fn file() -> PathBuf {
            crate::config::config_dir().join("filter.json")
        }

        fn load() -> Saved {
            std::fs::read(Saved::file())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        }

        fn save(&self) {
            let path = Saved::file();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_vec_pretty(self) {
                let _ = std::fs::write(path, json);
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
            Err(e) => Filter::Failed(e),
        };
        state().filter = filter;
    }

    pub fn microphones() -> Vec<Microphone> {
        match &state().filter {
            Filter::On(engine) => engine
                .microphones()
                .into_iter()
                .map(|m| Microphone {
                    guid: m.name,
                    name: m.description,
                })
                .collect(),
            // Before the first setup: the microphones PipeWire offers, read
            // without starting anything.
            _ => kyrr_linux::mic::list()
                .unwrap_or_default()
                .into_iter()
                .map(|m| Microphone {
                    guid: m.name,
                    name: m.description,
                })
                .collect(),
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

    /// Whether the desktop shows kyrr's tray icon.
    pub fn has_tray() -> bool {
        kyrr_linux::tray::available()
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

    pub fn start_hotkey() -> Option<std::sync::mpsc::Receiver<KeyEvent>> {
        kyrr_linux::hotkey::start().ok()
    }

    pub fn set_hotkey_active(active: bool) {
        kyrr_linux::hotkey::set_active(active);
    }

    pub fn key_note() -> KeyNote {
        use kyrr_linux::hotkey::{self, Backend};
        let hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
            || std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_lowercase().contains("hyprland"));
        match hotkey::backend() {
            Backend::X11 => KeyNote::None,
            Backend::Portal if hyprland => KeyNote::Hyprland(format!(
                "bind = , {}, global, io.github.heast13.kyrr:dictate",
                hotkey::key_name()
            )),
            Backend::Portal => KeyNote::Desktop,
            Backend::None => KeyNote::Command(dictate_command()),
        }
    }

    /// How this kyrr is started with `--dictate`: through Flatpak, the
    /// AppImage's path, or the program's own.
    fn dictate_command() -> String {
        if let Ok(id) = std::env::var("FLATPAK_ID") {
            return format!("flatpak run {id} --dictate");
        }
        let program = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .filter(|p| !p.starts_with("/usr/bin"))
            .map_or_else(|| "kyrr".to_string(), |p| p.display().to_string());
        format!("{program} --dictate")
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

    pub fn autostart_enabled() -> bool {
        autostart_file().is_file()
    }

    pub fn set_autostart(on: bool) {
        let file = autostart_file();
        if !on {
            let _ = std::fs::remove_file(file);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // An AppImage runs from a temporary mount; start the AppImage itself.
        let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or(exe);
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=kyrr\nExec=\"{}\" --background\nIcon=io.github.heast13.kyrr\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, entry);
    }

    pub fn work_area(_: i32, _: i32) -> Option<(i32, i32, i32, i32)> {
        None
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
