//! Push-to-talk key: a low-level keyboard hook reports when the key goes down
//! and up, and swallows it while dictation is active so the focused app does
//! not also react to it. Auto-repeat while holding is ignored.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_F9;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed,
    Released,
}

static KEY: AtomicU32 = AtomicU32::new(VK_F9.0 as u32);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELD: AtomicBool = AtomicBool::new(false);
static SENDER: OnceLock<mpsc::Sender<KeyEvent>> = OnceLock::new();

/// Whether the key is captured. When inactive, the key reaches apps normally.
pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Relaxed);
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && ACTIVE.load(Ordering::Relaxed) {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let ours = info.vkCode == KEY.load(Ordering::Relaxed) && (info.flags.0 & LLKHF_INJECTED.0) == 0;
        if ours {
            let msg = wparam.0 as u32;
            let event = match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN if !HELD.swap(true, Ordering::Relaxed) => Some(KeyEvent::Pressed),
                WM_KEYUP | WM_SYSKEYUP if HELD.swap(false, Ordering::Relaxed) => Some(KeyEvent::Released),
                _ => None,
            };
            if let (Some(event), Some(tx)) = (event, SENDER.get()) {
                let _ = tx.send(event);
            }
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
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
            let module = unsafe { GetModuleHandleW(None) }.ok().map(|m| HINSTANCE(m.0));
            let installed = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0) };
            let _ = ready_tx.send(installed.as_ref().map(|_| ()).map_err(|e| e.message()));
            if installed.is_err() {
                return;
            }
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {}
        })
        .map_err(|e| e.to_string())?;
    ready_rx.recv().map_err(|e| e.to_string())??;
    Ok(rx)
}
