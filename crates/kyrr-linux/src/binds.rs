//! Tiling compositors bind keys in their own config, not in a dialog: for
//! the dictation key (and, where the session does not read XDG autostart
//! files, the start at sign-in) a few lines have to be there. kyrr writes
//! them itself, as blocks between two marker lines that it alone rewrites,
//! and leaves everything else in the file as it is. A key that another bind
//! in the user's files already uses is left alone.
//!
//! - Hyprland reloads its config when the file changes. Omarchy keeps its own
//!   key bindings in `bindings.conf`, and newer Hyprland versions read
//!   `hyprland.lua` or `bindings.lua` instead of the `.conf` files when they
//!   exist. Sources: <https://wiki.hypr.land/Configuring/Binds/>.
//! - Sway reads `~/.config/sway/config`, or `/etc/sway/config` when there is
//!   none (kyrr then starts the user's file as a copy of it). It reloads on
//!   `swaymsg reload`. `bindsym --release` runs on the key's release, so the
//!   key can be held. Source: sway(5).
//! - river runs `~/.config/river/init` once at its start: the lines are
//!   `riverctl` calls, which kyrr also runs right away. `map -release` makes
//!   holding work. Source: riverctl(1).
//! - niri reloads `~/.config/niri/config.kdl` when it changes. It has no
//!   release binds, so the key starts and ends dictation with a press each.
//!   The key's lines go inside the file's `binds { }`. Source: niri's wiki,
//!   "Configuration: Key Bindings".
//!
//! Only the standard library is used here.

use std::path::{Path, PathBuf};

const APP_ID: &str = "io.github.heast13.kyrr";
/// The shortcut's name in `bind = , F9, global, <APP_ID>:dictate`.
const GLOBAL: &str = "io.github.heast13.kyrr:dictate";
/// The config sway reads when the user has none.
const SWAY_SYSTEM: &str = "/etc/sway/config";

/// The compositors whose config kyrr writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compositor {
    Hyprland,
    Sway,
    River,
    Niri,
}

impl Compositor {
    /// The compositor of this session, from what it puts in the environment.
    pub fn detect() -> Option<Compositor> {
        let has = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let desktop = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .collect::<Vec<_>>()
            .join(":")
            .to_ascii_lowercase();
        let named = |name: &str| desktop.split(':').any(|d| d.contains(name));
        if has("HYPRLAND_INSTANCE_SIGNATURE") || named("hyprland") {
            Some(Compositor::Hyprland)
        } else if has("NIRI_SOCKET") || named("niri") {
            Some(Compositor::Niri)
        } else if has("SWAYSOCK") || named("sway") {
            Some(Compositor::Sway)
        } else if named("river") {
            Some(Compositor::River)
        } else {
            None
        }
    }

    /// Its name, for the user to read.
    pub fn name(self) -> &'static str {
        match self {
            Compositor::Hyprland => "Hyprland",
            Compositor::Sway => "Sway",
            Compositor::River => "river",
            Compositor::Niri => "niri",
        }
    }

    /// Whether a key can be held: a bind for the press and one for the
    /// release. niri only knows presses.
    pub fn holds(self) -> bool {
        self != Compositor::Niri
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// Hyprland's `.conf`.
    Conf,
    /// Hyprland's `.lua`.
    Lua,
    Sway,
    /// river's init, a shell script.
    River,
    /// niri's KDL.
    Niri,
}

impl Syntax {
    fn comment(self) -> &'static str {
        match self {
            Syntax::Conf | Syntax::Sway | Syntax::River => "#",
            Syntax::Lua => "--",
            Syntax::Niri => "//",
        }
    }
}

/// Where the dictation key goes, and how it reaches kyrr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// The key's name, e.g. "F9".
    pub key: String,
    /// Through the GlobalShortcuts portal (Hyprland only); otherwise the key
    /// runs kyrr with `--dictate-start` on press and `--dictate-stop` on
    /// release, or `--dictate` on each press where it cannot be held.
    pub portal: bool,
    /// How kyrr is started: the program and the arguments before the flag
    /// (`flatpak run <id>`).
    pub program: Vec<String>,
}

impl Keys {
    /// kyrr with `flag`, as a shell reads it.
    pub fn command(&self, flag: &str) -> String {
        command(&self.program, flag)
    }
}

