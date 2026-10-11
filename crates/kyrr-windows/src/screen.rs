//! How much room a window has on the screen it is on.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};

/// The work area (the screen without the taskbar) of the monitor at or
/// nearest to (`x`, `y`), in physical pixels: left, top, right, bottom.
pub fn work_area(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let found = unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        GetMonitorInfoW(monitor, &mut info).as_bool()
    };
    let work = info.rcWork;
    (found && work.bottom > work.top && work.right > work.left).then_some((
        work.left,
        work.top,
        work.right,
        work.bottom,
    ))
}
