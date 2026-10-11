//! The app's side of the monitor the APO shares (see `kyrr_common::monitor`):
//! the microphone level and the before/after probe.

use std::sync::atomic::Ordering;

use kyrr_common::monitor::{Header, HEADER_SIZE, MAGIC, NAME, PROBE_CAPACITY, SIZE};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS,
};

pub struct Monitor {
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

/// What the meter shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    /// Changes while an app uses the microphone.
    pub heartbeat: u32,
    /// Peak before and after kyrr, linear.
    pub before: f32,
    pub after: f32,
    /// What the automatic volume adds right now, in dB.
    pub gain_db: f32,
}

/// A finished probe: the same take before and after kyrr, mono.
pub struct Probe {
    pub rate: u32,
    pub before: Vec<f32>,
    pub after: Vec<f32>,
}

impl Monitor {
    /// Opens the monitor; `None` until an APO created it (a stream ran since
    /// the audio service started).
    pub fn open() -> Option<Monitor> {
        let name: Vec<u16> = NAME.encode_utf16().chain(Some(0)).collect();
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        let mapping = unsafe { OpenFileMappingW(access.0, false, PCWSTR(name.as_ptr())) }.ok()?;
        let view = unsafe { MapViewOfFile(mapping, access, 0, 0, SIZE) };
        if view.Value.is_null() {
            unsafe {
                let _ = CloseHandle(mapping);
            }
            return None;
        }
        let monitor = Monitor { mapping, view };
        (monitor.header().magic.load(Ordering::Acquire) == MAGIC).then_some(monitor)
    }

    fn header(&self) -> &Header {
        unsafe { &*(self.view.Value as *const Header) }
    }

    pub fn levels(&self) -> Levels {
        let h = self.header();
        Levels {
            heartbeat: h.heartbeat.load(Ordering::Relaxed),
            before: f32::from_bits(h.level_in.load(Ordering::Relaxed)),
            after: f32::from_bits(h.level_out.load(Ordering::Relaxed)),
            gain_db: f32::from_bits(h.level_gain.load(Ordering::Relaxed)),
        }
    }

    /// Asks the APO for a probe; returns its number for [`Monitor::probe`].
    pub fn request_probe(&self) -> u32 {
        let h = self.header();
        let id = h
            .probe_request
            .load(Ordering::Acquire)
            .max(h.probe_claimed.load(Ordering::Acquire))
            .wrapping_add(1);
        h.probe_request.store(id, Ordering::Release);
        id
    }

    /// The probe `id` once the APO finished it. The take is cleared from the
    /// shared memory once read. Everything in the mapping is treated as
    /// untrusted: other signed-in users can write it too.
    pub fn probe(&self, id: u32) -> Option<Probe> {
        let h = self.header();
        if h.probe_done.load(Ordering::Acquire) != id {
            return None;
        }
        let frames = (h.probe_frames.load(Ordering::Relaxed) as usize).min(PROBE_CAPACITY);
        let rate = h.probe_rate.load(Ordering::Relaxed);
        if !(8_000..=384_000).contains(&rate) {
            return None;
        }
        let buffer = |which: usize| unsafe {
            (self.view.Value as *mut u8).add(HEADER_SIZE + which * PROBE_CAPACITY * 4) as *mut f32
        };
        let read = |which: usize| -> Vec<f32> {
            let src = buffer(which);
            (0..frames)
                .map(|i| unsafe { src.add(i).read_volatile() })
                .map(|x| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 })
                .collect()
        };
        let probe = Probe {
            rate,
            before: read(0),
            after: read(1),
        };
        for which in 0..2 {
            unsafe { std::ptr::write_bytes(buffer(which), 0, frames) };
        }
        h.probe_frames.store(0, Ordering::Relaxed);
        Some(probe)
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
        }
    }
}
