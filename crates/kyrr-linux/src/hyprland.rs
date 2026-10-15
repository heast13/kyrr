//! Hyprland binds keys in its own config, not in a dialog: for the dictation
//! key a few lines have to be there. kyrr writes them itself, as a block
//! between two marker lines that it alone rewrites (when the key changes), and
//! leaves everything else in the file as it is. A key that another bind in the
//! user's files already uses is left alone.
//!
//! Hyprland reloads its config when the file changes. Omarchy keeps its own
//! key bindings in `bindings.conf`, and newer Hyprland versions read
//! `hyprland.lua` or `bindings.lua` instead of the `.conf` files when they
//! exist. Only the standard library is used here.

use std::path::{Path, PathBuf};

const APP_ID: &str = "io.github.heast13.kyrr";
/// The shortcut's name in `bind = , F9, global, <APP_ID>:dictate`.
const GLOBAL: &str = "io.github.heast13.kyrr:dictate";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Conf,
    Lua,
}

impl Syntax {
    fn comment(self) -> &'static str {
        match self {
            Syntax::Conf => "#",
            Syntax::Lua => "--",
        }
    }
}

/// Where the dictation key goes, and how it reaches kyrr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// The key's name, e.g. "F9".
    pub key: String,
    /// Through the GlobalShortcuts portal; otherwise the key runs `start` on
    /// press and `stop` on release.
    pub portal: bool,
    pub start: String,
    pub stop: String,
}

/// How setting the key up went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The lines are in the file. `written` when they were not before, so
    /// Hyprland has to read the file again.
    Bound { file: PathBuf, written: bool },
    /// Another bind of the user's already uses the key.
    Taken { file: PathBuf },
    /// The file cannot be found or written (Flatpak without access, a config
    /// kept read-only): the lines have to be added by hand.
    Failed,
}

/// The config file that gets the key bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub syntax: Syntax,
}

/// Hyprland's config folder. In a Flatpak `XDG_CONFIG_HOME` is the app's own
/// folder, the user's lies at `HOST_XDG_CONFIG_HOME`.
pub fn config_dir() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = if std::env::var_os("FLATPAK_ID").is_some() {
        var("HOST_XDG_CONFIG_HOME")
    } else {
        var("XDG_CONFIG_HOME")
    };
    base.or_else(|| var("HOME").map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("hypr")
}

/// The files Hyprland reads its binds from, the one kyrr writes to first.
/// Hyprland reads `hyprland.lua` instead of `hyprland.conf` when it exists. A
/// separate bindings file (Omarchy's `bindings.conf`) counts when the main
/// file mentions it, and stays out of it otherwise.
fn files(dir: &Path) -> Vec<Target> {
    let (main, bindings, syntax) = if dir.join("hyprland.lua").is_file() {
        ("hyprland.lua", "bindings.lua", Syntax::Lua)
    } else {
        ("hyprland.conf", "bindings.conf", Syntax::Conf)
    };
    let target = |name: &str| Target {
        path: dir.join(name),
        syntax,
    };
    let Ok(text) = std::fs::read_to_string(dir.join(main)) else {
        return Vec::new();
    };
    if text.contains(bindings) && dir.join(bindings).is_file() {
        vec![target(bindings), target(main)]
    } else {
        vec![target(main)]
    }
}

/// The file the key's lines go to, `None` when Hyprland has no config there.
pub fn target(dir: &Path) -> Option<Target> {
    files(dir).into_iter().next()
}

/// The lines that bind the key.
pub fn lines(syntax: Syntax, keys: &Keys) -> String {
    let key = &keys.key;
    // Omarchy puts its own tools on some keys; unbinding frees the key.
    match (syntax, keys.portal) {
        (Syntax::Conf, true) => format!("unbind = , {key}\nbind = , {key}, global, {GLOBAL}"),
        (Syntax::Conf, false) => format!(
            "unbind = , {key}\nbind = , {key}, exec, {}\nbindr = , {key}, exec, {}",
            keys.start, keys.stop
        ),
        (Syntax::Lua, true) => format!("hl.unbind(\"{key}\")\nhl.bind(\"{key}\", hl.dsp.global(\"{GLOBAL}\"))"),
        (Syntax::Lua, false) => format!(
            "hl.unbind(\"{key}\")\nhl.bind(\"{key}\", hl.dsp.exec_cmd(\"{}\"))\nhl.bind(\"{key}\", hl.dsp.exec_cmd(\"{}\"), {{ release = true }})",
            lua_quoted(&keys.start),
            lua_quoted(&keys.stop)
        ),
    }
}

