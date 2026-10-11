//! Bringing kyrr's window to the front.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow,
    SW_RESTORE,
};

/// Restores the window `hwnd` and makes it the active one, in front of
/// everything. Windows only lets the program the user last worked with
/// take the front, so kyrr briefly joins that program's input for the
/// switch; otherwise the window would at times open behind the current one.
pub fn bring_to_front(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let foreground = GetForegroundWindow();
        if foreground == hwnd {
            return;
        }
        let ours = GetCurrentThreadId();
        let theirs = if foreground.is_invalid() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let joined = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if joined {
            let _ = AttachThreadInput(ours, theirs, false);
        }
    }
}
