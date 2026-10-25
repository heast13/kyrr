//! Omarchy (Basecamp's Arch setup around Hyprland) in its own terms: kyrr as
//! one of its dictation programs, and its theme as kyrr's look.
//!
//! - Dictation (Omarchy 4.5 on): a program `<name>` joins by putting
//!   `omarchy-dictation-<name>` (`start`, `stop`, `toggle`; quick, and `stop`
//!   fine when nothing runs) and `omarchy-install-dictation-<name>` on the
//!   PATH. The second ends in `omarchy-dictation-use <name>`, which keeps the
//!   choice in `~/.config/omarchy/defaults/dictation`. Omarchy's keys (F9 and
//!   Right Alt held, Super+Ctrl+X) then run `omarchy dictation start`, `stop`
//!   and `toggle`, but only while the program `<name>` itself is on the PATH.
//!   The menu reads extra rows from
//!   `~/.config/omarchy/extensions/omarchy-menu.jsonc`: JSON with whole-line
//!   `//` comments and trailing commas. Source: Omarchy's manual, "Text
//!   Extraction & Dictation", and its `docs/menu.md`.
//! - Theme: the palette of the theme in use is `colors.toml` in
//!   `~/.local/state/omarchy/current/theme` (Omarchy 3:
//!   `~/.config/omarchy/current/theme`), `key = "#rrggbb"` lines and
//!   `mode = "dark"` or `"light"` (older themes: a file `light.mode`).
//!   Omarchy draws everything with square corners, 2 px borders in the window
//!   border's colour and the system's monospace font. Source: Omarchy's
//!   `docs/theming.md` and `default/themed/shell.toml.tpl`.
//! - Bar widget (its own repository, `heast13/omarchy-kyrr`): it reads what
//!   kyrr does from a file kyrr keeps, flips kyrr's switches over kyrr's
//!   socket and renews a file of its own while it sits in the bar.
//!
//! Only the standard library is used here.

use std::path::{Path, PathBuf};

use crate::binds::{config_home, sh_quoted, write_over};

/// The name kyrr has among Omarchy's dictation programs.
const NAME: &str = "kyrr";
/// In each script kyrr writes, so it knows its own.
const MARK: &str = "kyrr writes this file and keeps it up to";
const MENU_START: &str = "// >>> kyrr (kyrr keeps this block up to date)";
const MENU_END: &str = "// <<< kyrr";

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Where Omarchy is installed: the package's folder, or a git checkout
/// (Omarchy 3).
pub fn path() -> Option<PathBuf> {
    let from_env = std::env::var_os("OMARCHY_PATH").map(PathBuf::from);
    let checkout = home().map(|h| h.join(".local/share/omarchy"));
    [from_env, Some(PathBuf::from("/usr/share/omarchy")), checkout]
        .into_iter()
        .flatten()
        .find(|p| p.join("bin").is_dir())
}

/// Whether this Omarchy takes other dictation programs (4.5 on).
pub fn takes_dictation() -> bool {
    path().is_some_and(|p| p.join("bin/omarchy-dictation-use").is_file())
}

/// Whether `program` is a command of the session. The AppImage puts its own
/// folder first in kyrr's PATH; that one does not count.
fn on_path(program: &str) -> bool {
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .filter(|dir| appdir.as_ref().is_none_or(|a| !dir.starts_with(a)))
            .any(|dir| dir.join(program).is_file())
    })
}

/// The dictation program Omarchy has chosen, if any.
fn chosen(config: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config.join("omarchy/defaults/dictation")).ok()?;
    let name = text.lines().next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Puts kyrr among Omarchy's dictation programs, or brings its files up to
