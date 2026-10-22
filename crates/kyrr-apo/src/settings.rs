//! Live filter settings. A watcher thread waits for changes to the app's
//! settings key and copies them into atomics the audio thread reads.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use kyrr_common::{FilterSettings, REG_FILTER};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_SUCCESS, WAIT_TIMEOUT};
use windows::Win32::System::Registry::{
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_QUERY_VALUE,
    REG_NOTIFY_CHANGE_LAST_SET,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::registry::{read_dword, wide};

/// Number of stored values, see [`FilterSettings::to_dwords`].
const VALUES: usize = FilterSettings::VALUES;

/// The settings as their stored DWORDs, in the order of `to_dwords`.
pub struct Live {
    values: [AtomicU32; VALUES],
}

impl Live {
    fn new(s: FilterSettings) -> Live {
        let live = Live {
            values: std::array::from_fn(|_| AtomicU32::new(0)),
        };
        live.set(s);
        live
    }

    /// Lock-free, for the audio thread. A change arriving meanwhile may be
    /// seen half; the next block sees all of it.
    pub fn get(&self) -> FilterSettings {
        let names = FilterSettings::default().to_dwords();
        FilterSettings::from_dwords(|name| {
            let i = names.iter().position(|(n, _)| *n == name)?;
            Some(self.values[i].load(Ordering::Relaxed))
        })
    }

    fn set(&self, s: FilterSettings) {
        for (slot, (_, value)) in self.values.iter().zip(s.to_dwords()) {
            slot.store(value, Ordering::Relaxed);
        }
    }
}

/// The process-wide settings; the first call reads them and starts the watcher.
pub fn live() -> &'static Live {
    static LIVE: OnceLock<Live> = OnceLock::new();
    LIVE.get_or_init(|| {
        let live = Live::new(FilterSettings::default());
        if let Some(key) = open_key() {
            live.set(read(key));
            unsafe {
                let _ = RegCloseKey(key);
            }
        }
        let _ = std::thread::Builder::new()
            .name("kyrr-apo-settings".into())
            .spawn(watch);
        live
    })
}

fn open_key() -> Option<HKEY> {
    let sub = wide(REG_FILTER);
    let mut key = HKEY::default();
    let err = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(sub.as_ptr()),
            None,
            KEY_QUERY_VALUE | KEY_NOTIFY,
            &mut key,
        )
    };
    (err == ERROR_SUCCESS).then_some(key)
}

fn read(key: HKEY) -> FilterSettings {
    FilterSettings::from_dwords(|name| read_dword(key, "", name))
}

/// Waits for changes for the life of the process. Without the key (not
/// installed yet, or removed) it retries every few seconds. Meanwhile it
/// tells the app, every second, about a filter too slow for the computer.
fn watch() {
    let Ok(event) = (unsafe { CreateEventW(None, false, false, PCWSTR::null()) }) else {
        return;
    };
    loop {
        let Some(key) = open_key() else {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        loop {
            let armed = unsafe { RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(event), true) };
            if armed != ERROR_SUCCESS {
                break;
            }
            let settings = read(key);
            live().set(settings);
            crate::swap::settings_changed(settings.light_model);
            while unsafe { WaitForSingleObject(event, 1000) } == WAIT_TIMEOUT {
                crate::swap::report_too_slow();
            }
        }
        unsafe {
            let _ = RegCloseKey(key);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kyrr_common::{Amount, Look};

    #[test]
    fn live_values_round_trip() {
        let mut s = FilterSettings {
            enabled: false,
            ..FilterSettings::default()
        };
        s.voice.enabled = true;
        s.voice.look = Look::Custom;
        s.voice.air = -35;
        s.voice.compressor = Amount::Strong;
        assert_eq!(Live::new(s).get(), s);
        assert_eq!(s.to_dwords().len(), VALUES);
    }
}
