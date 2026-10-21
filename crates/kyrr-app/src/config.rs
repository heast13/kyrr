//! The app's own settings (the filter's live in the registry, see
//! `kyrr_windows::filter`) and where kyrr keeps its files.

use std::path::{Path, PathBuf};

use kyrr_dictate::Language;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "de" or "en": user interface and dictation model. "multi": dictation
    /// with the model for many European languages, the interface in the
    /// system's language.
    pub language: String,
    pub dictation: bool,
    /// First-run setup finished.
    pub setup_done: bool,
    /// The dictation key, see [`DICTATION_KEYS`].
    pub dictation_key: u32,
    /// Tap once to start and once to stop, instead of holding the key.
    pub dictation_toggle: bool,
    /// The wave shows the words heard so far while the key is held.
    pub dictation_live: bool,
    /// The lean setting was picked once because the CPU lacks AVX2 (up to
    /// 0.5.8 only the small noise model); never again, so the user's later
    /// choice holds.
    pub lean_suggested: bool,
    /// Ask GitHub once a day whether there is a newer kyrr.
    pub update_check: bool,
    /// When kyrr last asked, in seconds since 1970.
    pub update_checked: u64,
    /// The version whose note the user closed; it stays away until the next.
    pub update_dismissed: String,
    /// The settings page shows everything, not just the basics.
    pub advanced_settings: bool,
    /// The microphone Windows used by default when the user picked another
    /// one on purpose; kyrr leaves that pick until the default changes.
    pub pinned_default: String,
    /// The version that ran last: when it differs, kyrr was updated and shows
    /// what the new version brings, once.
    pub last_run_version: String,
}

/// The language setting for dictation in many languages.
pub const MULTI: &str = "multi";

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
            dictation_live: true,
            lean_suggested: false,
            update_check: true,
            update_checked: 0,
            update_dismissed: String::new(),
            advanced_settings: false,
            pinned_default: String::new(),
            last_run_version: String::new(),
        }
    }
}

impl Config {
    /// The saved settings. A file that does not read is moved aside as
    /// config.json.bak, so the next save does not overwrite what the user
    /// might still want back.
    pub fn load() -> Config {
        Config::load_from(&config_file())
    }

    fn load_from(path: &Path) -> Config {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
            Err(e) => {
                eprintln!("kyrr: {} does not read ({e})", path.display());
                return Config::default();
            }
        };
        let (config, whole) = parse(&bytes);
        if !whole {
            eprintln!("kyrr: {} does not read fully; kept as config.json.bak", path.display());
            let _ = std::fs::copy(path, path.with_extension("json.bak"));
        }
        config
    }

    pub fn save(&self) {
        self.save_to(&config_file());
    }

    fn save_to(&self, path: &Path) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            if let Err(e) = write_atomic(path, &json) {
                eprintln!("kyrr: could not save {}: {e}", path.display());
            }
        }
    }

    /// The interface language.
    pub fn language(&self) -> Language {
        if self.more_languages() {
            return system_language();
        }
        Language::from_code(&self.language).unwrap_or(Language::English)
    }

    /// Dictation with the model for many European languages.
    pub fn more_languages(&self) -> bool {
        self.language == MULTI
    }

    /// What the language choice shows: German, English, multilingual.
    pub fn language_index(&self) -> i32 {
        if self.more_languages() {
            2
        } else {
            self.language() as i32
        }
    }
}

/// The settings in `bytes`, and whether all of them were understood. One
/// value of the wrong kind (edited by hand, written by a newer kyrr) costs
/// only that value, not every setting and the finished setup with it.
fn parse(bytes: &[u8]) -> (Config, bool) {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_slice(bytes) else {
        return (Config::default(), false);
    };
    if let Ok(config) = serde_json::from_value(serde_json::Value::Object(map.clone())) {
        return (config, true);
    }
    let good: serde_json::Map<_, _> = map
        .into_iter()
        .filter(|(k, v)| {
            let one = serde_json::Map::from_iter([(k.clone(), v.clone())]);
            serde_json::from_value::<Config>(serde_json::Value::Object(one)).is_ok()
        })
        .collect();
    let config = serde_json::from_value(serde_json::Value::Object(good)).unwrap_or_default();
    (config, false)
}