/// date. `program` starts kyrr (the AppImage's path, say). With `choose`,
/// kyrr also becomes the one Omarchy's keys use, but only where those keys
/// do nothing now: no program chosen, or one that is not installed (fresh
/// installs choose Superwhisper without installing it). Someone who set up
/// Voxtype or Superwhisper keeps it and can pick kyrr in the menu. True
/// when kyrr was chosen now.
pub fn join_dictation(program: &str, german: bool, choose: bool) -> bool {
    let (Some(omarchy), Some(home)) = (path(), home()) else {
        return false;
    };
    let bin = home.join(".local/bin");
    let config = config_home();
    for (name, text) in [
        (format!("omarchy-dictation-{NAME}"), adapter(program)),
        (format!("omarchy-install-dictation-{NAME}"), installer(program, german)),
    ] {
        if let Err(e) = write_script(&bin.join(&name), &text) {
            eprintln!("kyrr: writing {name}: {e}");
            return false;
        }
    }
    // Omarchy binds its keys only while `kyrr` itself is a command. An
    // AppImage kept elsewhere gets a link in ~/.local/bin (on the session's
    // PATH in Omarchy); a file of the user's there stays.
    let link = bin.join(NAME);
    // No file there yet, or kyrr's own link: to a kyrr file, or to one
    // that has gone (the AppImage moved).
    let ours = match std::fs::symlink_metadata(&link) {
        Err(_) => true,
        Ok(m) if m.file_type().is_symlink() => std::fs::read_link(&link).is_ok_and(|to| {
            !to.exists()
                || to
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with(NAME))
        }),
        Ok(_) => false,
    };
    if Path::new(program).is_absolute() && Path::new(program) != link && ours && !on_path(NAME) {
        let _ = std::fs::remove_file(&link);
        if let Err(e) = std::os::unix::fs::symlink(program, &link) {
            eprintln!("kyrr: linking {}: {e}", link.display());
        }
    }
    // A dotfile kept elsewhere and linked here: the file itself.
    let menu = config.join("omarchy/extensions/omarchy-menu.jsonc");
    let menu = std::fs::canonicalize(&menu).unwrap_or(menu);
    let old = std::fs::read_to_string(&menu).ok();
    if let Some(new) = menu_with_kyrr(old.as_deref()) {
        if let Err(e) = write_over(&menu, &new) {
            eprintln!("kyrr: writing {}: {e}", menu.display());
        }
    }
    let idle = match chosen(&config) {
        None => true,
        Some(name) => name != NAME && !on_path(&name),
    };
    if !choose || !idle {
        return false;
    }
    let bin_path = std::env::var_os("PATH").map_or_else(
        || omarchy.join("bin").into_os_string(),
        |path| {
            let mut all = omarchy.join("bin").into_os_string();
            all.push(":");
            all.push(path);
            all
        },
    );
    let chose = std::process::Command::new(omarchy.join("bin/omarchy-dictation-use"))
        .arg(NAME)
        .env("PATH", &bin_path)
        .env("OMARCHY_PATH", &omarchy)
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if chose {
        // Omarchy's keys are bound when Hyprland reads its config.
        let _ = std::process::Command::new("hyprctl")
            .arg("reload")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    chose
}

/// Writes an executable file when it is new or changed.
fn write_script(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let old = std::fs::read_to_string(path).ok();
    // A file of that name that kyrr did not write stays.
    if old.as_deref().is_some_and(|old| !old.contains(MARK)) {
        return Err(std::io::Error::other("a file of the user's is in the way"));
    }
    let same = old.is_some_and(|old| old == text);
    let executable = std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
    if same && executable {
        return Ok(());
    }
    write_over(path, text)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

/// `omarchy-dictation-kyrr`: one word to the running kyrr over its socket
/// (see `instance`), quicker than starting the AppImage on each press. When
/// kyrr does not run, `stop` has nothing to do and the rest starts it.
fn adapter(program: &str) -> String {
    format!(
        r#"#!/bin/bash
# Omarchy's dictation keys to kyrr. kyrr writes this file and keeps it up to
# date; delete it together with kyrr.

case "${{1:-}}" in
  start) request=dictate-start ;;
  stop) request=dictate-stop ;;
  toggle) request=dictate ;;
  *) echo "Usage: omarchy-dictation-kyrr <start|stop|toggle>" >&2; exit 1 ;;
esac

if [[ -n ${{XDG_RUNTIME_DIR:-}} ]]; then
  socket="$XDG_RUNTIME_DIR/kyrr.sock"
else
  socket="${{TMPDIR:-/tmp}}/kyrr-$(id -u).sock"
