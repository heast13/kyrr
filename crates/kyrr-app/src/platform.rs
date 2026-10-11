//! Everything the app needs from the operating system, behind one interface.
//! Windows has the real thing; elsewhere there are stand-ins so the window
//! can be developed and tried on Linux.

use kyrr_common::FilterSettings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    pub guid: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum FilterStatus {
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
}

/// The same take before and after kyrr, mono.
#[derive(Debug)]
pub struct Take {
    pub rate: u32,
    pub before: Vec<f32>,
    pub after: Vec<f32>,
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

    /// Top and bottom of the free screen at a point, in physical pixels.
    pub fn work_area(x: i32, y: i32) -> Option<(i32, i32)> {
        kyrr_windows::screen::work_area(x, y)
    }

    pub use kyrr_windows::hotkey::KeyEvent;

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
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait};

    pub fn microphones() -> Vec<Microphone> {
        let host = cpal::default_host();
        host.input_devices()
            .map(|devs| {
                devs.filter_map(|d| d.description().ok())
                    .map(|d| Microphone {
                        guid: d.name().to_string(),
                        name: d.name().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn filter_status() -> FilterStatus {
        FilterStatus::NotInstalled
    }

    pub fn filter_settings() -> FilterSettings {
        FilterSettings::default()
    }

    pub fn set_filter_values(_: &[(&str, u32)]) -> Result<(), String> {
        Ok(())
    }

    pub fn attach(_: &str) -> Result<(), SetupError> {
        Err(SetupError::Failed(
            "the audio filter is only available on Windows".into(),
        ))
    }

    pub fn paste_text(text: &str) -> bool {
        println!("{text}");
        true
    }

    pub fn copy_to_clipboard(_: &str) -> bool {
        false
    }

    pub fn set_hotkey_key(_: u32) {}

    pub fn open_licenses() {}

    pub fn open_url(url: &str) {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }

    #[derive(Clone, Copy, Debug)]
    #[allow(dead_code)]
    pub enum KeyEvent {
        Pressed,
        Released,
    }

    pub fn start_hotkey() -> Option<std::sync::mpsc::Receiver<KeyEvent>> {
        None
    }

    pub fn set_hotkey_active(_: bool) {}

    pub fn enable_autostart() {}

    pub fn autostart_enabled() -> bool {
        false
    }

    pub fn set_autostart(_: bool) {}

    pub fn work_area(_: i32, _: i32) -> Option<(i32, i32)> {
        None
    }

    pub struct Monitor;

    #[allow(dead_code)]
    impl Monitor {
        pub fn open() -> Option<Monitor> {
            None
        }

        pub fn levels(&self) -> Levels {
            Levels::default()
        }

        pub fn request_probe(&self) -> u32 {
            0
        }

        pub fn probe(&self, _: u32) -> Option<Take> {
            None
        }
    }
}

pub use imp::*;
