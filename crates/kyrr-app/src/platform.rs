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
        match filter::status() {
            Some(s) if s.state == "running" => FilterStatus::Running(guid, format!("{}, {}", s.engine, s.format)),
            Some(s) if s.state.starts_with("passthrough") => FilterStatus::Passthrough(guid, s.state),
            _ => FilterStatus::Waiting(guid),
        }
    }

    pub fn filter_settings() -> FilterSettings {
        filter::settings()
    }

    pub fn set_filter_settings(s: &FilterSettings) -> Result<(), String> {
        filter::set_settings(s)
    }

    /// Moves the filter to microphone `guid` (asks Windows for permission).
    pub fn attach(guid: &str) -> Result<(), SetupError> {
        for old in setup::attached().into_iter().filter(|g| g != guid) {
            run(&["--setup", "detach", &old])?;
        }
        run(&["--setup", "attach", guid])
    }

    fn run(args: &[&str]) -> Result<(), SetupError> {
        match elevate::run_self_elevated(args) {
            Ok(0) => Ok(()),
            Ok(code) => Err(SetupError::Failed(format!("setup failed ({code})"))),
            Err(elevate::ElevateError::Declined) => Err(SetupError::Declined),
            Err(elevate::ElevateError::Failed(e)) => Err(SetupError::Failed(e)),
        }
    }

    pub fn type_text(text: &str) -> bool {
        typing::type_text(text)
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

    pub use kyrr_windows::hotkey::KeyEvent;
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

    pub fn set_filter_settings(_: &FilterSettings) -> Result<(), String> {
        Ok(())
    }

    pub fn attach(_: &str) -> Result<(), SetupError> {
        Err(SetupError::Failed(
            "the audio filter is only available on Windows".into(),
        ))
    }

    pub fn type_text(text: &str) -> bool {
        println!("{text}");
        true
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
}

pub use imp::*;