fi
# socat comes with Omarchy; without it kyrr passes the word on itself.
if command -v socat >/dev/null; then
  if [[ -S $socket ]] && printf %s "$request" | socat -u - "UNIX-CONNECT:$socket" 2>/dev/null; then
    exit 0
  fi
  # Nobody answers: kyrr does not run, and there is nothing to stop.
  [[ $request == dictate-stop ]] && exit 0
elif [[ $request == dictate-stop && ! -S $socket ]]; then
  exit 0
fi
setsid -f {program} "--$request" >/dev/null 2>&1 </dev/null
"#,
        program = sh_quoted(program)
    )
}

/// `omarchy-install-dictation-kyrr`, which Omarchy's menu runs (Setup >
/// Defaults > Dictation > kyrr) in a small terminal.
fn installer(program: &str, german: bool) -> String {
    let (title, body) = if german {
        (
            "kyrr diktiert jetzt",
            "F9 halten zum Diktieren, oder Super+Strg+X zum Ein- und Ausschalten.",
        )
    } else {
        (
            "kyrr dictates now",
            "Hold F9 to dictate, or press Super+Ctrl+X to start and stop.",
        )
    };
    format!(
        r#"#!/bin/bash
# Makes kyrr the dictation program for Omarchy's keys (Setup > Defaults >
# Dictation); kyrr writes this file and keeps it up to date.

set -e
omarchy-dictation-use kyrr
hyprctl reload >/dev/null || true
pgrep -x kyrr >/dev/null || setsid -f {program} --background >/dev/null 2>&1 </dev/null
notify-send -a kyrr {title} {body} || true
"#,
        program = sh_quoted(program),
        title = sh_quoted(title),
        body = sh_quoted(body),
    )
}

/// Omarchy's menu file with kyrr's row under Setup > Defaults > Dictation;
/// None when it is there already or the file is not one kyrr understands.
/// The block goes right after the opening brace: its row ends in a comma,
/// which Omarchy allows before the closing brace, while a last row of the
/// user's may have none.
fn menu_with_kyrr(old: Option<&str>) -> Option<String> {
    let row = concat!(
        r#"  "setup.default.dictation.kyrr": {"icon":"󰍬","label":"kyrr","#,
        r#""when":"omarchy-cmd-present omarchy-dictation-kyrr","#,
        r#""checked":"[[ \"$(omarchy-default-dictation)\" == \"kyrr\" ]]","#,
        r#""action":"omarchy-default-dictation kyrr"},"#
    );
    let block = format!("  {MENU_START}\n{row}\n  {MENU_END}\n");
    let Some(old) = old.filter(|t| !t.trim().is_empty()) else {
        return Some(format!("{{\n{block}}}\n"));
    };
    let mut kept = String::with_capacity(old.len() + block.len());
    let mut inside = false;
    let mut found = false;
    for line in old.split_inclusive('\n') {
        match line.trim() {
            MENU_START => {
                inside = true;
                found = true;
                kept.push_str(&block);
            }
            MENU_END if inside => inside = false,
            _ if inside => {}
            _ => kept.push_str(line),
        }
    }
    if inside {
        // A start without an end: someone edited it; leave the file be.
        return None;
    }
    if !found {
        // The first brace that is not in a comment line.
        let mut at = 0;
        let mut brace = None;
        for line in old.split_inclusive('\n') {
            if !line.trim_start().starts_with("//") {
                if let Some(i) = line.find('{') {
                    brace = Some(at + i);
                    break;
                }
            }
            at += line.len();
        }
        let brace = brace?;
        let rest = &old[brace + 1..];
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        kept = format!("{}\n{block}{rest}", &old[..=brace]);
    }
    (kept != old).then_some(kept)
}

/// The file kyrr's widget in Omarchy's bar reads while kyrr runs: one line
/// of JSON with what kyrr does, its switches and its process id.
fn state_file() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    Some(dir.join("kyrr/state"))
}

struct Shown {
    mode: &'static str,
    /// Everything else the widget shows, a JSON object kyrr's app makes.
    status: Option<String>,
    written: String,
}

static SHOWN: std::sync::Mutex<Shown> = std::sync::Mutex::new(Shown {
    mode: "idle",
    status: None,
    written: String::new(),
});

