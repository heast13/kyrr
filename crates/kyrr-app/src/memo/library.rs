//! The folder of voice memos: Documents/kyrr. A memo is an Opus file named
//! `2026-10-21 18-05 Title.opus`; its text, once written, lies beside it as
//! `2026-10-21 18-05 Title.txt`. The folder is the list: what is in it is
//! shown, nothing else is kept about the memos.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};

use super::{opusfile, title};

/// How the date and time start a memo's name.
const STAMP: &str = "%Y-%m-%d %H-%M";
const STAMP_LEN: usize = 16;

/// One memo in the folder.
#[derive(Clone, Debug)]
pub struct Memo {
    pub path: PathBuf,
    /// The name after the date.
    pub title: String,
    /// From the name, else when the file was last written.
    pub when: NaiveDateTime,
    pub seconds: f64,
    /// Its text, when it has one.
    pub text: Option<Vec<Paragraph>>,
}

/// A paragraph of a memo's text: where in the memo it starts, and what was
/// said.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub seconds: u32,
    pub text: String,
}

/// The folder: kyrr in the user's Documents folder.
pub fn folder() -> PathBuf {
    documents().join("kyrr")
}

#[cfg(windows)]
fn documents() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    // SAFETY: the returned string is read once and freed as the call asks.
    let known = unsafe {
        SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None)
            .ok()
            .map(|p| {
                let path = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const _));
                path
            })
    };
    match known.flatten() {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => home().join("Documents"),
    }
}

#[cfg(not(windows))]
fn documents() -> PathBuf {
    let home = home();
    // The Flatpak sees the user's own settings file in its config folder.
    let configs = [
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOST_XDG_CONFIG_HOME").map(PathBuf::from),
        Some(home.join(".config")),
    ];
    configs
        .into_iter()
        .flatten()
        .find_map(|dir| std::fs::read_to_string(dir.join("user-dirs.dirs")).ok())
        .and_then(|dirs| user_dir(&dirs, "XDG_DOCUMENTS_DIR", &home))
        .unwrap_or_else(|| home.join("Documents"))
}

/// A folder from `user-dirs.dirs`, as in `XDG_DOCUMENTS_DIR="$HOME/Dokumente"`;
/// None when it is not there or is the home folder (switched off).
#[cfg_attr(windows, allow(dead_code))]
fn user_dir(dirs: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let line = dirs.lines().map(str::trim).find(|l| {
        l.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    })?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None if value.starts_with('/') => PathBuf::from(value),
        None => return None,
    };
    (path != home && path.as_os_str().len() > 1).then_some(path)
}

fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// The memos in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Memo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut memos: Vec<Memo> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("opus") {
                return None;
            }
            let meta = entry.metadata().ok().filter(|m| m.is_file())?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let (when, title) = match parse(&stem) {
                Some(found) => found,
                None => {
                    let modified = meta.modified().ok()?;
                    (chrono::DateTime::<Local>::from(modified).naive_local(), stem)
                }
            };
            Some(Memo {
                seconds: opusfile::seconds(&path).unwrap_or(0.0),
                text: read_text(&path),
                path,
                title,
                when,
            })
        })
        .collect();
    memos.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| b.path.cmp(&a.path)));
    memos
}

/// The date and title in a memo's name, when it is one of kyrr's.
fn parse(stem: &str) -> Option<(NaiveDateTime, String)> {
    let stamp = stem.get(..STAMP_LEN)?;
    let when = NaiveDateTime::parse_from_str(stamp, STAMP).ok()?;
    Some((when, stem[STAMP_LEN..].trim().to_string()))
}

/// A free name in `dir` for a memo recorded at `when` with `title`; a
/// second one in the same minute with the same title gets " (2)".
pub fn free_path(dir: &Path, when: NaiveDateTime, title: &str) -> PathBuf {
    let title = title::clean(title);
    let base = format!("{} {title}", when.format(STAMP));
    let base = base.trim_end();
    (1..)
        .map(|n| match n {
            1 => dir.join(format!("{base}.opus")),
            n => dir.join(format!("{base} ({n}).opus")),
        })
        .find(|p| !p.exists() && !text_path(p).exists())
        .expect("a free name")
}

