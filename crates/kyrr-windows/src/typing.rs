//! Typing text into whatever window has the focus, as Unicode key events.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};

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

/// Types `text`. Newlines become Enter presses. Returns false when Windows
/// blocked the input (for example into a window running as administrator).
pub fn type_text(text: &str) -> bool {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for unit in text.replace("\r\n", "\n").replace('\n', "\r").encode_utf16() {
        inputs.push(key(unit, false));
        inputs.push(key(unit, true));
    }
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}
