//! GNOME before 48 has no shortcut portal on Wayland:
//! kyrr cannot catch a key there. A custom shortcut of GNOME's own can run
//! `kyrr --dictate`, though, and kyrr adds that one itself, through
//! `gsettings`, like the user would under Settings › Keyboard › Custom
//! Shortcuts. GNOME runs it on the press only, so each press starts or ends
//! dictation. The command is quoted with [`crate::binds::command`].
//!
//! The shortcut is kyrr's own entry in the list of custom shortcuts; the
//! others are left alone, and a key one of them already uses is not taken.
//! The voice memo key is a second entry of kyrr's, running `kyrr --memo`.

use std::process::Command;

const APP_ID: &str = "io.github.heast13.kyrr";
const LIST: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ENTRY: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const OURS: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/kyrr/";
const OURS_MEMO: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/kyrr-memo/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The shortcut is there. `written` when kyrr just added or changed it.
    Bound { written: bool },
    /// Another custom shortcut, by this name, already uses the key.
    Taken { name: String },
    /// `gsettings` is missing or refused.
    Failed,
}

/// Binds `key` (e.g. "F9") to `command` in GNOME's custom shortcuts.
pub fn ensure(key: &str, command: &str) -> Outcome {
    bind(OURS, "kyrr", key, command, |theirs| {
        theirs.contains("--dictate") || (theirs.contains(APP_ID) && !theirs.contains("--memo"))
    })
}

/// Binds the voice memo key to `command` (`kyrr --memo`), or takes kyrr's
/// entry for it out (`None`).
pub fn ensure_memo(key: Option<&str>, command: &str) -> Outcome {
    match key {
        Some(key) => bind(OURS_MEMO, "kyrr: voice memo", key, command, |theirs| {
            theirs.contains("--memo")
        }),
        None => remove(OURS_MEMO),
    }
}

/// kyrr's entry `ours` with `name`: `key` runs `command`. `kyrrs` tells a
/// shortcut the user added by hand for kyrr by its command.
fn bind(ours: &str, name: &str, key: &str, command: &str, kyrrs: impl Fn(&str) -> bool) -> Outcome {
    let Some(list) = get(LIST, "custom-keybindings").and_then(|v| parse_strv(&v)) else {
        return Outcome::Failed;
    };
    // kyrr's two entries never stand in each other's way: swapping the
    // keys, each is rewritten in turn.
    for path in list.iter().filter(|p| p.as_str() != OURS && p.as_str() != OURS_MEMO) {
        let entry = format!("{ENTRY}:{path}");
        let binding = get(&entry, "binding").and_then(|v| parse_str(&v)).unwrap_or_default();
        if !binding.eq_ignore_ascii_case(key) {
            continue;
        }
        // One the user added by hand, as an earlier kyrr asked.
        let theirs = get(&entry, "command").and_then(|v| parse_str(&v)).unwrap_or_default();
        if kyrrs(&theirs) {
            return Outcome::Bound { written: false };
        }
        let name = get(&entry, "name").and_then(|v| parse_str(&v)).unwrap_or_default();
        return Outcome::Taken { name };
    }
    let entry = format!("{ENTRY}:{ours}");
    let mut written = false;
    // The entry first, so GNOME never sees half of it.
    for (field, value) in [("name", name), ("command", command), ("binding", key)] {
        if get(&entry, field).and_then(|v| parse_str(&v)).as_deref() == Some(value) {
            continue;
        }
        if !set(&entry, field, &quote(value)) {
            return Outcome::Failed;
        }
        written = true;
    }
    if !list.iter().any(|p| p == ours) {
        let mut list = list;
        list.push(ours.to_string());
        if !set(LIST, "custom-keybindings", &format_strv(&list)) {
            return Outcome::Failed;
        }
        written = true;
    }
    Outcome::Bound { written }
}

/// Takes kyrr's entry `ours` out of the list, if it is there.
fn remove(ours: &str) -> Outcome {
    let Some(list) = get(LIST, "custom-keybindings").and_then(|v| parse_strv(&v)) else {
        return Outcome::Failed;
    };
    if !list.iter().any(|p| p == ours) {
        return Outcome::Bound { written: false };
    }
    let rest: Vec<String> = list.into_iter().filter(|p| p != ours).collect();
    if !set(LIST, "custom-keybindings", &format_strv(&rest)) {
        return Outcome::Failed;
    }
    Outcome::Bound { written: true }
}

fn get(schema: &str, key: &str) -> Option<String> {
    let out = Command::new("gsettings").args(["get", schema, key]).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn set(schema: &str, key: &str, value: &str) -> bool {
    let ok = Command::new("gsettings")
        .args(["set", schema, key, value])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("kyrr: gsettings set {schema} {key} failed");
    }
    ok
}

/// A string in GVariant's text form.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn format_strv(list: &[String]) -> String {
    let items: Vec<String> = list.iter().map(|s| quote(s)).collect();
    format!("[{}]", items.join(", "))
}

/// The string `gsettings get` printed (`'F9'`, or `"it's"`).
fn parse_str(text: &str) -> Option<String> {
    let (value, rest) = take_str(text.trim())?;
    rest.trim().is_empty().then_some(value)
}

/// The string list `gsettings get` printed (`['/a/', '/b/']`, `@as []`).
fn parse_strv(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    let text = text.strip_prefix("@as").unwrap_or(text).trim();
    let mut rest = text.strip_prefix('[')?.trim_start();
    let mut list = Vec::new();
    loop {
        if let Some(after) = rest.strip_prefix(']') {
            return after.trim().is_empty().then_some(list);
        }
        let (item, after) = take_str(rest)?;
        list.push(item);
        rest = after.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
}

/// A quoted string at the start of `text`, and what follows it.
fn take_str(text: &str) -> Option<(String, &str)> {
    let mut chars = text.char_indices();
    let (_, q) = chars.next().filter(|&(_, c)| c == '\'' || c == '"')?;
    let mut value = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => value.push(chars.next()?.1),
            c if c == q => return Some((value, &text[i + 1..])),
            c => value.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_gsettings_prints() {
        assert_eq!(parse_strv("@as []"), Some(vec![]));
        assert_eq!(
            parse_strv("['/a/custom0/', '/a/kyrr/']"),
            Some(vec!["/a/custom0/".to_string(), "/a/kyrr/".to_string()])
        );
        assert_eq!(parse_str("'F9'").as_deref(), Some("F9"));
        assert_eq!(parse_str("\"it's\"").as_deref(), Some("it's"));
        assert_eq!(parse_str("'a\\'b'").as_deref(), Some("a'b"));
        assert_eq!(parse_strv("nonsense"), None);
    }

    #[test]
    fn writes_what_gsettings_reads() {
        assert_eq!(quote("it's"), "'it\\'s'");
        assert_eq!(parse_str(&quote("a\\b'c")).as_deref(), Some("a\\b'c"));
        let list = vec!["/x/".to_string(), OURS.to_string()];
        assert_eq!(parse_strv(&format_strv(&list)), Some(list));
    }
}