/// The name for a memo started now.
pub fn new_path(dir: &Path, placeholder: &str) -> PathBuf {
    free_path(dir, Local::now().naive_local(), placeholder)
}

/// Gives the memo at `path` the title `title` (its date stays); its text
/// goes along. Returns where it is now.
pub fn rename(path: &Path, title: &str) -> io::Result<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let when = match parse(&stem) {
        Some((_, old)) if old == title::clean(title) => return Ok(path.to_path_buf()),
        Some((when, _)) => when,
        None => Local::now().naive_local(),
    };
    if title::clean(title).is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty title"));
    }
    let to = free_path(dir, when, title);
    std::fs::rename(path, &to)?;
    let text = text_path(path);
    if text.exists() {
        let _ = std::fs::rename(&text, text_path(&to));
    }
    Ok(to)
}

/// Deletes the memo and its text, for good.
pub fn delete(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)?;
    let text = text_path(path);
    if text.exists() {
        let _ = std::fs::remove_file(text);
    }
    Ok(())
}

/// Where the text of the memo at `path` lies.
pub fn text_path(path: &Path) -> PathBuf {
    path.with_extension("txt")
}

/// Minutes and seconds, as in 1:05, or with hours, as in 1:02:05.
pub fn clock(seconds: u32) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Writes the text of the memo at `path`: each paragraph with where it
/// starts, a blank line between them. Paragraphs without words are left
/// out. Written whole or not at all.
pub fn write_text(path: &Path, paragraphs: &[Paragraph]) -> io::Result<PathBuf> {
    let body: Vec<String> = paragraphs
        .iter()
        .filter(|p| !p.text.trim().is_empty())
        .map(|p| format!("[{}] {}", clock(p.seconds), p.text.trim()))
        .collect();
    let to = text_path(path);
    let part = to.with_extension("txt.part");
    std::fs::write(&part, body.join("\n\n") + "\n")?;
    std::fs::rename(&part, &to)?;
    Ok(to)
}

/// The text of the memo at `path`, if it has one.
pub fn read_text(path: &Path) -> Option<Vec<Paragraph>> {
    let text = std::fs::read_to_string(text_path(path)).ok()?;
    Some(paragraphs(&text))
}

/// The paragraphs of a memo's text, as [`write_text`] writes them; a text
/// the user changed by hand still reads, without times where they are gone.
fn paragraphs(text: &str) -> Vec<Paragraph> {
    text.replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let timed = p.strip_prefix('[').and_then(|rest| {
                let (time, text) = rest.split_once(']')?;
                Some((parse_clock(time)?, text.trim()))
            });
            match timed {
                Some((seconds, text)) => Paragraph {
                    seconds,
                    text: text.to_string(),
                },
                None => Paragraph {
                    seconds: u32::MAX,
                    text: p.to_string(),
                },
            }
        })
        .collect()
}

fn parse_clock(time: &str) -> Option<u32> {
    let parts: Vec<u32> = time.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [m, s] if s < 60 => Some(m * 60 + s),
        [h, m, s] if m < 60 && s < 60 => Some(h * 3600 + m * 60 + s),
        _ => None,
    }
}

