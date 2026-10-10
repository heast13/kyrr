//! The app's own settings (the filter's live in the registry, see
//! `kyrr_windows::filter`) and where kyrr keeps its files.

use std::path::PathBuf;

use kyrr_dictate::Language;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "de" or "en": user interface and dictation model.
    pub language: String,
    pub dictation: bool,
    /// First-run setup finished.
    pub setup_done: bool,
    /// The dictation key, see [`DICTATION_KEYS`].
    pub dictation_key: u32,
    /// Tap once to start and once to stop, instead of holding the key.
    pub dictation_toggle: bool,
    /// The large noise model was swapped for the small one once because the
    /// CPU lacks AVX2; never again, so the user's later choice holds.
    pub lean_suggested: bool,
}

/// Keys dictation can use, as Windows virtual-key codes: F8, F9, Pause and
/// right Ctrl. The index is what the settings page shows.
pub const DICTATION_KEYS: [u32; 4] = [0x77, 0x78, 0x13, 0xA3];

impl Default for Config {
    fn default() -> Self {
        Config {
            language: system_language().code().into(),
            dictation: true,
            setup_done: false,
            dictation_key: DICTATION_KEYS[1],
            dictation_toggle: false,
            lean_suggested: false,
        }
    }
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(config_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written next to it and renamed, so a crash never leaves half a file.
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    pub fn language(&self) -> Language {
        Language::from_code(&self.language).unwrap_or(Language::English)
    }
}

/// German for German-speaking systems, English otherwise.
fn system_language() -> Language {
    #[cfg(windows)]
    let locale = {
        use windows::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buf) } as usize;
        String::from_utf16_lossy(&buf[..len.saturating_sub(1)])
    };
    #[cfg(not(windows))]
    let locale = std::env::var("LANG").unwrap_or_default();
    match Language::from_code(&locale) {
        Some(Language::German) => Language::German,
        _ => Language::English,
    }
}

fn base_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(fallback),
    }
}

fn config_file() -> PathBuf {
    #[cfg(windows)]
    let dir = base_dir("APPDATA", "AppData/Roaming");
    #[cfg(not(windows))]
    let dir = base_dir("XDG_CONFIG_HOME", ".config");
    dir.join("kyrr").join("config.json")
}

/// Where the dictation models are downloaded to.
pub fn models_dir() -> PathBuf {
    #[cfg(windows)]
    let dir = base_dir("LOCALAPPDATA", "AppData/Local");
    #[cfg(not(windows))]
    let dir = base_dir("XDG_DATA_HOME", ".local/share");
    dir.join("kyrr").join("models")
}
