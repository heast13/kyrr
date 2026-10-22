//! Push-to-talk key: a low-level keyboard hook reports when the key goes down
//! and up, and swallows it while dictation is active so the focused app does
//! not also react to it. Auto-repeat while holding is ignored. With Ctrl,
//! Alt, Shift or Windows held the key belongs to the app (Ctrl+F9 in Excel,
//! Shift+F9 in Visual Studio).
//!
//! A second key (F8 unless the user picks another) starts and ends a voice
//! memo: its press and release are reported, the app decides whether they
//! mean tap or hold.
//!
//! A release can get lost: the screen locked (Win+L) or an administrator
//! window came to the front while the key was down. A key held down repeats
//! well within a second, so a press after a longer quiet is a new one: the
//! lost release is reported first, and the key works again at once.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_F9, VK_LCONTROL, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed,
    Released,
    /// The voice memo key went down.
    MemoPressed,
    /// The voice memo key came up.
    MemoReleased,
}

static KEY: AtomicU32 = AtomicU32::new(VK_F9.0 as u32);

/// A held key repeats at least every 400 ms, after a first wait of at most
/// a second; a press after a longer quiet is a new one.
const REPEAT_GAP_MS: u64 = 1500;

/// The push-to-talk key (a virtual-key code, e.g. `VK_F9`). The same key
/// again changes nothing (a dictation held meanwhile goes on); another one
/// ends what the old one held, whose release would never match.
pub fn set_key(vk: u32) {
    if KEY.swap(vk, Ordering::Relaxed) != vk && HELD.swap(false, Ordering::Relaxed) {
        send(KeyEvent::Released);
    }
}

/// Whether a modifier other than `key` itself is held.
fn modifier_held(key: u32) -> bool {
    let down = |vk: VIRTUAL_KEY| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0;
    let ctrl_key = key == VK_RCONTROL.0 as u32 || key == VK_LCONTROL.0 as u32;
    (!ctrl_key && down(VK_CONTROL)) || down(VK_MENU) || down(VK_SHIFT) || down(VK_LWIN) || down(VK_RWIN)
}
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// A key field in the settings waits for a key: the hook lets every key
/// through, so kyrr's window sees it.
static CAPTURING: AtomicBool = AtomicBool::new(false);
static HELD: AtomicBool = AtomicBool::new(false);
/// The voice memo key; 0 for none.
static MEMO_KEY: AtomicU32 = AtomicU32::new(0);
static MEMO_HELD: AtomicBool = AtomicBool::new(false);
/// When each key last went down (milliseconds since Windows started).
static LAST_DOWN: AtomicU64 = AtomicU64::new(0);
static MEMO_LAST_DOWN: AtomicU64 = AtomicU64::new(0);

/// The voice memo key (a virtual-key code), 0 for none; as [`set_key`].
pub fn set_memo_key(vk: u32) {
    if MEMO_KEY.swap(vk, Ordering::Relaxed) != vk && MEMO_HELD.swap(false, Ordering::Relaxed) {
        send_memo(false);
    }
}
static SENDER: OnceLock<mpsc::Sender<KeyEvent>> = OnceLock::new();

fn send(event: KeyEvent) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(event);
    }
}

/// While a key field waits for a key, the keys are not taken: the window
/// gets them, and no dictation or memo starts.
pub fn set_capture(on: bool) {
    // First the keys go to the window, then a key still down is let go: its
    // release would be lost to the window.
    CAPTURING.store(on, Ordering::Relaxed);
    if on {
        if HELD.swap(false, Ordering::Relaxed) {
            send(KeyEvent::Released);
        }
        if MEMO_HELD.swap(false, Ordering::Relaxed) {
            send_memo(false);
        }
    }
}

/// Whether a press of a key last down at `last` (now `now`, both in
/// milliseconds) that still counts as held is a new press: its release got
/// lost.
fn release_lost(last: u64, now: u64) -> bool {
    now.saturating_sub(last) > REPEAT_GAP_MS
}

/// Reports the memo key going down or up.
fn send_memo(pressed: bool) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(if pressed {
            KeyEvent::MemoPressed
        } else {
            KeyEvent::MemoReleased
        });
    }
}