/// The whole text, for the clipboard: the paragraphs without their times.
pub fn plain(paragraphs: &[Paragraph]) -> String {
    paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Opens `path` (a folder, or a memo's text) with the app the system has
/// for it. Blocks for a moment on Linux: call it beside the window.
pub fn open(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        kyrr_linux::files::open(path);
    }
}

/// Opens the folder with the memo at `path` picked out.
pub fn show(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer takes the path only as one piece with the switch.
        let _ = std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        kyrr_linux::files::show(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kyrr-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn documents_from_the_user_dirs() {
        let home = Path::new("/home/flo");
        let dirs = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n";
        assert_eq!(
            user_dir(dirs, "XDG_DOCUMENTS_DIR", home),
            Some(PathBuf::from("/home/flo/Dokumente"))
        );
        assert_eq!(
            user_dir("XDG_DOCUMENTS_DIR=\"/data/docs\"", "XDG_DOCUMENTS_DIR", home),
            Some(PathBuf::from("/data/docs"))
        );
        assert_eq!(
            user_dir("XDG_DOCUMENTS_DIR=\"$HOME/\"", "XDG_DOCUMENTS_DIR", home),
            None
        );
        assert_eq!(
            user_dir("XDG_DOCUMENTS_DIRX=\"$HOME/x\"", "XDG_DOCUMENTS_DIR", home),
            None
        );
        assert_eq!(user_dir("", "XDG_DOCUMENTS_DIR", home), None);
    }

    #[test]
    fn names_date_and_title() {
        let dir = temp("names");
        let when = NaiveDateTime::parse_from_str("2026-10-21 18-05", STAMP).unwrap();
        let first = free_path(&dir, when, "Einkauf: Samstag");
        assert_eq!(first.file_name().unwrap(), "2026-10-21 18-05 Einkauf Samstag.opus");
        std::fs::write(&first, b"x").unwrap();
        let second = free_path(&dir, when, "Einkauf Samstag");
        assert_eq!(second.file_name().unwrap(), "2026-10-21 18-05 Einkauf Samstag (2).opus");
        assert_eq!(
            parse("2026-10-21 18-05 Einkauf Samstag"),
            Some((when, "Einkauf Samstag".into()))
        );
        assert_eq!(parse("Meine Aufnahme"), None);

        // Renaming keeps the date and takes the text along.
        std::fs::write(text_path(&first), "[0:00] Milch\n").unwrap();
        let renamed = rename(&first, "Wochenmarkt").unwrap();
        assert_eq!(renamed.file_name().unwrap(), "2026-10-21 18-05 Wochenmarkt.opus");
        assert!(!first.exists() && renamed.exists());
        assert_eq!(read_text(&renamed).unwrap()[0].text, "Milch");
        assert_eq!(rename(&renamed, "Wochenmarkt").unwrap(), renamed);
        assert!(rename(&renamed, " / ").is_err());

        delete(&renamed).unwrap();
        assert!(!renamed.exists() && !text_path(&renamed).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_list_is_newest_first() {
        let dir = temp("list");
        for name in [
            "2026-10-20 09-00 Alt.opus",
            "2026-10-21 18-05 Neu.opus",
            "Eigene.opus",
            "notiz.txt",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let memos = list(&dir);
        let titles: Vec<&str> = memos.iter().map(|m| m.title.as_str()).collect();
        // A file of the user's own is from now (its time), so first.
        assert_eq!(titles, ["Eigene", "Neu", "Alt"]);
        assert!(memos.iter().all(|m| m.seconds == 0.0 && m.text.is_none()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn texts_read_back() {
        let dir = temp("text");
        let memo = dir.join("2026-10-21 18-05 Test.opus");
        let written_out = vec![
            Paragraph {
                seconds: 0,
                text: "Erster Absatz.".into(),
            },
            Paragraph {
                seconds: 30,
                text: "   ".into(),
            },
            Paragraph {
                seconds: 3725,
                text: "Später.".into(),
            },
        ];
        write_text(&memo, &written_out).unwrap();
        let written = std::fs::read_to_string(text_path(&memo)).unwrap();
        assert_eq!(written, "[0:00] Erster Absatz.\n\n[1:02:05] Später.\n");
        let back = read_text(&memo).unwrap();
        assert_eq!(back, vec![written_out[0].clone(), written_out[2].clone()]);
        assert_eq!(plain(&back), "Erster Absatz.\n\nSpäter.");
        assert_eq!(
            paragraphs("Von Hand\r\n\r\n[0:05] dazu"),
            vec![
                Paragraph {
                    seconds: u32::MAX,
                    text: "Von Hand".into()
                },
                Paragraph {
                    seconds: 5,
                    text: "dazu".into()
                }
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