/// Tells the bar widget what kyrr does: `idle`, `listening`, `writing` or
/// `recording` (a voice memo).
pub fn set_state(mode: &'static str) {
    show(|shown| shown.mode = mode);
}

/// Tells the bar widget kyrr's settings and what they show: a JSON object,
/// described in the widget's README ("State").
pub fn set_status(status: String) {
    show(|shown| shown.status = Some(status));
}

/// Only on Omarchy, and only a change.
fn show(change: impl FnOnce(&mut Shown)) {
    if path().is_none() {
        return;
    }
    let mut shown = SHOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    change(&mut shown);
    let Some(status) = &shown.status else {
        return;
    };
    let text = state_json(shown.mode, status, std::process::id());
    if text == shown.written {
        return;
    }
    let Some(file) = state_file() else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // In one write, in place: the widget watches the file itself. kyrr's
    // process id lets it see a kyrr that ended without deleting the file.
    let _ = std::fs::write(&file, &text);
    shown.written = text;
}

/// The status object with the format's version, kyrr's process id and what
/// it does in front, on one line.
fn state_json(mode: &str, status: &str, pid: u32) -> String {
    let rest = status.trim().strip_prefix('{').unwrap_or("}").trim_start();
    let comma = if rest.starts_with('}') { "" } else { "," };
    format!("{{\"v\":2,\"pid\":{pid},\"mode\":\"{mode}\"{comma}{rest}\n")
}

/// kyrr ends: the widget shows it as not running.
pub fn clear_state() {
    if let Some(file) = state_file() {
        let _ = std::fs::remove_file(&file);
        if let Some(dir) = file.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// The id of kyrr's widget among Omarchy's plugins.
const PLUGIN: &str = "heast13.kyrr";

/// Whether kyrr's widget is in Omarchy's bar for good: installed, and
/// turned on, which is its id in the bar's layout in `shell.json` (turning
/// a widget off takes it out of the layout). Then the widget's menu is all
/// of kyrr: no window, no tray icon.
pub fn plugin_on() -> bool {
    let omarchy = config_home().join("omarchy");
    path().is_some()
        && omarchy.join("plugins").join(PLUGIN).join("manifest.json").is_file()
        && std::fs::read_to_string(omarchy.join("shell.json")).is_ok_and(|text| plugin_in(&text))
}

fn plugin_in(shell_json: &str) -> bool {
    shell_json.contains(&format!("\"{PLUGIN}\""))
}

/// Opens the widget's menu, where kyrr's window would come up elsewhere.
pub fn open_menu() {
    let program = path()
        .map(|p| p.join("bin/omarchy-shell"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("omarchy-shell"));
    let _ = std::process::Command::new(program)
        .args(["-q", PLUGIN, "open"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| std::thread::spawn(move || child.wait()));
}

/// Whether kyrr's widget sits in Omarchy's bar: while it does, it renews
/// this file every few seconds. Then the widget's menu is kyrr's tray menu.
pub fn widget_shown() -> bool {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return false;
    };
    std::fs::metadata(dir.join("kyrr-widget"))
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < std::time::Duration::from_secs(30)))
}

/// A colour as red, green and blue, 0..255.
pub type Rgb = [u8; 3];

/// What kyrr takes from the Omarchy theme in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub dark: bool,
    pub background: Rgb,
    pub foreground: Rgb,
    pub accent: Rgb,
    /// The window border's colour (the first of a gradient): Omarchy's
    /// popups take it for their frame.
    pub border: Rgb,
    pub red: Rgb,
    pub yellow: Rgb,
    pub green: Rgb,
}

/// The folder of the theme in use.
fn theme_dir() -> Option<PathBuf> {
    let home = home()?;
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"));
    [
        state.join("omarchy/current/theme"),
        config_home().join("omarchy/current/theme"),
    ]
    .into_iter()
    .find(|d| d.join("colors.toml").is_file())
}

/// The Omarchy theme in use; None outside Omarchy.
pub fn theme() -> Option<Theme> {
    let dir = theme_dir()?;
    let text = std::fs::read_to_string(dir.join("colors.toml")).ok()?;
    parse_theme(&text, dir.join("light.mode").exists())
}

