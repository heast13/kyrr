//! Live filter settings. A watcher thread waits for changes to the app's
//! settings key and copies them into atomics the audio thread reads.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use kyrr_common::{filter, FilterSettings, REG_FILTER};
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_QUERY_VALUE,
    REG_NOTIFY_CHANGE_LAST_SET,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};

use crate::registry::{read_dword, wide};

pub struct Live {
    enabled: AtomicBool,
    limit_db: AtomicU32,
    light_model: AtomicBool,
}

impl Live {
    pub fn get(&self) -> FilterSettings {
        FilterSettings {
            enabled: self.enabled.load(Ordering::Relaxed),
            limit_db: self.limit_db.load(Ordering::Relaxed),
            light_model: self.light_model.load(Ordering::Relaxed),
        }
    }

    fn set(&self, s: FilterSettings) {
        self.enabled.store(s.enabled, Ordering::Relaxed);
        self.limit_db.store(s.limit_db, Ordering::Relaxed);
        self.light_model.store(s.light_model, Ordering::Relaxed);
    }
}

/// The process-wide settings; the first call reads them and starts the watcher.
pub fn live() -> &'static Live {
    static LIVE: OnceLock<Live> = OnceLock::new();
    LIVE.get_or_init(|| {
        let defaults = FilterSettings::default();
        let live = Live {
            enabled: AtomicBool::new(defaults.enabled),
            limit_db: AtomicU32::new(defaults.limit_db),
            light_model: AtomicBool::new(defaults.light_model),
        };
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
    let d = FilterSettings::default();
    FilterSettings {
        enabled: read_dword(key, "", filter::ENABLED).map_or(d.enabled, |v| v != 0),
        limit_db: read_dword(key, "", filter::LIMIT_DB).map_or(d.limit_db, |v| v.min(100)),
        light_model: read_dword(key, "", filter::MODEL).map_or(d.light_model, |v| v == 1),
    }
}

/// Waits for changes for the life of the process. Without the key (not
/// installed yet, or removed) it retries every few seconds.
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
            live().set(read(key));
            unsafe { WaitForSingleObject(event, INFINITE) };
        }
        unsafe {
            let _ = RegCloseKey(key);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
