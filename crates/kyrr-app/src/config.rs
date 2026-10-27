//! The app's own settings (the filter's live in the registry, see
//! `kyrr_windows::filter`) and where kyrr keeps its files.

use std::path::{Path, PathBuf};

use kyrr_common::keys;
use kyrr_dictate::Language;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "de" or "en": user interface and dictation model. "multi": dictation
    /// with the model for many European languages, the interface in English
    /// (the language someone who speaks any of them most likely reads).
    pub language: String,
    pub dictation: bool,
    /// First-run setup finished.
    pub setup_done: bool,
    /// The dictation key, a Windows virtual-key code (see `kyrr_common::keys`
    /// for the ones kyrr offers); [`Config::dictation_key`] says what counts.
    pub dictation_key: u32,
    /// Tap once to start and once to stop, instead of holding the key.
    pub dictation_toggle: bool,
    /// The wave shows the words heard so far while the key is held.
    pub dictation_live: bool,
    /// The lean setting was picked once because the CPU lacks AVX2 (up to
    /// 0.5.8 only the small noise model); never again, so the user's later
    /// choice holds.
    pub lean_suggested: bool,
    /// A computer that cannot keep up with the best noise model switches to
    /// the lean setting by itself; off, the filter only pauses a while.
    pub auto_lean: bool,
    /// Ask GitHub once a day whether there is a newer kyrr.
    pub update_check: bool,
    /// When kyrr last asked, in seconds since 1970.
    pub update_checked: u64,
    /// The version whose note the user closed; it stays away until the next.
    pub update_dismissed: String,
    /// The microphone Windows used by default when the user picked another
    /// one on purpose; kyrr leaves that pick until the default changes.
    pub pinned_default: String,
    /// The version that ran last: when it differs, kyrr was updated and shows
    /// what the new version brings, once.
    pub last_run_version: String,
    /// Voice memos: the card on the main page, the key and the tray entry.
    pub memos: bool,
    /// The voice memo key as a virtual-key code; [`Config::memo_key`] says
    /// what counts.
    pub memo_key: u32,
    /// Tap once to start a memo and once to stop it, instead of holding the key.
    pub memo_toggle: bool,
    /// Every new memo gets its text right away, not just on a click.
    pub memo_text_auto: bool,
    /// What the memo text field shows before each paragraph: 0 nothing,
    /// 1 how far into the memo, 2 the time of day.
    pub memo_stamps: u8,
    /// When the user last looked at the memo list, in seconds since 1970;
    /// memos newer than that are "new" (the dot on the main page). 0: never
    /// set, kyrr takes the first start as the moment.
    pub memos_seen: u64,
    /// Holding the memo key became the standard in 0.6.1: a settings file
    /// without this (from 0.6.0, where "tap" was only the standard) gets
    /// "hold" once; after that the user's own choice stays.
    #[serde(default = "never")]
    pub memo_hold_default: bool,
    /// The user's own language model that reworks dictations (see
    /// `kyrr_server`): its address ("" for none) and model.
    pub rework_url: String,
    pub rework_model: String,
    /// What it does with a dictation: 0 nothing, 1 fix what was misheard,
    /// 2 reword it, 3 translate it.
    pub rework: u8,
    /// The language translations go into, a code.
    pub rework_target: String,
    /// Dictation through the user's own speech server instead of the
    /// built-in model, its address and model.
    pub asr_server: bool,
    pub asr_url: String,
    pub asr_model: String,
}

/// A setting that an older settings file does not have yet.
fn never() -> bool {
    false
}

/// The language setting for dictation in many languages.
pub const MULTI: &str = "multi";