/// A theme from the text of its `colors.toml`; `light_file` when its folder
/// has a file `light.mode`.
pub fn parse_theme(text: &str, light_file: bool) -> Option<Theme> {
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            if k.trim() != key {
                return None;
            }
            // `"#7aa2f7"`, maybe with a comment after it.
            let v = v.trim();
            Some(match v.strip_prefix('"').and_then(|rest| rest.split_once('"')) {
                Some((inner, _)) => inner.to_string(),
                None => v.to_string(),
            })
        })
    };
    let color = |key: &str| value(key).as_deref().and_then(hex);
    let background = color("background").or_else(|| color("bg"))?;
    let foreground = color("foreground").or_else(|| color("fg"))?;
    let accent = color("accent").or_else(|| color("color4")).unwrap_or(foreground);
    let border = value("hyprland_active_border")
        .as_deref()
        .and_then(first_color)
        .unwrap_or(accent);
    let dark = match value("mode").as_deref() {
        Some("light") => false,
        Some("dark") => true,
        _ => !light_file,
    };
    Some(Theme {
        dark,
        background,
        foreground,
        accent,
        border,
        red: color("red").or_else(|| color("color1")).unwrap_or([0xe5, 0x48, 0x4d]),
        yellow: color("yellow")
            .or_else(|| color("color3"))
            .unwrap_or([0xd2, 0x99, 0x22]),
        green: color("green").or_else(|| color("color2")).unwrap_or([0x2e, 0xa0, 0x43]),
    })
}

/// `#rrggbb` (or `rrggbb`).
fn hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 || !s.is_char_boundary(6) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The first colour of a Hyprland border: `rgba(33ccffee) rgba(00ff99ee)
/// 45deg`, `rgb(...)`, `0xee33ccff` or `#33ccff`.
fn first_color(s: &str) -> Option<Rgb> {
    let word = s.split_whitespace().next()?;
    if let Some(inner) = word.strip_prefix("rgba(").or_else(|| word.strip_prefix("rgb(")) {
        return hex(inner.trim_end_matches(')'));
    }
    if let Some(argb) = word.strip_prefix("0x") {
        return argb.get(2..).and_then(hex);
    }
    hex(word)
}