/// Writes `bytes` to a file next to `path`, onto the disk, and renames it
/// into place, so a crash or a full disk never leaves half a file. Right
/// after writing, a virus scanner may hold the file on Windows; the rename
/// is tried again for a moment. Only the user may read the file (Linux).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    // A leftover from a crash would keep its old permissions.
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let mut result = written;
    for _ in 0..5 {
        if result.is_err() {
            break;
        }
        result = std::fs::rename(&tmp, path);
        if result.is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&tmp);
    result
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
    // In the order gettext asks: LANGUAGE is a list like "de_AT:de".
    #[cfg(not(windows))]
    let locale = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|v| std::env::var(v).ok())
        .find_map(|v| v.split(':').map(str::to_string).find(|l| !l.is_empty()))
        .unwrap_or_default();
    match Language::from_code(&locale) {
        Some(Language::German) => Language::German,
        _ => Language::English,
    }
}

/// The folder in `var`, else `fallback` below the home folder. Empty or
/// relative values do not count (as the XDG rules say): they would put the
/// settings wherever kyrr happened to be started.
fn base_dir(var: &str, fallback: &str) -> PathBuf {
    let absolute = |v: std::ffi::OsString| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    std::env::var_os(var).and_then(absolute).unwrap_or_else(|| {
        std::env::var_os("HOME")
            .and_then(absolute)
            .unwrap_or_else(std::env::temp_dir)
            .join(fallback)
    })
}

/// kyrr's settings folder.
#[cfg_attr(windows, allow(dead_code))]
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let dir = base_dir("APPDATA", "AppData/Roaming");
    #[cfg(not(windows))]
    let dir = base_dir("XDG_CONFIG_HOME", ".config");
    dir.join("kyrr")
}

fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// Where the desktop looks for a user's own launchers and icons.
#[cfg(target_os = "linux")]
pub fn data_home() -> PathBuf {
    base_dir("XDG_DATA_HOME", ".local/share")
}

/// Where the dictation models are downloaded to.
pub fn models_dir() -> PathBuf {
    #[cfg(windows)]
    let dir = base_dir("LOCALAPPDATA", "AppData/Local");
    #[cfg(not(windows))]
    let dir = base_dir("XDG_DATA_HOME", ".local/share");
    dir.join("kyrr").join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_saving() {
        let dir = std::env::temp_dir().join(format!("kyrr-config-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let config = Config {
            language: "de".into(),
            setup_done: true,
            dictation_key: DICTATION_KEYS[2],
            update_checked: 123,
            ..Config::default()
        };
        config.save_to(&path);
        let back = Config::load_from(&path);
        assert_eq!(
            serde_json::to_value(&back).unwrap(),
            serde_json::to_value(&config).unwrap()
        );
        assert!(!dir.join("config.json.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn one_bad_value_costs_only_itself() {
        let (config, whole) = parse(br#"{"setup_done": true, "dictation": "yes", "language": "de", "future": 1}"#);
        assert!(!whole);
        assert!(config.setup_done);
        assert_eq!(config.language, "de");
        assert_eq!(config.dictation, Config::default().dictation);
    }

    #[test]
    fn older_and_newer_files_read() {
        // Missing fields take their defaults, unknown ones are ignored.
        let (config, whole) = parse(br#"{"setup_done": true, "something_new": [1, 2]}"#);
        assert!(whole && config.setup_done && config.update_check);
    }

    #[test]
    fn garbage_gives_defaults() {
        for bytes in [&b"\xff\xfe"[..], b"", b"[1,2]", b"{\"setup_done\": tr"] {
            let (config, whole) = parse(bytes);
            assert!(!whole && !config.setup_done);
        }
    }
}