impl Default for Config {
    fn default() -> Self {
        Config {
            language: system_language().code().into(),
            dictation: true,
            setup_done: false,
            dictation_key: keys::DICTATION_DEFAULT,
            dictation_toggle: false,
            dictation_live: true,
            lean_suggested: false,
            auto_lean: true,
            update_check: true,
            update_checked: 0,
            update_dismissed: String::new(),
            pinned_default: String::new(),
            last_run_version: String::new(),
            memos: true,
            memo_key: keys::MEMO_DEFAULT,
            memo_toggle: false,
            memo_text_auto: false,
            memo_stamps: 0,
            memos_seen: 0,
            memo_hold_default: true,
            rework_url: String::new(),
            rework_model: String::new(),
            rework: 0,
            rework_target: "en".into(),
            asr_server: false,
            asr_url: String::new(),
            asr_model: String::new(),
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
        let (mut config, whole) = parse(&bytes);
        if !config.memo_hold_default {
            config.memo_toggle = false;
            config.memo_hold_default = true;
        }
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

    /// The interface language: English while dictating in many languages.
    pub fn language(&self) -> Language {
        if self.more_languages() {
            return Language::English;
        }
        Language::from_code(&self.language).unwrap_or(Language::English)
    }

    /// The dictation key as it works: one kyrr offers, else the standard.
    pub fn dictation_key(&self) -> u32 {
        if keys::allowed(self.dictation_key) {
            self.dictation_key
        } else {
            keys::DICTATION_DEFAULT
        }
    }

    /// The voice memo key as it works: never dictation's, which goes first
    /// (the next free one then); 0 while memos are off.
    pub fn memo_key(&self) -> u32 {
        if self.memos {
            self.memo_key_choice()
        } else {
            0
        }
    }

    /// The voice memo key whether memos are on or not. A key kyrr does not
    /// offer (0, what "none" was in the unreleased 0.6.0 builds) is the
    /// standard.
    pub fn memo_key_choice(&self) -> u32 {
        let dictation = self.dictation_key();
        let wanted = if keys::allowed(self.memo_key) {
            self.memo_key
        } else {
            keys::MEMO_DEFAULT
        };
        if wanted != dictation {
            return wanted;
        }
        [keys::MEMO_DEFAULT, keys::DICTATION_DEFAULT, keys::PAUSE]
            .into_iter()
            .find(|&k| k != dictation)
            .unwrap_or(0)
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
    if result.is_ok() {
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            result = std::fs::rename(&tmp, path);
            if result.is_ok() {
                return Ok(());
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    result
}

/// The keys for the user's own servers, if they need one: kept apart from
/// the settings, so a settings file shown to someone does not give them
/// away. Only the user may read the file (Linux; on Windows it lies in the
/// user's own profile).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub rework: String,
    pub asr: String,
}

impl Keys {
    pub fn load() -> Keys {
        std::fs::read(keys_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = keys_file();
        if self.rework.is_empty() && self.asr.is_empty() {
            let _ = std::fs::remove_file(path);
        } else if let Ok(json) = serde_json::to_vec(self) {
            if let Err(e) = write_atomic(&path, &json) {
                eprintln!("kyrr: could not save the server keys: {e}");
            }
        }
    }
}

fn keys_file() -> PathBuf {
    config_dir().join("keys.json")
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
    fn many_languages_means_an_english_interface() {
        let config = Config {
            language: MULTI.into(),
            ..Config::default()
        };
        assert_eq!(config.language(), Language::English);
        assert_eq!(config.language_index(), 2);
        let config = Config {
            language: "de".into(),
            ..Config::default()
        };
        assert_eq!(config.language(), Language::German);
    }

    #[test]
    fn the_memo_key_is_never_dictations() {
        let config = Config::default();
        assert_eq!(config.dictation_key(), keys::F9);
        assert_eq!(config.memo_key(), keys::F8);
        let config = Config {
            dictation_key: keys::F8,
            ..Config::default()
        };
        assert_eq!(config.memo_key(), keys::F9);
        let config = Config {
            dictation_key: keys::F8,
            memo_key: keys::F9,
            ..Config::default()
        };
        assert_eq!(config.memo_key(), keys::F9);
        let config = Config {
            memos: false,
            ..Config::default()
        };
        assert_eq!(config.memo_key(), 0);
    }

    #[test]
    fn keys_kyrr_does_not_offer_fall_back_to_the_standard() {
        // 0 was "none" for the memo key; 0x41 is the letter A.
        let config = Config {
            memo_key: 0,
            dictation_key: 0x41,
            ..Config::default()
        };
        assert_eq!(config.dictation_key(), keys::F9);
        assert_eq!(config.memo_key(), keys::F8);
    }

    #[test]
    fn holding_the_memo_key_is_the_standard_once() {
        let dir = std::env::temp_dir().join(format!("kyrr-config-hold-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        // 0.6.0 saved "tap" without anyone choosing it.
        std::fs::write(&path, br#"{"memo_toggle": true}"#).unwrap();
        let config = Config::load_from(&path);
        assert!(!config.memo_toggle && config.memo_hold_default);
        // From then on the user's choice stays.
        Config {
            memo_toggle: true,
            ..config
        }
        .save_to(&path);
        assert!(Config::load_from(&path).memo_toggle);
        // A new install holds, too.
        assert!(!Config::default().memo_toggle);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keys_of_older_versions_stay() {
        // F8, F9, Pause and right Ctrl were the choices up to 0.5.8.
        for vk in [0x77, 0x78, 0x13, 0xA3] {
            let config = Config {
                dictation_key: vk,
                ..Config::default()
            };
            assert_eq!(config.dictation_key(), vk);
        }
    }

    #[test]
    fn settings_survive_saving() {
        let dir = std::env::temp_dir().join(format!("kyrr-config-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let config = Config {
            language: "de".into(),
            setup_done: true,
            dictation_key: keys::PAUSE,
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