/// The family and file of the system's monospace font, which Omarchy uses
/// everywhere (JetBrainsMono Nerd Font unless the user picked another).
pub fn mono_font() -> Option<(String, PathBuf)> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{family[0]}\n%{file}", "monospace"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let (family, file) = text.split_once('\n')?;
    let file = PathBuf::from(file.trim());
    (!family.trim().is_empty() && file.is_file()).then(|| (family.trim().to_string(), file))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO: &str = "mode = \"dark\"\n\naccent = \"#7aa2f7\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"\nred = \"#f7768e\"\nyellow = \"#e0af68\"\ngreen = \"#9ece6a\"\n";

    #[test]
    fn state_for_the_widget() {
        assert_eq!(
            state_json("listening", "{\"lang\":\"de\",\"filter\":{\"on\":true}}", 42),
            "{\"v\":2,\"pid\":42,\"mode\":\"listening\",\"lang\":\"de\",\"filter\":{\"on\":true}}\n"
        );
        assert_eq!(state_json("idle", "{}", 7), "{\"v\":2,\"pid\":7,\"mode\":\"idle\"}\n");
    }

    #[test]
    fn plugin_turned_on() {
        assert!(plugin_in("{\"bar\":{\"right\":[{\"id\":\"heast13.kyrr\"}]}}"));
        assert!(!plugin_in("{\"bar\":{\"right\":[{\"id\":\"heast13.kyrr-old\"}]}}"));
    }

    #[test]
    fn reads_a_theme() {
        let theme = parse_theme(TOKYO, false).unwrap();
        assert!(theme.dark);
        assert_eq!(theme.accent, [0x7a, 0xa2, 0xf7]);
        assert_eq!(theme.border, theme.accent);
        assert_eq!(theme.background, [0x1a, 0x1b, 0x26]);
        assert_eq!(theme.red, [0xf7, 0x76, 0x8e]);
    }

    #[test]
    fn light_themes_and_borders() {
        let light = parse_theme("background = \"#ffffff\"\nforeground = \"#000000\"\n", true).unwrap();
        assert!(!light.dark);
        assert_eq!(light.accent, light.foreground);
        let gradient = format!("{TOKYO}hyprland_active_border = \"rgba(33ccffee) rgba(00ff99ee) 45deg\"\n");
        assert_eq!(parse_theme(&gradient, false).unwrap().border, [0x33, 0xcc, 0xff]);
        assert_eq!(first_color("0xee33ccff"), Some([0x33, 0xcc, 0xff]));
        assert_eq!(parse_theme("accent = \"#123456\"\n", false), None);
    }

    #[test]
    fn menu_row_goes_after_the_brace() {
        let sample = "{\n  // Extend the menu.\n  // \"personal\": {\"label\":\"x\"},\n}\n";
        let new = menu_with_kyrr(Some(sample)).unwrap();
        assert!(new.starts_with("{\n  // >>> kyrr"));
        assert!(new.ends_with("  // \"personal\": {\"label\":\"x\"},\n}\n"));
        // Once there, nothing changes; a changed row is replaced in place.
        assert_eq!(menu_with_kyrr(Some(&new)), None);
        let stale = new.replace("\"label\":\"kyrr\"", "\"label\":\"old\"");
        assert_eq!(menu_with_kyrr(Some(&stale)).as_deref(), Some(new.as_str()));
    }

    #[test]
    fn menu_file_new_and_odd() {
        let new = menu_with_kyrr(None).unwrap();
        assert!(new.starts_with("{\n") && new.ends_with("}\n"));
        assert!(new.contains("\"action\":\"omarchy-default-dictation kyrr\"},\n"));
        // A comment above the object, a row of the user's without a comma.
        let user = "// mine\n{ \"about\": {\"label\":\"About\"}\n}\n";
        let with = menu_with_kyrr(Some(user)).unwrap();
        assert!(with.starts_with("// mine\n{\n  // >>> kyrr"));
        assert!(with.ends_with("  // <<< kyrr\n \"about\": {\"label\":\"About\"}\n}\n"));
        assert_eq!(menu_with_kyrr(Some("not json")), None);
        assert_eq!(menu_with_kyrr(Some(&format!("{{\n  {MENU_START}\n}}\n"))), None);
    }

    #[test]
    fn scripts_quote_the_program() {
        let a = adapter("/home/flo/Apps/k'yrr.AppImage");
        assert!(a.contains("setsid -f '/home/flo/Apps/k'\\''yrr.AppImage' \"--$request\""));
        assert!(a.contains("start) request=dictate-start ;;"));
        let i = installer("/x/kyrr", true);
        assert!(i.contains("omarchy-dictation-use kyrr\n"));
        assert!(i.contains("'kyrr diktiert jetzt'"));
    }

    #[test]
    fn scripts_are_marked_and_spare_other_files() {
        assert!(adapter("kyrr").contains(MARK) && installer("kyrr", false).contains(MARK));
        let dir = std::env::temp_dir().join(format!("kyrr-omarchy-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("omarchy-dictation-kyrr");
        std::fs::write(&mine, "#!/bin/sh\necho mine\n").unwrap();
        assert!(write_script(&mine, &adapter("kyrr")).is_err());
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "#!/bin/sh\necho mine\n");
        let ours = dir.join("omarchy-install-dictation-kyrr");
        write_script(&ours, &installer("kyrr", true)).unwrap();
        write_script(&ours, &installer("kyrr", false)).unwrap();
        assert!(std::fs::read_to_string(&ours).unwrap().contains("'kyrr dictates now'"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scripts_parse() {
        use std::io::Write;
        for script in [adapter("/a b/kyrr"), installer("/a b/kyrr", true)] {
            let mut bash = std::process::Command::new("bash")
                .arg("-n")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            bash.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
            assert!(bash.wait().unwrap().success(), "{script}");
        }
    }

    #[test]
    fn chosen_backend() {
        let dir = std::env::temp_dir().join(format!("kyrr-omarchy-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("omarchy/defaults")).unwrap();
        assert_eq!(chosen(&dir), None);
        std::fs::write(dir.join("omarchy/defaults/dictation"), "voxtype\n").unwrap();
        assert_eq!(chosen(&dir).as_deref(), Some("voxtype"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
