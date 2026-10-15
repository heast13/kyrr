//! Putting dictated text into whatever window has the focus on Linux.
//!
//! The text is typed as key events, never pasted: a paste needs Ctrl+V in
//! most apps but Ctrl+Shift+V in terminals, and the dictation cannot know
//! which window it is going to. Every backend takes keysyms, so the text can
//! contain any character without depending on the keyboard layout.
//!
//! The first backend that works is remembered:
//! 1. the Wayland virtual keyboard protocol (Hyprland, Sway and other wlroots
//!    compositors, no dialog),
//! 2. the XDG RemoteDesktop portal (GNOME, KDE), which asks once and then
//!    remembers the answer through a restore token,
//! 3. X11 XTest, when there is no Wayland session.

mod layout;
mod portal;
mod wayland;
mod x11;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Name under which the portal remembers the permission for this app.
const APP_ID: &str = "io.github.heast13.kyrr";

/// Pause between key events: some apps lose keys that come faster.
const KEY_DELAY: std::time::Duration = std::time::Duration::from_millis(2);

enum Backend {
    Wayland(wayland::Keyboard),
    /// The portal works; a session is opened for each text (see
    /// [`portal::Keyboard`]).
    Portal,
    X11(Box<x11::Keyboard>),
}

static BACKEND: Mutex<Option<Backend>> = Mutex::new(None);
static RESTORE_TOKEN: Mutex<Option<String>> = Mutex::new(None);
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// The state behind these mutexes stays valid when a thread panics.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Types `text` into the focused window, line breaks as Return. Blocks until
/// done; the first call under GNOME or KDE also waits for the permission
/// dialog. Returns false when no backend could type, or the user's layout
/// lacks a character the portal would then drop, so the caller can put the
/// text on the clipboard instead.
pub fn type_text(text: &str) -> bool {
    let keysyms: Vec<u32> = text.chars().filter_map(keysym).collect();
    if keysyms.is_empty() {
        return true;
    }
    let mut backend = lock(&BACKEND);
    if backend.is_none() {
        *backend = probe();
    }
    let done = match backend.as_mut() {
        Some(Backend::Wayland(keyboard)) => keyboard.type_keysyms(&keysyms),
        Some(Backend::Portal) => {
            let missing = layout::keysyms().is_some_and(|layout| keysyms.iter().any(|k| !layout.contains(k)));
            if missing {
                return false;
            }
            match portal::Keyboard::open() {
                Some(mut keyboard) => {
                    let typed = keyboard.type_keysyms(&keysyms);
                    keyboard.close();
                    typed
                }
                None => false,
            }
        }
        Some(Backend::X11(keyboard)) => keyboard.type_keysyms(&keysyms),
        None => false,
    };
    if !done {
        // The compositor may have restarted or the permission been revoked:
        // look again next time.
        *backend = None;
    }
    done
}

/// Gets the permission to type now instead of on the first dictation:
/// GNOME and KDE ask with their dialog. Blocks until answered. True when
/// typing will work.
pub fn prepare() -> bool {
    let mut backend = lock(&BACKEND);
    if backend.is_none() {
        *backend = probe();
    }
    backend.is_some()
}

/// True in a Flatpak when no way to type was found: the sandbox is the likely
/// reason (the compositor keeps the virtual keyboard from sandboxed clients
/// and has no RemoteDesktop portal, or the user said no to the portal), so the
/// app can say so instead of a bare "could not type".
pub fn blocked_by_sandbox() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() && lock(&BACKEND).is_none()
}

/// The portal's restore token, to be saved by the app. It changes only when
/// the permission was granted anew, so reading it after `type_text` is enough.
pub fn restore_token() -> Option<String> {
    lock(&RESTORE_TOKEN).clone()
}

/// Hands over the token saved earlier, before the first `type_text`; the
/// portal then does not ask again.
pub fn set_restore_token(token: Option<String>) {
    *lock(&RESTORE_TOKEN) = token;
}

fn probe() -> Option<Backend> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()) {
        return wayland::Keyboard::connect().map(Backend::Wayland).or_else(|| {
            portal::Keyboard::open().map(|keyboard| {
                keyboard.close();
                Backend::Portal
            })
        });
    }
    x11::Keyboard::connect().map(|keyboard| Backend::X11(Box::new(keyboard)))
}

/// Puts `text` on the clipboard, for when typing was not possible: the user
/// can still paste it by hand. The clipboard is kept open for the life of
/// the process because the owner has to answer the paste requests. Returns
/// false if no clipboard was reachable.
pub fn copy_to_clipboard(text: &str) -> bool {
    let mut held = lock(&CLIPBOARD);
    if held.is_none() {
        *held = arboard::Clipboard::new().ok();
    }
    let done = held.as_mut().is_some_and(|clipboard| clipboard.set_text(text).is_ok());
    if !done {
        *held = None;
    }
    done
}

/// The X11 keysym of `c`, None for characters that cannot be typed.
fn keysym(c: char) -> Option<u32> {
    match c {
        '\n' => Some(0xFF0D), // Return
        '\t' => Some(0xFF09), // Tab
        ' '..='~' | '\u{A0}'..='\u{FF}' => Some(u32::from(c)),
        // Other control characters (including a lone \r) are not text.
        '\0'..='\u{9F}' => None,
        _ => Some(0x0100_0000 + u32::from(c)),
    }
}

#[cfg(test)]
mod tests {
    use super::keysym;

    #[test]
    fn keysyms_follow_the_unicode_rule() {
        assert_eq!(keysym('a'), Some(0x61));
        assert_eq!(keysym('ä'), Some(0xE4));
        assert_eq!(keysym('ß'), Some(0xDF));
        assert_eq!(keysym('€'), Some(0x0100_20AC));
        assert_eq!(keysym('\n'), Some(0xFF0D));
        assert_eq!(keysym('\t'), Some(0xFF09));
    }

    #[test]
    fn control_characters_are_skipped() {
        assert_eq!(keysym('\r'), None);
        assert_eq!(keysym('\u{7F}'), None);
        assert_eq!(keysym('\u{85}'), None);
    }
}
