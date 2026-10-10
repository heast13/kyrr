//! Starting kyrr with Windows, for the current user.

use windows::Win32::System::Registry::HKEY_CURRENT_USER;

use crate::reg::Key;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "kyrr";

/// Registers this executable to start in the background at sign-in.
pub fn enable() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let key = Key::create(HKEY_CURRENT_USER, RUN_KEY, None)?;
    key.set_string(NAME, &format!("\"{}\" --background", exe.display()))
}

pub fn disable() -> Result<(), String> {
    match Key::open_writable(HKEY_CURRENT_USER, RUN_KEY) {
        Some(key) => key.delete_value(NAME),
        None => Ok(()),
    }
}

pub fn is_enabled() -> bool {
    Key::open(HKEY_CURRENT_USER, RUN_KEY).is_some_and(|k| k.get_string(NAME).is_some())
}