/// Whether the key is captured. When inactive, the key reaches apps normally.
pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Relaxed);
}

/// What a key event does. True when kyrr takes it and the app it was meant
/// for does not see it.
fn process(vk: u32, down: bool, up: bool, injected: bool) -> bool {
    // A key field in the settings waits for a key: it is the window's.
    if CAPTURING.load(Ordering::Relaxed) {
        return false;
    }
    let key = KEY.load(Ordering::Relaxed);
    let ours = vk == key && !injected;
    let memo = MEMO_KEY.load(Ordering::Relaxed);
    let now = || unsafe { GetTickCount64() };
    if memo != 0 && vk == memo && !injected {
        if up && MEMO_HELD.swap(false, Ordering::Relaxed) {
            send_memo(false);
            return true;
        }
        if down {
            let now = now();
            let last = MEMO_LAST_DOWN.swap(now, Ordering::Relaxed);
            if MEMO_HELD.load(Ordering::Relaxed) && release_lost(last, now) && MEMO_HELD.swap(false, Ordering::Relaxed)
            {
                send_memo(false);
            }
        }
        if down && (MEMO_HELD.load(Ordering::Relaxed) || !modifier_held(memo)) {
            // Auto-repeat while it is held is one press.
            if !MEMO_HELD.swap(true, Ordering::Relaxed) {
                send_memo(true);
            }
            return true;
        }
    }
    if ours && up && HELD.swap(false, Ordering::Relaxed) {
        // Released after a press we took: report it even if dictation
        // stopped meanwhile, so the next press is seen again.
        send(KeyEvent::Released);
        return true;
    }
    if ours && down {
        let now = now();
        let last = LAST_DOWN.swap(now, Ordering::Relaxed);
        if HELD.load(Ordering::Relaxed) && release_lost(last, now) && HELD.swap(false, Ordering::Relaxed) {
            send(KeyEvent::Released);
        }
    }
    if ours && down && ACTIVE.load(Ordering::Relaxed) && (HELD.load(Ordering::Relaxed) || !modifier_held(key)) {
        if !HELD.swap(true, Ordering::Relaxed) {
            send(KeyEvent::Pressed);
        }
        return true;
    }
    false
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let msg = wparam.0 as u32;
        let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(msg, WM_KEYUP | WM_SYSKEYUP);
        if process(info.vkCode, down, up, (info.flags.0 & LLKHF_INJECTED.0) != 0) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// A key that reached kyrr's own window. Windows does not run the hook for
/// those, so the window passes them on; the keys then work there as in
/// every other app. True when kyrr takes the key.
pub fn window_key(vk: u32, down: bool) -> bool {
    process(vk, down, !down, false)
}

/// Starts the hook on its own thread (hooks need a message loop) and returns
/// the receiver for key events. Call once.
pub fn start() -> Result<mpsc::Receiver<KeyEvent>, String> {
    let (tx, rx) = mpsc::channel();
    SENDER.set(tx).map_err(|_| "hotkey already started")?;
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kyrr-hotkey".into())
        .spawn(move || {
            // Windows drops a hook that answers too slowly, silently: ahead
            // of the speech model's threads on a busy computer.
            let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
            let module = unsafe { GetModuleHandleW(None) }.ok().map(|m| HINSTANCE(m.0));
            let installed = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0) };
            let _ = ready_tx.send(installed.as_ref().map(|_| ()).map_err(|e| e.message()));
            if installed.is_err() {
                return;
            }
            let mut msg = MSG::default();
            // -1 is an error, not a message.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}
        })
        .map_err(|e| e.to_string())?;
    ready_rx.recv().map_err(|e| e.to_string())??;
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_after_a_long_quiet_is_a_new_one() {
        // Auto-repeat: every 33 ms, the first after up to a second.
        assert!(!release_lost(10_000, 10_033));
        assert!(!release_lost(10_000, 11_000));
        // Back from the lock screen.
        assert!(release_lost(10_000, 25_000));
        // A clock that went back never counts.
        assert!(!release_lost(10_000, 9_000));
    }
}
