//! The dictation wave on the screen (see `kyrr_common::wave`): a layered
//! window above all others, at the bottom of the screen the user types on.
//! It never becomes the active window, has no taskbar button and lets
//! clicks through, so the text still goes where the user was typing.

use std::ffi::c_void;

use kyrr_common::wave::{Surface, HEIGHT, MARGIN};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetMonitorInfoW, MonitorFromWindow, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, PeekMessageW,
    RegisterClassExW, ShowWindow, UpdateLayeredWindow, MA_NOACTIVATE, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    WM_MOUSEACTIVATE, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};

const CLASS: windows::core::PCWSTR = w!("kyrr-wave");

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub struct Overlay {
    hwnd: HWND,
    dc: HDC,
    bitmap: Option<(HBITMAP, HGDIOBJ, *mut u32)>,
    size: (u32, u32),
    scale: f32,
    /// Middle of the pill's bottom edge, physical pixels.
    anchor: (i32, i32),
    shown: bool,
}

impl Overlay {
    /// On the screen of the window the user works in.
    pub fn open() -> Option<Overlay> {
        unsafe {
            let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return None;
            }
            let (mut dpi, mut unused) = (96, 0);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused).is_err() {
                dpi = 96;
            }
            let scale = (dpi as f32 / 96.0).clamp(1.0, 4.0);
            let work = info.rcWork;
            let anchor = (
                (work.left + work.right) / 2,
                work.bottom - (MARGIN as f32 * scale) as i32,
            );

            let instance = GetModuleHandleW(None).ok()?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            // Fails harmlessly when the class is already there.
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                w!("kyrr"),
                WS_POPUP,
                anchor.0,
                anchor.1 - (HEIGHT as f32 * scale) as i32,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()?;
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                let _ = DestroyWindow(hwnd);
                return None;
            }
            Some(Overlay {
                hwnd,
                dc,
                bitmap: None,
                size: (0, 0),
                scale,
                anchor,
                shown: false,
            })
        }
    }

    /// A top-down 32-bit bitmap of `width` × `height` to draw into.
    fn bits(&mut self, width: u32, height: u32) -> Option<*mut u32> {
        if self.size == (width, height) {
            return self.bitmap.map(|(_, _, bits)| bits);
        }
        self.free_bitmap();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        unsafe {
            let bitmap = CreateDIBSection(Some(self.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            if bits.is_null() {
                let _ = DeleteObject(bitmap.into());
                return None;
            }
            let old = SelectObject(self.dc, bitmap.into());
            self.bitmap = Some((bitmap, old, bits.cast()));
        }
        self.size = (width, height);
        Some(bits.cast())
    }

    fn free_bitmap(&mut self) {
        if let Some((bitmap, old, _)) = self.bitmap.take() {
            unsafe {
                SelectObject(self.dc, old);
                let _ = DeleteObject(bitmap.into());
            }
        }
        self.size = (0, 0);
    }
}

impl Surface for Overlay {
    fn scale(&mut self) -> f32 {
        self.scale
    }

    fn show(&mut self, pixels: &[u32], width: u32, height: u32) -> bool {
        if pixels.len() < (width * height) as usize {
            return false;
        }
        unsafe {
            // Nothing much arrives for a window like this; handle it anyway.
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
            let Some(bits) = self.bits(width, height) else {
                return false;
            };
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits, (width * height) as usize);
            let position = POINT {
                x: self.anchor.0 - width as i32 / 2,
                y: self.anchor.1 - height as i32,
            };
            let size = SIZE {
                cx: width as i32,
                cy: height as i32,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let source = POINT::default();
            let updated = UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&position),
                Some(&size),
                Some(self.dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            if updated.is_err() {
                return false;
            }
            if !self.shown {
                self.shown = true;
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        true
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.free_bitmap();
        unsafe {
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
