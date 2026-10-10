//! Push-to-talk key: a low-level keyboard hook reports when the key goes down
//! and up, and swallows it while dictation is active so the focused app does
//! not also react to it. Auto-repeat while holding is ignored. With Ctrl,
//! Alt, Shift or Windows held the key belongs to the app (Ctrl+F9 in Excel,
//! Shift+F9 in Visual Studio).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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
}

static KEY: AtomicU32 = AtomicU32::new(VK_F9.0 as u32);

/// The push-to-talk key (a virtual-key code, e.g. `VK_F9`).
pub fn set_key(vk: u32) {
    KEY.store(vk, Ordering::Relaxed);
    HELD.store(false, Ordering::Relaxed);
}

/// Whether a modifier other than `key` itself is held.
fn modifier_held(key: u32) -> bool {
    let down = |vk: VIRTUAL_KEY| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0;
    let ctrl_key = key == VK_RCONTROL.0 as u32 || key == VK_LCONTROL.0 as u32;
    (!ctrl_key && down(VK_CONTROL)) || down(VK_MENU) || down(VK_SHIFT) || down(VK_LWIN) || down(VK_RWIN)
}
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELD: AtomicBool = AtomicBool::new(false);
static SENDER: OnceLock<mpsc::Sender<KeyEvent>> = OnceLock::new();

/// Whether the key is captured. When inactive, the key reaches apps normally.
pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Relaxed);
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let key = KEY.load(Ordering::Relaxed);
        let ours = info.vkCode == key && (info.flags.0 & LLKHF_INJECTED.0) == 0;
        let msg = wparam.0 as u32;
        let up = matches!(msg, WM_KEYUP | WM_SYSKEYUP);
        if ours && up && HELD.swap(false, Ordering::Relaxed) {
            // Released after a press we took: report it even if dictation
            // stopped meanwhile, so the next press is seen again.
            if let Some(tx) = SENDER.get() {
                let _ = tx.send(KeyEvent::Released);
            }
            return LRESULT(1);
        }
        let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
        if ours && down && ACTIVE.load(Ordering::Relaxed) && (HELD.load(Ordering::Relaxed) || !modifier_held(key)) {
            if !HELD.swap(true, Ordering::Relaxed) {
                if let Some(tx) = SENDER.get() {
                    let _ = tx.send(KeyEvent::Pressed);
                }
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