/// `s` for inside a double-quoted Lua string.
fn lua_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The path with the home folder as `~`, for the user to read.
pub fn display(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Puts the key's lines into the config in `dir`, or brings the ones kyrr put
/// there before up to date.
pub fn ensure(dir: &Path, keys: &Keys) -> Outcome {
    let files = files(dir);
    let Some(target) = files.first() else {
        return Outcome::Failed;
    };
    let mut taken = None;
    for file in &files {
        let Ok(content) = std::fs::read_to_string(&file.path) else {
            return Outcome::Failed;
        };
        let Ok(rest) = strip_block(&content, file.syntax) else {
            return Outcome::Failed;
        };
        match claim(&rest, file.syntax, &keys.key) {
            Claim::Free => {}
            Claim::Kyrr => {
                return Outcome::Bound {
                    file: file.path.clone(),
                    written: false,
                }
            }
            Claim::Other => taken = taken.or(Some(file.path.clone())),
        }
    }
    if let Some(file) = taken {
        return Outcome::Taken { file };
    }
    // Dotfile managers link the config: write through the link.
    let path = std::fs::canonicalize(&target.path).unwrap_or_else(|_| target.path.clone());
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Outcome::Failed;
    };
    let new = match with_block(&text, target.syntax, keys) {
        Ok(Some(new)) => new,
        Ok(None) => {
            return Outcome::Bound {
                file: target.path.clone(),
                written: false,
            }
        }
        Err(()) => return Outcome::Failed,
    };
    match write_over(&path, &new) {
        Ok(()) => Outcome::Bound {
            file: target.path.clone(),
            written: true,
        },
        Err(e) => {
            eprintln!("kyrr: writing {}: {e}", path.display());
            Outcome::Failed
        }
    }
}

