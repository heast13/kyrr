//! Putting dictated text into whatever window has the focus: pasted through
//! the clipboard, which is put back as it was afterwards.
//!
//! Typing the text as key events loses keys in some apps (Windows 11 Notepad
//! drops them while it saves its tabs), and Enter sends the message in chat
//! apps. A paste arrives whole, line breaks included.

use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_CONTROL, VK_RETURN, VK_V,
};

/// The app reads the clipboard some time after Ctrl+V; only then does the
/// previous content go back.
const RESTORE_AFTER: Duration = Duration::from_millis(500);

/// Puts `text` into the focused window with Ctrl+V and then restores the
/// clipboard, unless something else was copied in the meantime. The text
/// stays out of the clipboard history. Returns false when Windows blocked
/// the input (for example into a window running as administrator); the text
/// is then left on the clipboard.
pub fn paste_text(text: &str) -> bool {
    let Some(saved) = Saved::take() else {
        return type_keys(text);
    };
    let windows_text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    if !set_text(&windows_text, true) {
        return type_keys(text);
    }
    let ours = unsafe { GetClipboardSequenceNumber() };
    if !send(&[
        vk(VK_CONTROL, false),
        vk(VK_V, false),
        vk(VK_V, true),
        vk(VK_CONTROL, true),
    ]) {
        return false;
    }
    std::thread::sleep(RESTORE_AFTER);
    if unsafe { GetClipboardSequenceNumber() } == ours {
        saved.restore();
    }
    true
}

/// Puts `text` on the clipboard, for when pasting was blocked: the user can
/// still paste it by hand. Returns false if the clipboard was not available.
pub fn copy_to_clipboard(text: &str) -> bool {
    set_text(&text.replace("\r\n", "\n").replace('\n', "\r\n"), false)
}

/// What was on the clipboard before, every format that is plain memory.
struct Saved(Vec<(u32, Vec<u8>)>);

impl Saved {
    /// None when the clipboard could not be opened.
    fn take() -> Option<Saved> {
        if !open() {
            return None;
        }
        let mut formats = Vec::new();
        let mut format = 0;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if !is_memory(format) {
                continue;
            }
            if let Some(bytes) = unsafe { read(format) } {
                formats.push((format, bytes));
            }
        }
        let _ = unsafe { CloseClipboard() };
        Some(Saved(formats))
    }

    fn restore(self) {
        if !open() {
            return;
        }
        unsafe {
            if EmptyClipboard().is_ok() {
                for (format, bytes) in &self.0 {
                    put(*format, bytes);
                }
                // Back as it was, not a new entry in the clipboard history.
                if !self.0.is_empty() {
                    put(format_id(w!("ExcludeClipboardContentFromMonitorProcessing")), &[0; 4]);
                }
            }
            let _ = CloseClipboard();
        }
    }
}

/// Formats whose data is a handle to something other than plain memory
/// (bitmaps, metafiles, palettes, owner-drawn, private and GDI objects)
/// cannot be copied byte for byte. Images survive as CF_DIB.
fn is_memory(format: u32) -> bool {
    !matches!(format, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E) && !(0x200..=0x3FF).contains(&format)
}

/// The clipboard must be open.
unsafe fn read(format: u32) -> Option<Vec<u8>> {
    let memory = HGLOBAL(unsafe { GetClipboardData(format) }.ok()?.0);
    let size = unsafe { GlobalSize(memory) };
    let data = unsafe { GlobalLock(memory) } as *const u8;
    if size == 0 || data.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, size) }.to_vec();
    let _ = unsafe { GlobalUnlock(memory) };
    Some(bytes)
}

/// The clipboard must be open and emptied by us.
unsafe fn put(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else {
            return false;
        };
        let target = GlobalLock(memory) as *mut u8;
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        let _ = GlobalUnlock(memory);
        // On success the clipboard owns the memory.
        if SetClipboardData(format, Some(HANDLE(memory.0))).is_err() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        true
    }
}

/// Replaces the clipboard with `text`; `private` keeps it out of the
/// clipboard history and the cloud clipboard.
fn set_text(text: &str, private: bool) -> bool {
    let bytes: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    if !open() {
        return false;
    }
    unsafe {
        let done = EmptyClipboard().is_ok() && put(u32::from(CF_UNICODETEXT.0), &bytes);
        if done && private {
            put(format_id(w!("ExcludeClipboardContentFromMonitorProcessing")), &[0; 4]);
            put(format_id(w!("CanIncludeInClipboardHistory")), &[0; 4]);
            put(format_id(w!("CanUploadToCloudClipboard")), &[0; 4]);
        }
        let _ = CloseClipboard();
        done
    }
}

fn format_id(name: windows::core::PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

/// Another app may hold the clipboard for a moment.
fn open() -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn vk(key: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: unsafe { MapVirtualKeyW(u32::from(key.0), MAPVK_VK_TO_VSC) } as u16,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn unicode(unit: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: if up {
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                } else {
                    KEYEVENTF_UNICODE
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// When the clipboard is not available: types `text` as key events, line
/// breaks as Enter. False when Windows blocked it.
fn type_keys(text: &str) -> bool {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for unit in text.replace("\r\n", "\n").encode_utf16() {
        if unit == u16::from(b'\n') {
            inputs.extend([vk(VK_RETURN, false), vk(VK_RETURN, true)]);
        } else {
            inputs.extend([unicode(unit, false), unicode(unit, true)]);
        }
    }
    send(&inputs)
}

/// Key events per call to `SendInput`. The input queue of a window holds
/// about 10,000 messages; a long dictation typed in one go would overflow it.
/// An even number, so no key goes down in one batch and up in the next.
const BATCH: usize = 256;

/// Pause between batches, for the receiving window to empty its queue.
const BATCH_PAUSE: Duration = Duration::from_millis(5);

fn send(inputs: &[INPUT]) -> bool {
    for (index, batch) in inputs.chunks(BATCH).enumerate() {
        if index > 0 {
            std::thread::sleep(BATCH_PAUSE);
        }
        if unsafe { SendInput(batch, std::mem::size_of::<INPUT>() as i32) } as usize != batch.len() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::is_memory;

    #[test]
    fn handle_formats_are_left_alone() {
        // CF_UNICODETEXT, CF_DIB, CF_HDROP and registered formats are memory.
        for format in [13, 8, 15, 0xC000] {
            assert!(is_memory(format), "{format}");
        }
        // CF_BITMAP, CF_ENHMETAFILE, private and GDI object formats are not.
        for format in [2, 14, 0x200, 0x3FF] {
            assert!(!is_memory(format), "{format}");
        }
    }
}
