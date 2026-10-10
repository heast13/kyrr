//! The app's side of the filter: writing its settings (users may write the
//! key, see `setup.rs`) and reading what the APO reports.

use kyrr_common::{status, FilterSettings, REG_FILTER, REG_STATUS};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use crate::reg::Key;

/// The settings as stored, defaults for missing values.
pub fn settings() -> FilterSettings {
    let key = Key::open(HKEY_LOCAL_MACHINE, REG_FILTER);
    FilterSettings::from_dwords(|name| key.as_ref()?.get_dword(name))
}

/// Stores the settings; the APO picks them up within milliseconds.
pub fn set_settings(s: &FilterSettings) -> Result<(), String> {
    let key = Key::open_writable(HKEY_LOCAL_MACHINE, REG_FILTER).ok_or("filter settings key missing")?;
    for (name, value) in s.to_dwords() {
        key.set_dword(name, value)?;
    }
    Ok(())
}

/// Stores just these values (the ones that changed): every write wakes the
/// filter's settings watcher.
pub fn set_values(values: &[(&str, u32)]) -> Result<(), String> {
    let key = Key::open_writable(HKEY_LOCAL_MACHINE, REG_FILTER).ok_or("filter settings key missing")?;
    for (name, value) in values {
        key.set_dword(name, *value)?;
    }
    Ok(())
}

/// What the APO last reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub state: String,
    pub engine: String,
    pub format: String,
}

pub fn status() -> Option<Status> {
    let key = Key::open(HKEY_LOCAL_MACHINE, REG_STATUS)?;
    Some(Status {
        state: key.get_string(status::STATE)?,
        engine: key.get_string(status::ENGINE).unwrap_or_default(),
        format: key.get_string(status::FORMAT).unwrap_or_default(),
    })
}