/// Replaces the file's content in one step, so a crash leaves the old file.
fn write_over(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("kyrr-tmp");
    let result = std::fs::write(&tmp, text)
        .and_then(|()| std::fs::set_permissions(&tmp, std::fs::metadata(path)?.permissions()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The lines that open and close kyrr's block. A later kyrr may word the
/// note after `>>> kyrr` differently, so the start is found by its beginning.
fn markers(syntax: Syntax) -> (String, String) {
    let c = syntax.comment();
    (
        format!("{c} >>> kyrr: dictation key (kyrr keeps this block up to date)"),
        format!("{c} <<< kyrr"),
    )
}

fn is_start(row: &str, syntax: Syntax) -> bool {
    row.trim().starts_with(&format!("{} >>> kyrr", syntax.comment()))
}

/// Where kyrr's block lies in `rows`: the start and end row. `Err` when a
/// start has no end after it: not kyrr's to rewrite, the user touched it.
fn find_block(rows: &[&str], syntax: Syntax) -> Result<Option<(usize, usize)>, ()> {
    let (_, end) = markers(syntax);
    let Some(a) = rows.iter().position(|r| is_start(r, syntax)) else {
        return Ok(None);
    };
    match rows[a..].iter().position(|r| r.trim() == end) {
        Some(b) => Ok(Some((a, a + b))),
        None => Err(()),
    }
}

/// `text` with kyrr's block, as it should be: `None` when it already is.
fn with_block(text: &str, syntax: Syntax, keys: &Keys) -> Result<Option<String>, ()> {
    let (start, end) = markers(syntax);
    let block = format!("{start}\n{}\n{end}\n", lines(syntax, keys));
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    let new = match find_block(&rows, syntax)? {
        Some((a, b)) => format!("{}{block}{}", rows[..a].concat(), rows[b + 1..].concat()),
        None => {
            let mut new = text.to_string();
            if !new.is_empty() {
                new.push_str(if new.ends_with('\n') { "\n" } else { "\n\n" });
            }
            new.push_str(&block);
            new
        }
    };
    Ok((new != text).then_some(new))
}

/// `text` without kyrr's own block.
fn strip_block(text: &str, syntax: Syntax) -> Result<String, ()> {
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    Ok(match find_block(&rows, syntax)? {
        Some((a, b)) => format!("{}{}", rows[..a].concat(), rows[b + 1..].concat()),
        None => text.to_string(),
    })
}

enum Claim {
    Free,
    /// A bind of kyrr's that was added by hand, as an earlier kyrr asked.
    Kyrr,
    /// Something else is on the key.
    Other,
}

/// Who has the bare `key` in `text`.
fn claim(text: &str, syntax: Syntax, key: &str) -> Claim {
    let mut claim = Claim::Free;
    for row in text.lines().map(str::trim) {
        let on_key = match syntax {
            Syntax::Conf => {
                // `bind = MODS, KEY, ...`, also bindr, bindd and the others.
                let Some((name, rest)) = row.split_once('=') else {
                    continue;
                };
                let mut fields = rest.split(',').map(str::trim);
                name.trim().starts_with("bind")
                    && fields.next() == Some("")
                    && fields.next().is_some_and(|k| k.eq_ignore_ascii_case(key))
            }
            Syntax::Lua => row.split_once("hl.bind(").is_some_and(|(_, rest)| {
                let rest = rest.trim_start();
                let quoted = |q: char| rest.strip_prefix(q).and_then(|r| r.split(q).next());
                [quoted('"'), quoted('\'')]
                    .into_iter()
                    .flatten()
                    .any(|k| k.eq_ignore_ascii_case(key))
            }),
        };
        if !on_key || row.starts_with(syntax.comment()) {
            continue;
        }
        if row.contains(APP_ID) || row.contains("--dictate") {
            return Claim::Kyrr;
        }
        claim = Claim::Other;
    }
    claim
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(key: &str, portal: bool) -> Keys {
        Keys {
            key: key.into(),
            portal,
            start: "kyrr --dictate-start".into(),
            stop: "kyrr --dictate-stop".into(),
        }
    }

    fn block(text: &str, syntax: Syntax, keys: &Keys) -> Option<String> {
        with_block(text, syntax, keys).unwrap()
    }

    #[test]
    fn appends_once_and_leaves_the_rest() {
        let old = "bind = SUPER, Q, killactive\n# my notes";
        let new = block(old, Syntax::Conf, &keys("F9", true)).unwrap();
        assert!(new.starts_with("bind = SUPER, Q, killactive\n# my notes\n\n# >>> kyrr"));
        assert!(new.contains("unbind = , F9\nbind = , F9, global, io.github.heast13.kyrr:dictate\n"));
        assert!(new.ends_with("# <<< kyrr\n"));
        assert_eq!(block(&new, Syntax::Conf, &keys("F9", true)), None);
    }

    #[test]
    fn changing_the_key_rewrites_only_the_block() {
        let first = block("a = 1\n", Syntax::Conf, &keys("F9", true)).unwrap();
        let tail = format!("{first}b = 2\n");
        let new = block(&tail, Syntax::Conf, &keys("F8", false)).unwrap();
        assert!(new.starts_with("a = 1\n\n# >>> kyrr"));
        assert!(new.contains("bind = , F8, exec, kyrr --dictate-start\nbindr = , F8, exec, kyrr --dictate-stop\n"));
        assert!(!new.contains("F9"));
        assert!(new.ends_with("# <<< kyrr\nb = 2\n"));
        assert_eq!(new.matches(">>> kyrr").count(), 1);
    }

    #[test]
    fn empty_file_and_lua() {
        let new = block("", Syntax::Lua, &keys("F9", true)).unwrap();
        assert!(new.starts_with("-- >>> kyrr"));
        assert!(new.contains("hl.unbind(\"F9\")\nhl.bind(\"F9\", hl.dsp.global("));
        assert_eq!(block(&new, Syntax::Lua, &keys("F9", true)), None);
    }

    #[test]
    fn strips_its_block_only() {
        let text = block("keep\n", Syntax::Conf, &keys("F9", true)).unwrap();
        assert_eq!(strip_block(&text, Syntax::Conf), Ok("keep\n\n".to_string()));
    }

    #[test]
    fn a_start_without_an_end_is_left_alone() {
        let text = "a = 1\n# >>> kyrr: mine\nbind = , F9, exec, mute\n";
        assert_eq!(with_block(text, Syntax::Conf, &keys("F9", true)), Err(()));
        assert_eq!(strip_block(text, Syntax::Conf), Err(()));
    }

    #[test]
    fn finds_who_has_the_key() {
        let free = "bind = SUPER, F9, exec, x\nbind = , F10, exec, y\n# bind = , F9, exec, z\nunbind = , F9\n";
        assert!(matches!(claim(free, Syntax::Conf, "F9"), Claim::Free));
        assert!(matches!(
            claim("bind = , F9, exec, x", Syntax::Conf, "F9"),
            Claim::Other
        ));
        assert!(matches!(claim("bindd=,f9,Do,exec,x", Syntax::Conf, "F9"), Claim::Other));
        assert!(matches!(
            claim(
                "bind = , F9, global, io.github.heast13.kyrr:dictate",
                Syntax::Conf,
                "F9"
            ),
            Claim::Kyrr
        ));
        assert!(matches!(
            claim("hl.bind(\"F9\", hl.dsp.exec_cmd(\"x\"))", Syntax::Lua, "F9"),
            Claim::Other
        ));
        assert!(matches!(
            claim("hl.bind(\"SUPER + F9\", x)", Syntax::Lua, "F9"),
            Claim::Free
        ));
    }

    #[test]
    fn ensure_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("kyrr-hypr-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(ensure(&dir, &keys("F9", true)), Outcome::Failed);

        // A bindings file nothing sources is not Hyprland's.
        std::fs::write(dir.join("hyprland.conf"), "monitor = , preferred\n").unwrap();
        std::fs::write(dir.join("bindings.conf"), "bind = SUPER, Q, killactive").unwrap();
        let main = dir.join("hyprland.conf");
        let file = dir.join("bindings.conf");
        assert_eq!(
            ensure(&dir, &keys("F9", true)),
            Outcome::Bound {
                file: main.clone(),
                written: true
            }
        );
        std::fs::write(&main, "source = ~/.config/hypr/bindings.conf\n").unwrap();

        assert_eq!(
            ensure(&dir, &keys("F9", true)),
            Outcome::Bound {
                file: file.clone(),
                written: true
            }
        );
        assert_eq!(
            ensure(&dir, &keys("F9", true)),
            Outcome::Bound {
                file: file.clone(),
                written: false
            }
        );
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("bind = SUPER, Q, killactive\n\n# >>> kyrr"));
        assert_eq!(std::fs::read_to_string(&main).unwrap().lines().count(), 1);
        assert!(!dir.join("bindings.kyrr-tmp").exists());

        // The key changes; the user's own bind on the new key stays, in
        // whichever file it is.
        std::fs::write(
            &main,
            "source = ~/.config/hypr/bindings.conf\nbind = , Pause, exec, mute\n",
        )
        .unwrap();
        assert_eq!(
            ensure(&dir, &keys("Pause", true)),
            Outcome::Taken { file: main.clone() }
        );
        assert_eq!(
            ensure(&dir, &keys("F8", true)),
            Outcome::Bound {
                file: file.clone(),
                written: true
            }
        );
        assert!(!std::fs::read_to_string(&file).unwrap().contains("F9"));

        // The line an earlier kyrr asked the user to add.
        std::fs::write(&file, "bind = , F8, global, io.github.heast13.kyrr:dictate\n").unwrap();
        assert_eq!(
            ensure(&dir, &keys("F8", true)),
            Outcome::Bound {
                file: file.clone(),
                written: false
            }
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);

        // A block the user cut in half: hands off.
        std::fs::write(&file, "# >>> kyrr: x\nbind = , F8, exec, mute\n").unwrap();
        assert_eq!(ensure(&dir, &keys("F8", true)), Outcome::Failed);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
