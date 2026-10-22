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
    /// The microphone sounds far from the voice (room echo on every word).
    pub far: bool,
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
    /// Another bind of the user's already has the key. Where it is goes to
    /// the log, the window only says that the key is taken.
    Taken { key: String },
    /// kyrr bound the key, but the desktop runs it on presses only (GNOME
    /// before 48, niri): each press starts or ends dictation.
    PressOnly { desktop: &'static str, key: String },
    /// Another of GNOME's custom shortcuts, by this name, already has the key.
    GnomeTaken { key: String, name: String },
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

/// How installing PipeWire went (Linux).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub enum PipewireInstall {
    Running,
    /// Installed; it starts with the next sign-in.
    AfterSignIn,
    /// The password prompt was cancelled.
    Declined,
    Failed,
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

    pub fn typing_blocked_by_sandbox() -> bool {
        false
    }

    pub fn set_hotkey_key(vk: u32) {
        hotkey::set_key(vk);
    }

    /// The voice memo key, 0 for none.
    pub fn set_memo_key(vk: u32, _description: &str) {
        hotkey::set_memo_key(vk);
    }

    /// Windows does not run the key hook for keys that reach kyrr's own
    /// window, so the window passes the dictation and memo keys on to it.
    pub fn keys_in_window(window: &slint::Window) {
        use slint::winit_030::winit::event::{ElementState, WindowEvent};
        use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};
        use slint::winit_030::{EventResult, WinitWindowAccessor};
        window.on_winit_window_event(|_, event| {
            // Not the synthetic presses and releases winit makes up when the
            // window gains or loses focus with a key down.
            if let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = event
            {
                let vk = match event.physical_key {
                    PhysicalKey::Code(KeyCode::F8) => 0x77,
                    PhysicalKey::Code(KeyCode::F9) => 0x78,
                    PhysicalKey::Code(KeyCode::Pause) => 0x13,
                    PhysicalKey::Code(KeyCode::ControlRight) => 0xA3,
                    _ => return EventResult::Propagate,
                };
                if hotkey::window_key(vk, event.state == ElementState::Pressed) {
                    return EventResult::PreventDefault;
                }
            }
            EventResult::Propagate
        });
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

    /// One line about this computer for a problem report.
    pub fn system_info() -> String {
        "Windows".into()
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

    pub fn minimize(window: &slint::Window) {
        window.set_minimized(true);
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

    /// The audio service keeps the filter going by itself.
    pub fn tend_filter() {}

    /// A window for the dictation wave, on the screen the user types on.
    pub fn open_wave() -> Option<Box<dyn kyrr_common::wave::Surface>> {
        kyrr_windows::overlay::Overlay::open().map(|o| Box::new(o) as Box<dyn kyrr_common::wave::Surface>)
    }

    pub fn has_tray() -> bool {
        true
    }

    pub fn can_minimize() -> bool {
        true
    }

    pub fn pipewire_missing() -> bool {
        false
    }

    pub fn pipewire_plan() -> Option<(bool, String)> {
        None
    }

    pub fn install_pipewire() -> PipewireInstall {
        PipewireInstall::Failed
    }

    pub fn restart_after_quit() {}

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
                far: l.far,
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

    use kyrr_linux::binds::{self, Compositor};
    use kyrr_linux::gnome;
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
        /// The system's default microphone from before "kyrr" (PipeWire's
        /// metadata JSON). After a crash the system still names "kyrr" as
        /// its default; this is what quitting gives it back to then.
        system_default: Option<String>,
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
            if let Ok(json) = serde_json::to_vec_pretty(self) {
                Saved::write(&json);
            }
        }

        /// One writer at a time: they share the file next to it.
        fn write(json: &[u8]) {
            static WRITING: Mutex<()> = Mutex::new(());
            let _one = WRITING.lock().unwrap_or_else(|p| p.into_inner());
            let path = Saved::file();
            // Only the user may read it: it holds the keyboard portal's token.
            if let Err(e) = crate::config::write_atomic(&path, json) {
                eprintln!("kyrr: could not save {}: {e}", path.display());
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
        let (settings, remembered) = {
            let mut s = state();
            if matches!(s.filter, Filter::Starting | Filter::On(_)) {
                return;
            }
            s.filter = Filter::Starting;
            (s.saved.settings(), s.saved.system_default.clone())
        };
        let filter = match Engine::start(Some(microphone), remembered, &settings) {
            Ok(engine) => Filter::On(engine),
            Err(e) => {
                eprintln!("kyrr: the noise filter did not start: {e}");
                Filter::Failed(e)
            }
        };
        state().filter = filter;
    }

    pub fn microphones() -> Vec<Microphone> {
        let s = state();
        match &s.filter {
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
            // without starting anything, and without holding everyone else
            // up meanwhile.
            _ => {
                drop(s);
                list_microphones()
            }
        }
    }

    fn list_microphones() -> Vec<Microphone> {
        kyrr_linux::mic::list()
            .unwrap_or_default()
            .into_iter()
            .map(|m| Microphone {
                id: m.name.clone(),
                guid: m.name,
                name: m.description,
                is_default: false,
            })
            .collect()
    }

    /// Whether the microphone (node name) is muted in the sound settings,
    /// asked of WirePlumber's `wpctl`; `None` where that is out of reach
    /// (the Flatpak sandbox) or the filter does not run.
    pub fn microphone_muted(name: &str) -> Option<bool> {
        let id = mic_node(name)?;
        let mut wpctl = std::process::Command::new("wpctl");
        wpctl.args(["get-volume", &id.to_string()]);
        let out = output_within(&mut wpctl, std::time::Duration::from_secs(1))?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).contains("[MUTED]"))
    }

    /// Runs `command` and returns what it printed; `None` if it did not
    /// start or took longer than `limit` (the window asks every few seconds
    /// and must not freeze on a hung sound server).
    fn output_within(command: &mut std::process::Command, limit: std::time::Duration) -> Option<std::process::Output> {
        use std::process::Stdio;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let until = std::time::Instant::now() + limit;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return child.wait_with_output().ok(),
                Ok(None) if std::time::Instant::now() < until => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
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
            // Nothing chosen yet: the setup asks for a microphone, once
            // there is PipeWire to take it from.
            Filter::Off if s.saved.microphone.is_none() && !kyrr_linux::pipewire_setup::running() => {
                FilterStatus::Unsupported
            }
            Filter::Off if s.saved.microphone.is_none() => FilterStatus::NotAttached,
            Filter::Off | Filter::Starting => FilterStatus::Waiting(wanted),
            // Tried again while PipeWire is there (see `tend_filter`).
            Filter::Failed(_) if kyrr_linux::pipewire_setup::running() && s.saved.microphone.is_some() => {
                FilterStatus::Waiting(wanted)
            }
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
        let settings = s.saved.settings();
        if let Filter::On(engine) = &s.filter {
            engine.set_settings(&settings);
        }
        // After the sound changed, and without the lock: saving waits for
        // the disk.
        let json = serde_json::to_vec_pretty(&s.saved);
        drop(s);
        if let Ok(json) = json {
            Saved::write(&json);
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
        let filter = std::mem::replace(&mut state().filter, Filter::Off);
        // Without the lock: this waits for PipeWire a moment.
        drop(filter);
    }

    /// Called every few seconds: keeps the system's default microphone from
    /// before "kyrr" in the saved settings, and starts the filter again
    /// when it could not start (kyrr came up at login before PipeWire did)
    /// or PipeWire went away under it (restarted, updated), as soon as
    /// PipeWire is back. Waits longer after each try that fails.
    pub fn tend_filter() {
        use std::time::{Duration, Instant};
        const FIRST: Duration = Duration::from_secs(2);
        const MOST: Duration = Duration::from_secs(60);
        static NEXT: Mutex<Option<(Instant, Duration)>> = Mutex::new(None);
        let mut next = NEXT.lock().unwrap_or_else(|p| p.into_inner());
        let (microphone, old) = {
            let mut s = state();
            let dead = match &s.filter {
                Filter::Failed(_) => true,
                Filter::On(engine) => engine.gone(),
                Filter::Off | Filter::Starting => false,
            };
            if !dead {
                if let Filter::On(engine) = &s.filter {
                    *next = None;
                    let previous = engine.previous_default();
                    if previous.is_some() && previous != s.saved.system_default {
                        s.saved.system_default = previous;
                        s.saved.save();
                    }
                }
                return;
            }
            let Some(microphone) = s.saved.microphone.clone() else {
                return;
            };
            let now = Instant::now();
            if !kyrr_linux::pipewire_setup::running() || next.is_some_and(|(at, _)| now < at) {
                return;
            }
            let wait = next.map_or(FIRST, |(_, wait)| (wait * 2).min(MOST));
            *next = Some((now + wait, wait));
            (microphone, std::mem::replace(&mut s.filter, Filter::Off))
        };
        eprintln!("kyrr: starting the noise filter again");
        std::thread::spawn(move || {
            drop(old);
            start_filter(microphone);
        });
    }

    /// A surface for the dictation wave, on the screen the user types on.
    pub fn open_wave() -> Option<Box<dyn kyrr_common::wave::Surface>> {
        kyrr_linux::overlay::open()
    }

    /// Whether the desktop shows kyrr's tray icon: a tray host is on the
    /// bus. In a Flatpak the manifest lets kyrr talk to the watcher and own
    /// its icon's name, so it is asked there like anywhere else; without
    /// those rights the sandbox hides the watcher and this stays false.
    pub fn has_tray() -> bool {
        kyrr_linux::tray::available()
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

    /// Typing failed in a Flatpak on a desktop without the RemoteDesktop
    /// portal and without kyrr's link to the compositor: the sandbox is the
    /// reason, which the message can name. On GNOME and KDE a "no" in the
    /// portal's dialog is the likely reason, and the AppImage would ask the
    /// same.
    pub fn typing_blocked_by_sandbox() -> bool {
        flatpak() && desktop() == Desktop::Other
    }

    pub fn set_hotkey_key(vk: u32) {
        kyrr_linux::hotkey::set_key(vk);
    }

    /// The voice memo key, 0 for none; `description` names it in the
    /// desktop's settings.
    pub fn set_memo_key(vk: u32, description: &str) {
        kyrr_linux::hotkey::set_memo_key(vk, description);
    }

    /// The desktop's shortcuts work in kyrr's window too.
    pub fn keys_in_window(_: &slint::Window) {}

    /// `description` names the shortcut in the desktop's dialog and settings.
    /// Before the setup is done, GNOME and KDE are asked for the key only
    /// when the setup gets to it, not out of the blue at the start.
    pub fn start_hotkey(description: &str, setup_done: bool) -> Option<std::sync::mpsc::Receiver<KeyEvent>> {
        integrate_appimage();
        start_in_compositor(autostart_enabled());
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

    /// True when the file is new or changed.
    fn write_if_changed(path: &std::path::Path, contents: Option<Vec<u8>>) -> bool {
        let Some(contents) = contents else {
            return false;
        };
        if std::fs::read(path).is_ok_and(|old| old == contents) {
            return false;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(path, contents) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("kyrr: writing {}: {e}", path.display());
                false
            }
        }
    }

    pub fn set_hotkey_active(active: bool) {
        kyrr_linux::hotkey::set_active(active);
    }

    pub fn key_note() -> KeyNote {
        use kyrr_linux::hotkey::{self, Backend};
        let compositor = Compositor::detect();
        let hyprland = compositor == Some(Compositor::Hyprland);
        let backend = hotkey::backend();
        if !hyprland && hotkey::unbound() {
            return KeyNote::Unbound;
        }
        if backend == Backend::X11 {
            return KeyNote::None;
        }
        // Hyprland's portal only forwards a bind of the config; the other
        // tiling ones have no portal at all.
        if let Some(compositor) = compositor.filter(|_| hyprland || backend == Backend::None) {
            return compositor_note(compositor, backend == Backend::Portal);
        }
        match backend {
            Backend::Portal => KeyNote::Desktop(desktop()),
            _ if desktop() == Desktop::Gnome && !flatpak() => gnome_note(),
            _ => KeyNote::Command(dictate_command("--dictate")),
        }
    }

    fn compositor_note(compositor: Compositor, portal: bool) -> KeyNote {
        let keys = binds::Keys {
            key: kyrr_linux::hotkey::key_name().to_string(),
            memo: kyrr_linux::hotkey::memo_key_name().map(str::to_string),
            portal,
            program: program(),
            link: flatpak(),
        };
        let base = binds::config_home();
        // Dictation is off: nothing to say about its key.
        let Some(outcome) = compositor_outcome(compositor, &base, &keys).filter(|_| dictation_wanted()) else {
            return KeyNote::None;
        };
        match outcome {
            binds::Outcome::Bound { .. } if compositor.holds() => KeyNote::None,
            binds::Outcome::Bound { .. } => KeyNote::PressOnly {
                desktop: compositor.name(),
                key: keys.key,
            },
            binds::Outcome::Taken { file, line, bind } => {
                eprintln!(
                    "kyrr: {} is taken in {} line {line}: {bind}",
                    keys.key,
                    binds::display(&file)
                );
                KeyNote::Taken { key: keys.key }
            }
            binds::Outcome::Failed => {
                // The window only asks for a key on the command; the config
                // lines go to the log for those who want them.
                let target = binds::target_or_default(compositor, &base);
                eprintln!(
                    "kyrr: could not write {}; lines for it:\n{}",
                    binds::display(&target.path),
                    binds::lines(target.syntax, &keys)
                );
                KeyNote::Command(dictate_command("--dictate"))
            }
        }
    }

    /// GNOME before 48 on Wayland: kyrr adds a custom shortcut of GNOME's.
    fn gnome_note() -> KeyNote {
        let key = kyrr_linux::hotkey::key_name().to_string();
        let command = binds::command(&program(), "--dictate");
        let memo = kyrr_linux::hotkey::memo_key_name().map(str::to_string);
        // The memo key has no note of its own on the main page: the log.
        match memo_outcome(memo.clone(), binds::command(&program(), "--memo")) {
            Some(gnome::Outcome::Taken { name }) => {
                eprintln!("kyrr: the voice memo key {memo:?} is taken by GNOME's shortcut \"{name}\"")
            }
            Some(gnome::Outcome::Failed) => eprintln!("kyrr: could not add GNOME's shortcut for the voice memo key"),
            _ => {}
        }
        let Some(outcome) = gnome_outcome(&key, &command) else {
            return KeyNote::None;
        };
        match outcome {
            gnome::Outcome::Bound { .. } => KeyNote::PressOnly { desktop: "GNOME", key },
            gnome::Outcome::Taken { name } => KeyNote::GnomeTaken { key, name },
            gnome::Outcome::Failed => KeyNote::Command(dictate_command("--dictate")),
        }
    }

    /// Whether dictation is on, so kyrr may set its key up in the desktop.
    pub fn set_key_wanted(wanted: bool) {
        KEY_WANTED.store(wanted, std::sync::atomic::Ordering::Relaxed);
    }

    static KEY_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    type Memo<K, O> = std::sync::Mutex<Option<(K, O)>>;
    static COMPOSITOR: Memo<(Compositor, binds::Keys), binds::Outcome> = std::sync::Mutex::new(None);
    static GNOME: Memo<(String, String), gnome::Outcome> = std::sync::Mutex::new(None);
    static GNOME_MEMO: Memo<(Option<String>, String), gnome::Outcome> = std::sync::Mutex::new(None);

    /// Runs `bind` while `wanted`, once per what it binds: the answer
    /// stands until that changes, so a key freed elsewhere is noticed after
    /// the next start. No longer wanted, a change still binds once more
    /// (the memo key switched off comes out again), but nothing is said.
    fn memo<K: PartialEq, O: Clone>(memo: &Memo<K, O>, key: K, wanted: bool, bind: impl FnOnce() -> O) -> Option<O> {
        let mut memo = memo.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !wanted && memo.is_none() {
            return None;
        }
        if let Some((seen, outcome)) = memo.as_ref() {
            if *seen == key {
                return wanted.then(|| outcome.clone());
            }
        }
        let outcome = bind();
        *memo = Some((key, outcome.clone()));
        wanted.then_some(outcome)
    }

    fn dictation_wanted() -> bool {
        KEY_WANTED.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Puts the key into the compositor's config and has it read the file.
    fn compositor_outcome(
        compositor: Compositor,
        base: &std::path::Path,
        keys: &binds::Keys,
    ) -> Option<binds::Outcome> {
        memo(
            &COMPOSITOR,
            (compositor, keys.clone()),
            dictation_wanted() || keys.memo.is_some(),
            || {
                let before = binds::bound_keys(compositor, base);
                let outcome = binds::ensure(compositor, base, keys);
                if matches!(outcome, binds::Outcome::Bound { written: true, .. }) && !flatpak() {
                    let lines = binds::target(compositor, base).map(|t| binds::lines(t.syntax, keys));
                    let now = binds::bound_keys(compositor, base);
                    std::thread::spawn(move || reload(compositor, before, &now, lines));
                }
                outcome
            },
        )
    }

    /// Makes the compositor take the new key now. Hyprland and niri see the
    /// change by themselves (this only makes Hyprland quicker), sway reads
    /// the file on `reload`, river only at its start: there the lines run.
    fn reload(compositor: Compositor, before: Vec<String>, now: &[String], lines: Option<String>) {
        let run = |program: &str, args: &[&str]| {
            let _ = std::process::Command::new(program).args(args).status();
        };
        match compositor {
            Compositor::Hyprland => run("hyprctl", &["reload"]),
            Compositor::Sway => run("swaymsg", &["reload"]),
            Compositor::Niri => {}
            Compositor::River => {
                for old in before.iter().filter(|old| !now.contains(old)) {
                    run("riverctl", &["unmap", "normal", "None", old]);
                    run("riverctl", &["unmap", "-release", "normal", "None", old]);
                }
                if let Some(lines) = lines {
                    run("sh", &["-c", &lines]);
                }
            }
        }
    }

    /// The voice memo key as a second custom shortcut of GNOME's, or none.
    fn memo_outcome(key: Option<String>, command: String) -> Option<gnome::Outcome> {
        memo(&GNOME_MEMO, (key.clone(), command.clone()), true, || {
            gnome::ensure_memo(key.as_deref(), &command)
        })
    }

    fn gnome_outcome(key: &str, command: &str) -> Option<gnome::Outcome> {
        memo(
            &GNOME,
            (key.to_string(), command.to_string()),
            dictation_wanted(),
            || gnome::ensure(key, command),
        )
    }

    /// How this kyrr is started: through Flatpak, the AppImage's path, or the
    /// program's own; the flag goes after it.
    fn program() -> Vec<String> {
        if let Ok(id) = std::env::var("FLATPAK_ID") {
            return vec!["flatpak".into(), "run".into(), id];
        }
        let program = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .filter(|p| !p.starts_with("/usr/bin"))
            .map_or_else(|| "kyrr".to_string(), |p| p.display().to_string());
        vec![program]
    }

    fn dictate_command(flag: &str) -> String {
        binds::command(&program(), flag)
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

    /// One line about this computer for a problem report: the distribution,
    /// the desktop and how kyrr is installed.
    pub fn system_info() -> String {
        let distro = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_string())
            })
            .unwrap_or_else(|| "Linux".into());
        let mut parts = vec![distro];
        if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
            if !desktop.is_empty() {
                parts.push(desktop);
            }
        }
        if flatpak() {
            parts.push("Flatpak".into());
        } else if std::env::var_os("APPIMAGE").is_some() {
            parts.push("AppImage".into());
        }
        parts.join(", ")
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
        // Tiling compositors without XDG autostart start what their config
        // says; a Flatpak may write there too, its desktop often has no
        // Background portal (Hyprland). A start from both is harmless.
        start_in_compositor(on);
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
            if std::fs::remove_file(file).is_ok() {
                autostart_changed();
            }
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
        if write_if_changed(&file, Some(entry.into_bytes())) {
            autostart_changed();
        }
    }

    /// systemd turns the autostart files into services when it starts, and
    /// keeps that copy across a sign-out: without a reload the next sign-in
    /// starts the old file (an AppImage since moved). Where systemd runs the
    /// session it reloads now; elsewhere the file is read at sign-in anyway.
    fn autostart_changed() {
        let user_systemd =
            std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| std::path::Path::new(&d).join("systemd").is_dir());
        if !user_systemd {
            return;
        }
        std::thread::spawn(|| {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }

    /// Tiling compositors started without uwsm or a systemd session leave
    /// the XDG autostart folder unread: there the start at sign-in goes into
    /// their config too, in a block of kyrr's. Where the folder is read the
    /// block goes, so kyrr does not start twice.
    fn start_in_compositor(on: bool) {
        let Some(compositor) = Compositor::detect() else {
            return;
        };
        let wanted = on && !xdg_autostart_read();
        let program = program();
        binds::set_start(compositor, &binds::config_home(), wanted.then_some(program.as_slice()));
    }

    /// Whether this session starts the XDG autostart files: systemd's target
    /// for them is active, or still activating while it starts kyrr (uwsm,
    /// niri-session, GNOME, KDE).
    fn xdg_autostart_read() -> bool {
        std::process::Command::new("systemctl")
            .args([
                "--user",
                "show",
                "--property=ActiveState",
                "--value",
                "xdg-desktop-autostart.target",
            ])
            .output()
            .is_ok_and(|o| matches!(String::from_utf8_lossy(&o.stdout).trim(), "active" | "activating"))
    }

    /// Whether PipeWire is missing, so the window offers to set it up.
    pub fn pipewire_missing() -> bool {
        !kyrr_linux::pipewire_setup::running()
    }

    /// How PipeWire gets onto this system: whether kyrr can do it after the
    /// system's password prompt, and the command for a terminal. `None` where
    /// kyrr does not know (a Flatpak, a read-only system).
    pub fn pipewire_plan() -> Option<(bool, String)> {
        use kyrr_linux::pipewire_setup as setup;
        setup::plan().map(|p| (setup::can_ask(), p.shown))
    }

    /// Installs PipeWire; blocks for the download.
    pub fn install_pipewire() -> PipewireInstall {
        use kyrr_linux::pipewire_setup::{self as setup, Installed};
        let Some(plan) = setup::plan() else {
            return PipewireInstall::Failed;
        };
        match setup::install(&plan) {
            Installed::Running => PipewireInstall::Running,
            Installed::AfterSignIn => PipewireInstall::AfterSignIn,
            Installed::Declined => PipewireInstall::Declined,
            Installed::Failed => PipewireInstall::Failed,
        }
    }

    /// Starts kyrr again once this one has quit.
    pub fn restart_after_quit() {
        kyrr_linux::pipewire_setup::restart_after_exit(&[]);
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

    /// Nothing to raise on Linux, but Hyprland tiles a window it has no rule
    /// for, and a window coming back from the tray lands in the tiling
    /// layout (the fixed size that keeps it floating does not always reach
    /// it). Setting "floating" is safe to repeat; it only does something to a
    /// tiled window. The window may not be mapped yet, hence the tries.
    pub fn bring_to_front(_: &slint::Window) {
        if Compositor::detect() != Some(Compositor::Hyprland) {
            return;
        }
        std::thread::spawn(|| {
            for pause in [50, 150, 300] {
                std::thread::sleep(std::time::Duration::from_millis(pause));
                let _ = std::process::Command::new("hyprctl")
                    .args(["dispatch", "setfloating", "class:^(io\\.github\\.heast13\\.kyrr)$"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        });
    }

    /// Minimizes the window. A Wayland compositor never tells a window that
    /// it came back, so Slint would take it as minimized for good: the next
    /// close or "Off to the corner" would do nothing, and the window would
    /// no longer fit its content. On Wayland the flag goes back right away;
    /// unminimizing asks nothing of the compositor there.
    pub fn minimize(window: &slint::Window) {
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use slint::winit_030::WinitWindowAccessor;
        window.set_minimized(true);
        let wayland = window
            .with_winit_window(|w| {
                w.window_handle()
                    .is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
            })
            .unwrap_or(false);
        if wayland {
            window.set_minimized(false);
        }
    }

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
