//! Typing text into whatever window has the focus, as Unicode key events.

use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    VIRTUAL_KEY, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// Pause between words, so the text arrives as a quick wave rather than in
/// one block.
const WORD_PAUSE: Duration = Duration::from_millis(30);

/// A long text gets shorter pauses: the whole wave takes at most this long.
const WAVE_MAX: Duration = Duration::from_millis(900);

fn key(unit: u16, up: bool) -> INPUT {
    let flags = if up {
        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
    } else {
        KEYEVENTF_UNICODE
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// The Enter key itself: some apps ignore a Unicode carriage return.
fn enter(up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Types `text` word by word, as a quick wave. Newlines become Enter
/// presses. Returns how many bytes of `text` were typed: all of them, or
/// fewer when Windows blocked the input (for example into a window running
/// as administrator) or the focus moved to another window on the way.
pub fn type_text(text: &str) -> usize {
    let words = words(text);
    let pause = (WAVE_MAX / words.len().max(1) as u32).min(WORD_PAUSE);
    let window = unsafe { GetForegroundWindow() };
    let mut done = 0;
    for (i, word) in words.into_iter().enumerate() {
        if i > 0 {
            std::thread::sleep(pause);
            if unsafe { GetForegroundWindow() } != window {
                break;
            }
        }
        if !send(word) {
            break;
        }
        done += word.len();
    }
    done
}

/// Splits `text` after each run of whitespace: every piece is a word with
/// the spaces or line breaks after it.
fn words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let (mut start, mut in_space) = (0, false);
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            in_space = true;
        } else if in_space {
            words.push(&text[start..i]);
            (start, in_space) = (i, false);
        }
    }
    if start < text.len() {
        words.push(&text[start..]);
    }
    words
}

/// Sends one piece in a single batch. False when Windows blocked it.
fn send(text: &str) -> bool {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for unit in text.replace("\r\n", "\n").encode_utf16() {
        if unit == u16::from(b'\n') {
            inputs.push(enter(false));
            inputs.push(enter(true));
        } else {
            inputs.push(key(unit, false));
            inputs.push(key(unit, true));
        }
    }
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// Puts `text` on the clipboard, for when typing was blocked: the user can
/// still paste it. Returns false if the clipboard was not available.
pub fn copy_to_clipboard(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let target = GlobalLock(memory) as *mut u16;
            if target.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            let _ = GlobalUnlock(memory);
            // The clipboard owns the memory from here on.
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))).ok()?;
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        done
    }
}

#[cfg(test)]
mod tests {
    use super::words;

    #[test]
    fn words_keep_their_spaces_and_line_breaks() {
        let text = "Hallo Welt.
Wie  geht's? ";
        assert_eq!(
            words(text),
            [
                "Hallo ", "Welt.
", "Wie  ", "geht's? "
            ]
        );
        assert_eq!(words(text).concat(), text);
        assert!(words("").is_empty());
        assert_eq!(words("  vorne"), ["  ", "vorne"]);
    }
}