/// `program` with `flag`, as a shell reads it: Hyprland, sway, river and
/// GNOME hand their commands to `sh -c` (GNOME to its own shell-like
/// parser). Words a shell would split or expand are quoted.
pub fn command(program: &[String], flag: &str) -> String {
    let safe = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:=@%,".contains(c));
    program
        .iter()
        .map(String::as_str)
        .chain([flag])
        .map(|w| if safe(w) { w.to_string() } else { sh_quoted(w) })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How setting the key up went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The lines are in the file. `written` when they were not before, so
    /// the compositor has to read the file again.
    Bound { file: PathBuf, written: bool },
    /// Another bind of the user's already uses the key.
    Taken {
        file: PathBuf,
        /// The line (from 1) of that bind, and the bind as written there.
        line: usize,
        bind: String,
    },
    /// The file cannot be found or written (Flatpak without access, a config
    /// kept read-only): the lines have to be added by hand.
    Failed,
}

/// A config file that holds key bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub syntax: Syntax,
}

/// The user's config folder (`~/.config`). In a Flatpak `XDG_CONFIG_HOME` is
/// the app's own folder, the user's lies at `HOST_XDG_CONFIG_HOME`.
pub fn config_home() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = if std::env::var_os("FLATPAK_ID").is_some() {
        var("HOST_XDG_CONFIG_HOME")
    } else {
        var("XDG_CONFIG_HOME")
    };
    base.or_else(|| var("HOME").map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

/// The files the compositor reads its binds from, below the config folder
/// `base`, the one kyrr writes to first.
fn files(compositor: Compositor, base: &Path) -> Vec<Target> {
    match compositor {
        Compositor::Hyprland => hyprland_files(&base.join("hypr")),
        Compositor::Sway => {
            // sway also reads ~/.sway/config (before this one) and the i3
            // configs (after it): a new file would hide or miss those.
            let path = base.join("sway/config");
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|_| base == config_home());
            let elsewhere = [base.join("i3/config")]
                .into_iter()
                .chain(home.iter().flat_map(|h| [h.join(".sway/config"), h.join(".i3/config")]))
                .any(|p| p.is_file());
            if elsewhere && !path.is_file() {
                return Vec::new();
            }
            vec![Target {
                path,
                syntax: Syntax::Sway,
            }]
        }
        Compositor::River => vec![Target {
            path: base.join("river/init"),
            syntax: Syntax::River,
        }],
        Compositor::Niri => {
            // niri --config, and the start scripts, set this.
            let path = std::env::var_os("NIRI_CONFIG")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute() && base == config_home())
                .unwrap_or_else(|| base.join("niri/config.kdl"));
            vec![Target {
                path,
                syntax: Syntax::Niri,
            }]
        }
    }
    .into_iter()
    .filter(|t| t.path.is_file() || (t.syntax == Syntax::Sway && Path::new(SWAY_SYSTEM).is_file()))
    .collect()
}

