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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: system_language().code().into(),
            dictation: true,
            setup_done: false,
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
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
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
