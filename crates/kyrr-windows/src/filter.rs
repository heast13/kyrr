//! The app's side of the filter: writing its settings (users may write the
//! key, see `setup.rs`) and reading what the APO reports.

use kyrr_common::{filter, status, FilterSettings, REG_FILTER, REG_STATUS};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use crate::reg::Key;

/// The settings as stored, defaults for missing values.
pub fn settings() -> FilterSettings {
    let d = FilterSettings::default();
    let Some(key) = Key::open(HKEY_LOCAL_MACHINE, REG_FILTER) else {
        return d;
    };
    FilterSettings {
        enabled: key.get_dword(filter::ENABLED).map_or(d.enabled, |v| v != 0),
        limit_db: key.get_dword(filter::LIMIT_DB).map_or(d.limit_db, |v| v.min(100)),
        light_model: key.get_dword(filter::MODEL).map_or(d.light_model, |v| v == 1),
    }
}

/// Stores the settings; the APO picks them up within milliseconds.
pub fn set_settings(s: &FilterSettings) -> Result<(), String> {
    let key = Key::open_writable(HKEY_LOCAL_MACHINE, REG_FILTER).ok_or("filter settings key missing")?;
    key.set_dword(filter::ENABLED, s.enabled as u32)?;
    key.set_dword(filter::LIMIT_DB, s.limit_db.min(100))?;
    key.set_dword(filter::MODEL, s.light_model as u32)
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