/// Hyprland reads `hyprland.lua` instead of `hyprland.conf` when it exists. A
/// separate bindings file (Omarchy's `bindings.conf`) counts when the main
/// file mentions it, and stays out of it otherwise.
fn hyprland_files(dir: &Path) -> Vec<Target> {
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

/// The file the key's lines go to, `None` when the compositor has no config
/// there.
pub fn target(compositor: Compositor, base: &Path) -> Option<Target> {
    files(compositor, base).into_iter().next()
}

/// The file kyrr would write to, also when there is none, for the user to
/// read.
pub fn target_or_default(compositor: Compositor, base: &Path) -> Target {
    target(compositor, base).unwrap_or_else(|| match compositor {
        Compositor::Hyprland => Target {
            path: base.join("hypr/hyprland.conf"),
            syntax: Syntax::Conf,
        },
        Compositor::Sway => Target {
            path: base.join("sway/config"),
            syntax: Syntax::Sway,
        },
        Compositor::River => Target {
            path: base.join("river/init"),
            syntax: Syntax::River,
        },
        Compositor::Niri => Target {
            path: base.join("niri/config.kdl"),
            syntax: Syntax::Niri,
        },
    })
}

/// The lines that bind the key.
pub fn lines(syntax: Syntax, keys: &Keys) -> String {
    let key = &keys.key;
    let (start, stop) = (keys.command("--dictate-start"), keys.command("--dictate-stop"));
    // Omarchy puts its own tools on some keys; unbinding frees the key.
    match (syntax, keys.portal) {
        (Syntax::Conf, true) => format!("unbind = , {key}\nbind = , {key}, global, {GLOBAL}"),
        (Syntax::Conf, false) => format!("unbind = , {key}\nbind = , {key}, exec, {start}\nbindr = , {key}, exec, {stop}"),
        (Syntax::Lua, true) => format!("hl.unbind(\"{key}\")\nhl.bind(\"{key}\", hl.dsp.global(\"{GLOBAL}\"))"),
        (Syntax::Lua, false) => format!(
            "hl.unbind(\"{key}\")\nhl.bind(\"{key}\", hl.dsp.exec_cmd(\"{}\"))\nhl.bind(\"{key}\", hl.dsp.exec_cmd(\"{}\"), {{ release = true }})",
            quoted(&start),
            quoted(&stop)
        ),
        (Syntax::Sway, _) => format!("bindsym --no-repeat {key} exec {start}\nbindsym --release {key} exec {stop}"),
        (Syntax::River, _) => format!(
            "riverctl map normal None {key} spawn {}\nriverctl map -release normal None {key} spawn {}",
            sh_quoted(&start),
            sh_quoted(&stop)
        ),
        (Syntax::Niri, _) => {
            let mut words = keys.program.clone();
            words.push("--dictate".into());
            let words: Vec<String> = words.iter().map(|w| format!("\"{}\"", quoted(w))).collect();
            format!("    {key} repeat=false {{ spawn {}; }}", words.join(" "))
        }
    }
}

/// The lines that start kyrr with the session.
fn start_lines(syntax: Syntax, program: &[String]) -> String {
    let command = command(program, "--background");
    match syntax {
        Syntax::Conf => format!("exec-once = {command}"),
        Syntax::Lua => format!(
            "hl.on(\"hyprland.start\", function() hl.exec_cmd(\"{}\") end)",
            quoted(&command)
        ),
        Syntax::Sway => format!("exec {command}"),
        Syntax::River => format!("riverctl spawn {}", sh_quoted(&command)),
        Syntax::Niri => {
            let mut words: Vec<String> = program.iter().map(|w| format!("\"{}\"", quoted(w))).collect();
            words.push("\"--background\"".into());
            format!("spawn-at-startup {}", words.join(" "))
        }
    }
}

/// `s` for inside a double-quoted Lua or KDL string.
fn quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `s` as one word for a shell.
fn sh_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The path with the home folder as `~`, for the user to read.
pub fn display(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The key kyrr's block in the compositor's config binds now, to free it
/// when the key changes (river keeps binds until they are unmapped).
pub fn bound_key(compositor: Compositor, base: &Path) -> Option<String> {
    let target = target(compositor, base)?;
    let text = std::fs::read_to_string(&target.path).ok()?;
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    let (a, b) = find_block(&rows, target.syntax, Block::Key).ok()??;
    rows[a + 1..b].iter().find_map(|row| key_of(row, target.syntax))
}

/// Puts the key's lines into the compositor's config below `base`, or brings
/// the ones kyrr put there before up to date.
pub fn ensure(compositor: Compositor, base: &Path, keys: &Keys) -> Outcome {
    let files = files(compositor, base);
    // The file that holds kyrr's block already (Omarchy's bindings file may
    // have come later), else the first.
    let holds = |t: &&Target| {
        read(t).is_some_and(|text| {
            let rows: Vec<&str> = text.split_inclusive('\n').collect();
            matches!(find_block(&rows, t.syntax, Block::Key), Ok(Some(_)))
        })
    };
    let Some(target) = files.iter().find(holds).or(files.first()) else {
        return Outcome::Failed;
    };
    let mut taken = None;
    for file in &files {
        let Some(content) = read(file) else {
            return Outcome::Failed;
        };
        let Ok(claim) = claim(&content, file.syntax, &keys.key) else {
            return Outcome::Failed;
        };
        match claim {
            Claim::Free => {}
            Claim::Kyrr => {
                return Outcome::Bound {
                    file: file.path.clone(),
                    written: false,
                }
            }
            Claim::Other { line, bind } => {
                taken = taken.or(Some(Outcome::Taken {
                    file: file.path.clone(),
                    line,
                    bind,
                }))
            }
        }
    }
    if let Some(taken) = taken {
        return taken;
    }
    let block = lines(target.syntax, keys);
    match put_block(target, Block::Key, Some(&block)) {
        Ok(written) => Outcome::Bound {
            file: target.path.clone(),
            written,
        },
        Err(()) => Outcome::Failed,
    }
}

/// Starts kyrr with the session from the compositor's config (`program`
/// is how kyrr is started), or no longer (`None`). True when the file
/// is as asked.
pub fn set_start(compositor: Compositor, base: &Path, program: Option<&[String]>) -> bool {
    let Some(target) = target(compositor, base) else {
        return program.is_none();
    };
    let lines = program.map(|p| start_lines(target.syntax, p));
    put_block(&target, Block::Start, lines.as_deref()).is_ok()
}

/// Whether kyrr's start at sign-in is in the compositor's config.
pub fn starts(compositor: Compositor, base: &Path) -> bool {
    let Some(target) = target(compositor, base) else {
        return false;
    };
    let Some(text) = read(&target) else {
        return false;
    };
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    matches!(find_block(&rows, target.syntax, Block::Start), Ok(Some(_)))
}

/// The file's text; sway's system config when the user has none yet.
fn read(target: &Target) -> Option<String> {
    match std::fs::read_to_string(&target.path) {
        Ok(text) => Some(text),
        Err(e) if target.syntax == Syntax::Sway && e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::read_to_string(SWAY_SYSTEM).ok()
        }
        Err(_) => None,
    }
}

/// Writes kyrr's block `block` with `lines` into the file, or takes it out
/// (`None`). `Ok(true)` when the file changed.
fn put_block(target: &Target, block: Block, lines: Option<&str>) -> Result<bool, ()> {
    // Dotfile managers link the config: write through the link.
    let path = std::fs::canonicalize(&target.path).unwrap_or_else(|_| target.path.clone());
    let text = read(&Target {
        path: path.clone(),
        syntax: target.syntax,
    })
    .ok_or(())?;
    let new = match lines {
        Some(lines) => with_block(&text, target.syntax, block, lines)?,
        None => {
            let rest = strip_block(&text, target.syntax, block)?;
            (rest != text).then_some(rest)
        }
    };
    let Some(new) = new else {
        return Ok(false);
    };
    match write_over(&path, &new) {
        Ok(()) => Ok(true),
        Err(e) => {
            eprintln!("kyrr: writing {}: {e}", path.display());
            Err(())
        }
    }
}

/// Replaces the file's content in one step, so a crash leaves the old file.
/// A new file (sway's first own config) gets the folder made.
fn write_over(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.kyrr-tmp"));
    let result = std::fs::write(&tmp, text)
        .and_then(|()| match std::fs::metadata(path) {
            Ok(old) => std::fs::set_permissions(&tmp, old.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// kyrr's blocks: the dictation key, and the start at sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    Key,
    Start,
}

impl Block {
    /// What follows `>>> ` in the start line, up to the note.
    fn tag(self) -> &'static str {
        match self {
            Block::Key => "kyrr:",
            Block::Start => "kyrr start:",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Block::Key => "dictation key (kyrr keeps this block up to date)",
            Block::Start => "start at sign-in (kyrr keeps this block up to date)",
        }
    }

    fn end(self) -> &'static str {
        match self {
            Block::Key => "kyrr",
            Block::Start => "kyrr start",
        }
    }
}

/// The lines that open and close the block. A later kyrr may word the note
/// differently, so the start is found by its beginning.
fn markers(syntax: Syntax, block: Block) -> (String, String) {
    let c = syntax.comment();
    (
        format!("{c} >>> {} {}", block.tag(), block.note()),
        format!("{c} <<< {}", block.end()),
    )
}

fn is_start(row: &str, syntax: Syntax, block: Block) -> bool {
    row.trim()
        .starts_with(&format!("{} >>> {}", syntax.comment(), block.tag()))
}

/// Where the block lies in `rows`: the start and end row. `Err` when a start
/// has no end after it: not kyrr's to rewrite, the user touched it.
fn find_block(rows: &[&str], syntax: Syntax, block: Block) -> Result<Option<(usize, usize)>, ()> {
    let (_, end) = markers(syntax, block);
    let Some(a) = rows.iter().position(|r| is_start(r, syntax, block)) else {
        return Ok(None);
    };
    match rows[a..].iter().position(|r| r.trim() == end) {
        Some(b) => Ok(Some((a, a + b))),
        None => Err(()),
    }
}

/// `text` with the block holding `lines`: `None` when it already does.
fn with_block(text: &str, syntax: Syntax, block: Block, lines: &str) -> Result<Option<String>, ()> {
    let (start, end) = markers(syntax, block);
    // niri's key lines sit inside `binds { }`, indented like the rest.
    let indent = if syntax == Syntax::Niri && block == Block::Key {
        "    "
    } else {
        ""
    };
    let new_block = format!("{indent}{start}\n{lines}\n{indent}{end}\n");
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    let new = match find_block(&rows, syntax, block)? {
        Some((a, b)) => format!("{}{new_block}{}", rows[..a].concat(), rows[b + 1..].concat()),
        None => insert(text, syntax, block, &new_block)?,
    };
    Ok((new != text).then_some(new))
}

/// `text` with a new block: at the end, in river's init before a closing
/// `exec` (which would not return), in niri's config inside `binds`.
fn insert(text: &str, syntax: Syntax, block: Block, new_block: &str) -> Result<String, ()> {
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    let at = match (syntax, block) {
        (Syntax::Niri, Block::Key) => {
            // The top-level `binds {`; without one niri would not read a second.
            let a = rows
                .iter()
                .position(|r| !r.starts_with(char::is_whitespace) && r.trim() == "binds {")
                .ok_or(())?;
            return Ok(format!("{}{new_block}{}", rows[..=a].concat(), rows[a + 1..].concat()));
        }
        (Syntax::River, _) => rows
            .iter()
            .rposition(|r| !r.trim().is_empty() && !r.trim().starts_with('#'))
            .filter(|&i| rows[i].trim_start().starts_with("exec ")),
        _ => None,
    };
    if let Some(i) = at {
        return Ok(format!("{}{new_block}\n{}", rows[..i].concat(), rows[i..].concat()));
    }
    let mut new = text.to_string();
    if !new.is_empty() {
        new.push_str(if new.ends_with('\n') { "\n" } else { "\n\n" });
    }
    new.push_str(new_block);
    Ok(new)
}

/// `text` without kyrr's block, and without the empty line kyrr put before
/// it.
fn strip_block(text: &str, syntax: Syntax, block: Block) -> Result<String, ()> {
    let rows: Vec<&str> = text.split_inclusive('\n').collect();
    let Some((mut a, mut b)) = find_block(&rows, syntax, block)? else {
        return Ok(text.to_string());
    };
    let blank = |i: usize| rows.get(i).is_some_and(|r| r.trim().is_empty());
    if b + 1 == rows.len() && a > 0 && blank(a - 1) {
        a -= 1;
    } else if blank(b + 1) {
        b += 1;
    }
    Ok(format!("{}{}", rows[..a].concat(), rows[b + 1..].concat()))
}

#[derive(Debug)]
enum Claim {
    Free,
    /// A bind of kyrr's that was added by hand, as an earlier kyrr asked.
    Kyrr,
    /// Something else is on the key: the first such bind and its line.
    Other {
        line: usize,
        bind: String,
    },
}

/// The bare key (no modifiers) that `row` binds, if it is a bind.
fn key_of(row: &str, syntax: Syntax) -> Option<String> {
    let row = row.trim();
    if row.starts_with(syntax.comment()) {
        return None;
    }
    match syntax {
        Syntax::Conf => {
            // `bind = MODS, KEY, ...`, also bindr, bindd and the others.
            let (name, rest) = row.split_once('=')?;
            let mut fields = rest.split(',').map(str::trim);
            (name.trim().starts_with("bind") && fields.next() == Some(""))
                .then(|| fields.next().map(str::to_string))
                .flatten()
        }
        Syntax::Lua => {
            let (_, rest) = row.split_once("hl.bind(")?;
            let rest = rest.trim_start();
            let q = rest.chars().next().filter(|&c| c == '"' || c == '\'')?;
            rest[1..].split(q).next().map(str::to_string)
        }
        Syntax::Sway => {
            let mut words = row.split_whitespace();
            (words.next() == Some("bindsym"))
                .then(|| words.find(|w| !w.starts_with("--")).map(str::to_string))
                .flatten()
        }
        Syntax::River => {
            let mut words = row.split_whitespace();
            if words.next() != Some("riverctl") || words.next() != Some("map") {
                return None;
            }
            let mut words = words.peekable();
            while let Some(flag) = words.next_if(|w| w.starts_with('-')) {
                if flag == "-layout" {
                    words.next();
                }
            }
            let (mode, mods, key) = (words.next()?, words.next()?, words.next()?);
            (mode == "normal" && mods == "None").then(|| key.to_string())
        }
        Syntax::Niri => {
            if row.starts_with("/-") {
                return None;
            }
            let key = row.split(|c: char| c.is_whitespace() || c == '{').next()?;
            (!key.is_empty() && row.contains('{')).then(|| key.to_string())
        }
    }
}

/// Who has the bare `key` in `text`, kyrr's own block aside. Binds in
/// sway's modes and outside niri's `binds` do not count. `Err` when kyrr's
/// block lost its end (see [`find_block`]).
fn claim(text: &str, syntax: Syntax, key: &str) -> Result<Claim, ()> {
    let rows: Vec<&str> = text.lines().collect();
    let ours = find_block(&rows, syntax, Block::Key)?;
    let mut claim = Claim::Free;
    // The blocks around the row: whether each is one where binds count.
    let mut nesting: Vec<bool> = Vec::new();
    for (i, row) in rows.iter().map(|r| r.trim()).enumerate() {
        if ours.is_some_and(|(a, b)| (a..=b).contains(&i)) {
            continue;
        }
        let counts = match syntax {
            Syntax::Sway => nesting.iter().all(|&c| c),
            Syntax::Niri => nesting == [true],
            _ => true,
        };
        let on_key = counts && key_of(row, syntax).is_some_and(|k| k.eq_ignore_ascii_case(key));
        if matches!(syntax, Syntax::Sway | Syntax::Niri) && !row.starts_with(syntax.comment()) {
            if row.ends_with('{') {
                nesting.push(match syntax {
                    Syntax::Sway => !row.starts_with("mode"),
                    _ => nesting.is_empty() && row == "binds {",
                });
            } else if row.starts_with('}') && !row.contains('{') {
                nesting.pop();
            }
        }
        if !on_key {
            continue;
        }
        if row.contains(APP_ID) || row.contains("--dictate") {
            return Ok(Claim::Kyrr);
        }
        if matches!(claim, Claim::Free) {
            claim = Claim::Other {
                line: i + 1,
                bind: row.to_string(),
            };
        }
    }
    Ok(claim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(key: &str, portal: bool) -> Keys {
        Keys {
            key: key.into(),
            portal,
            program: vec!["kyrr".into()],
        }
    }

    fn block(text: &str, syntax: Syntax, keys: &Keys) -> Option<String> {
        with_block(text, syntax, Block::Key, &lines(syntax, keys)).unwrap()
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kyrr-binds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
        assert_eq!(strip_block(&text, Syntax::Conf, Block::Key), Ok("keep\n".to_string()));
        let river = block("a\nexec b\n", Syntax::River, &keys("F9", false)).unwrap();
        assert_eq!(
            strip_block(&river, Syntax::River, Block::Key),
            Ok("a\nexec b\n".to_string())
        );
    }

    #[test]
    fn a_start_without_an_end_is_left_alone() {
        let text = "a = 1\n# >>> kyrr: mine\nbind = , F9, exec, mute\n";
        assert_eq!(with_block(text, Syntax::Conf, Block::Key, "x"), Err(()));
        assert_eq!(strip_block(text, Syntax::Conf, Block::Key), Err(()));
    }

    #[test]
    fn the_two_blocks_keep_apart() {
        let text = block("a = 1\n", Syntax::Conf, &keys("F9", true)).unwrap();
        let both = with_block(
            &text,
            Syntax::Conf,
            Block::Start,
            &start_lines(Syntax::Conf, &["kyrr".into()]),
        )
        .unwrap()
        .unwrap();
        assert!(both.contains("# >>> kyrr start: start at sign-in"));
        assert!(both.contains("exec-once = kyrr --background\n# <<< kyrr start\n"));
        assert_eq!(block(&both, Syntax::Conf, &keys("F9", true)), None);
        let rows: Vec<&str> = both.split_inclusive('\n').collect();
        let (a, b) = find_block(&rows, Syntax::Conf, Block::Key).unwrap().unwrap();
        assert_eq!(rows[b].trim(), "# <<< kyrr");
        assert!(rows[a].contains("dictation key"));
        assert_eq!(
            strip_block(&both, Syntax::Conf, Block::Start).unwrap().trim_end(),
            text.trim_end()
        );
    }

    #[test]
    fn finds_who_has_the_key() {
        let free = "bind = SUPER, F9, exec, x\nbind = , F10, exec, y\n# bind = , F9, exec, z\nunbind = , F9\n";
        assert!(matches!(claim(free, Syntax::Conf, "F9").unwrap(), Claim::Free));
        assert!(matches!(
            claim("bind = , F9, exec, x", Syntax::Conf, "F9").unwrap(),
            Claim::Other { .. }
        ));
        assert!(matches!(
            claim("bindd=,f9,Do,exec,x", Syntax::Conf, "F9").unwrap(),
            Claim::Other { .. }
        ));
        assert!(matches!(
            claim(
                "bind = , F9, global, io.github.heast13.kyrr:dictate",
                Syntax::Conf,
                "F9"
            )
            .unwrap(),
            Claim::Kyrr
        ));
        assert!(matches!(
            claim("hl.bind(\"F9\", hl.dsp.exec_cmd(\"x\"))", Syntax::Lua, "F9").unwrap(),
            Claim::Other { .. }
        ));
        // Flo's Omarchy test: kyrr's own block on F9, someone else's F8 below.
        let lua = "-- >>> kyrr: dictation key (kyrr keeps this block up to date)\nhl.unbind(\"F9\")\nhl.bind(\"F9\", hl.dsp.global(\"io.github.heast13.kyrr:dictate\"))\n-- <<< kyrr\n\nhl.bind(\"F8\", hl.dsp.exec_cmd(\"true\"))\n";
        match claim(lua, Syntax::Lua, "F8").unwrap() {
            Claim::Other { line, bind } => {
                assert_eq!(line, 6);
                assert_eq!(bind, "hl.bind(\"F8\", hl.dsp.exec_cmd(\"true\"))");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            claim("hl.bind(\"SUPER + F9\", x)", Syntax::Lua, "F9").unwrap(),
            Claim::Free
        ));
    }

    #[test]
    fn sway() {
        let config = "set $mod Mod4\nbindsym $mod+F9 exec x\nmode \"resize\" {\n    bindsym F9 resize grow\n}\n";
        assert!(matches!(claim(config, Syntax::Sway, "F9").unwrap(), Claim::Free));
        let mine = format!("{config}bindsym --to-code F9 exec mute\n");
        assert!(matches!(claim(&mine, Syntax::Sway, "F9").unwrap(), Claim::Other { .. }));
        let new = block(config, Syntax::Sway, &keys("F9", false)).unwrap();
        assert!(new.contains(
            "bindsym --no-repeat F9 exec kyrr --dictate-start\nbindsym --release F9 exec kyrr --dictate-stop\n"
        ));
        // kyrr's own block does not count.
        assert!(matches!(claim(&new, Syntax::Sway, "F9").unwrap(), Claim::Free));
    }

    #[test]
    fn river() {
        let init = "#!/bin/sh\nriverctl map normal Super F9 spawn x\nriverctl map -layout 0 normal None F8 spawn y\nexec rivertile\n";
        assert!(matches!(claim(init, Syntax::River, "F9").unwrap(), Claim::Free));
        assert!(matches!(claim(init, Syntax::River, "F8").unwrap(), Claim::Other { .. }));
        let new = block(init, Syntax::River, &keys("F9", false)).unwrap();
        // Before the exec, which never returns.
        assert!(new.ends_with("# <<< kyrr\n\nexec rivertile\n"));
        assert!(new.contains("riverctl map -release normal None F9 spawn 'kyrr --dictate-stop'\n"));
        assert_eq!(
            key_of(
                "riverctl map normal None F9 spawn 'kyrr --dictate-start'",
                Syntax::River
            )
            .as_deref(),
            Some("F9")
        );
        assert_eq!(sh_quoted("it's"), "'it'\\''s'");
        assert_eq!(
            command(&["/home/u/kyrr.AppImage".into()], "--dictate"),
            "/home/u/kyrr.AppImage --dictate"
        );
        assert_eq!(
            command(&["/home/u/My Apps/kyrr.AppImage".into()], "--dictate"),
            "'/home/u/My Apps/kyrr.AppImage' --dictate"
        );
    }

    #[test]
    fn niri() {
        let config = "input {\n    keyboard {\n    }\n}\n\nbinds {\n    Mod+F9 { spawn \"x\"; }\n    /- F9 { spawn \"y\"; }\n}\n\nwindow-rule {\n    F9 { }\n}\n";
        assert!(matches!(claim(config, Syntax::Niri, "F9").unwrap(), Claim::Free));
        let new = block(config, Syntax::Niri, &keys("F9", false)).unwrap();
        assert!(new.contains(
            "binds {\n    // >>> kyrr: dictation key (kyrr keeps this block up to date)\n    F9 repeat=false { spawn \"kyrr\" \"--dictate\"; }\n    // <<< kyrr\n    Mod+F9"
        ));
        // kyrr's own block does not count.
        assert!(matches!(claim(&new, Syntax::Niri, "F9").unwrap(), Claim::Free));
        let theirs = config.replace("Mod+F9", "F9");
        assert!(matches!(
            claim(&theirs, Syntax::Niri, "F9").unwrap(),
            Claim::Other { .. }
        ));
        assert_eq!(with_block("input {\n}\n", Syntax::Niri, Block::Key, "x"), Err(()));
        let start = start_lines(Syntax::Niri, &["flatpak".into(), "run".into(), APP_ID.into()]);
        assert_eq!(
            start,
            "spawn-at-startup \"flatpak\" \"run\" \"io.github.heast13.kyrr\" \"--background\""
        );
    }

    #[test]
    fn hyprland_in_a_folder() {
        let base = folder("hypr");
        let dir = base.join("hypr");
        std::fs::create_dir_all(&dir).unwrap();
        let hypr = Compositor::Hyprland;
        assert_eq!(ensure(hypr, &base, &keys("F9", true)), Outcome::Failed);

        // A bindings file nothing sources is not Hyprland's.
        std::fs::write(dir.join("hyprland.conf"), "monitor = , preferred\n").unwrap();
        std::fs::write(dir.join("bindings.conf"), "bind = SUPER, Q, killactive").unwrap();
        let main = dir.join("hyprland.conf");
        let file = dir.join("bindings.conf");
        assert_eq!(
            ensure(hypr, &base, &keys("F9", true)),
            Outcome::Bound {
                file: main.clone(),
                written: true
            }
        );
        std::fs::write(&main, "source = ~/.config/hypr/bindings.conf\n").unwrap();

        assert_eq!(
            ensure(hypr, &base, &keys("F9", true)),
            Outcome::Bound {
                file: file.clone(),
                written: true
            }
        );
        assert_eq!(
            ensure(hypr, &base, &keys("F9", true)),
            Outcome::Bound {
                file: file.clone(),
                written: false
            }
        );
        assert_eq!(bound_key(hypr, &base).as_deref(), Some("F9"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("bind = SUPER, Q, killactive\n\n# >>> kyrr"));
        assert_eq!(std::fs::read_to_string(&main).unwrap().lines().count(), 1);
        assert!(!dir.join(".bindings.conf.kyrr-tmp").exists());

        // The start at sign-in comes and goes in a block of its own.
        assert!(!starts(hypr, &base));
        assert!(set_start(hypr, &base, Some(&["kyrr".into()])));
        assert!(starts(hypr, &base));
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("exec-once = kyrr --background\n"));
        assert!(set_start(hypr, &base, None));
        assert!(!starts(hypr, &base));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);

        // The key changes; the user's own bind on the new key stays, in
        // whichever file it is.
        std::fs::write(
            &main,
            "source = ~/.config/hypr/bindings.conf\nbind = , Pause, exec, mute\n",
        )
        .unwrap();
        assert_eq!(
            ensure(hypr, &base, &keys("Pause", true)),
            Outcome::Taken {
                file: main.clone(),
                line: 2,
                bind: "bind = , Pause, exec, mute".into()
            }
        );
        assert_eq!(
            ensure(hypr, &base, &keys("F8", true)),
            Outcome::Bound {
                file: file.clone(),
                written: true
            }
        );
        assert!(!std::fs::read_to_string(&file).unwrap().contains("F9"));

        // The line an earlier kyrr asked the user to add.
        std::fs::write(&file, "bind = , F8, global, io.github.heast13.kyrr:dictate\n").unwrap();
        assert_eq!(
            ensure(hypr, &base, &keys("F8", true)),
            Outcome::Bound {
                file: file.clone(),
                written: false
            }
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);

        // A block the user cut in half: hands off.
        std::fs::write(&file, "# >>> kyrr: x\nbind = , F8, exec, mute\n").unwrap();
        assert_eq!(ensure(hypr, &base, &keys("F8", true)), Outcome::Failed);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn river_keeps_its_init_runnable() {
        use std::os::unix::fs::PermissionsExt;
        let base = folder("river");
        let init = base.join("river/init");
        std::fs::create_dir_all(init.parent().unwrap()).unwrap();
        std::fs::write(&init, "#!/bin/sh\nriverctl spawn foot\n").unwrap();
        std::fs::set_permissions(&init, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            ensure(Compositor::River, &base, &keys("F9", false)),
            Outcome::Bound { written: true, .. }
        ));
        assert_eq!(std::fs::metadata(&init).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(bound_key(Compositor::River, &base).as_deref(), Some("F9"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
