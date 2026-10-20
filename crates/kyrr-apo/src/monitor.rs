//! The APO's side of the monitor shared with the app: it publishes the
//! microphone level and records the before/after probe the app asks for.
//! Everything the app writes is treated as untrusted; the APO only ever
//! reads `probe_request` from it.

use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use kyrr_common::monitor::{Header, HEADER_SIZE, MAGIC, NAME, PROBE_CAPACITY, PROBE_SECONDS, SIZE};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HLOCAL, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::System::Memory::{CreateFileMappingW, MapViewOfFile, FILE_MAP_ALL_ACCESS, PAGE_READWRITE};

use crate::log::log;
use crate::registry::wide;

/// The audio service and SYSTEM own it; interactively signed-in users read
/// the level and probe and write the probe request. Not services, network
/// or batch logons. The app clears the probe after reading it.
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;LS)(A;;GRGW;;;IU)";

/// How much the published level falls per block (about 10 ms): roughly
/// 30 dB in a third of a second, slow enough for a meter polled at 20 Hz.
const LEVEL_FALL: f32 = 0.9;

pub struct Monitor {
    base: *mut u8,
}

// The mapping lives for the whole process; all shared fields are atomics
// or probe samples written by one claimed instance at a time.
unsafe impl Send for Monitor {}
unsafe impl Sync for Monitor {}

/// The process-wide monitor; created on first use, `None` if that failed.
pub fn monitor() -> Option<&'static Monitor> {
    static MONITOR: OnceLock<Option<Monitor>> = OnceLock::new();
    MONITOR
        .get_or_init(|| match create() {
            Ok(m) => Some(m),
            Err(e) => {
                log(&format!("monitor unavailable: {e}"));
                None
            }
        })
        .as_ref()
}

fn create() -> Result<Monitor, String> {
    let sddl = wide(SDDL);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None)
    }
    .map_err(|e| format!("security descriptor: {e}"))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let name = wide(NAME);
    let mapping = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            Some(&attributes),
            PAGE_READWRITE,
            0,
            SIZE as u32,
            PCWSTR(name.as_ptr()),
        )
    };
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(sd.0)));
    }
    // The handle stays open for the life of the process.
    let mapping = mapping.map_err(|e| format!("mapping: {e}"))?;
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, SIZE) };
    if view.Value.is_null() {
        return Err("map view failed".into());
    }
    let monitor = Monitor {
        base: view.Value as *mut u8,
    };
    monitor.header().magic.store(MAGIC, Ordering::Release);
    Ok(monitor)
}

impl Monitor {
    fn header(&self) -> &Header {
        unsafe { &*(self.base as *const Header) }
    }

    /// Probe buffer 0 (before kyrr) or 1 (after kyrr).
    fn probe_ptr(&self, which: usize) -> *mut f32 {
        unsafe { self.base.add(HEADER_SIZE + which * PROBE_CAPACITY * 4) as *mut f32 }
    }

    /// Publishes the peak of a block before and after kyrr, what the
    /// automatic volume adds and whether the microphone sounds far.
    pub fn levels(&self, before: f32, after: f32, gain_db: f32, far: bool) {
        let h = self.header();
        h.level_gain.store(gain_db.to_bits(), Ordering::Relaxed);
        h.far.store(far as u32, Ordering::Relaxed);
        h.heartbeat.fetch_add(1, Ordering::Relaxed);
        for (slot, peak) in [(&h.level_in, before), (&h.level_out, after)] {
            let old = f32::from_bits(slot.load(Ordering::Relaxed));
            slot.store((old * LEVEL_FALL).max(peak).to_bits(), Ordering::Relaxed);
        }
    }

    /// A probe this instance should start recording, if the app asked for
    /// one and no other instance took it yet.
    pub fn claim_probe(&self, rate: u32) -> Option<Probe> {
        let h = self.header();
        let request = h.probe_request.load(Ordering::Acquire);
        let claimed = h.probe_claimed.load(Ordering::Acquire);
        if request == claimed {
            return None;
        }
        h.probe_claimed
            .compare_exchange(claimed, request, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        h.probe_rate.store(rate, Ordering::Relaxed);
        h.probe_frames.store(0, Ordering::Relaxed);
        let frames = (rate as usize * PROBE_SECONDS as usize).min(PROBE_CAPACITY);
        Some(Probe {
            id: request,
            pos: 0,
            frames,
        })
    }

    /// Appends the mono mix of `buf` to probe buffer `which` at the probe's
    /// position (the position moves on in [`Monitor::advance`]).
    pub fn record(&self, probe: &Probe, which: usize, buf: &[f32], channels: usize) {
        let dst = self.probe_ptr(which);
        for (i, frame) in buf.chunks_exact(channels).enumerate() {
            let at = probe.pos + i;
            if at >= probe.frames {
                break;
            }
            let v = frame.iter().sum::<f32>() / channels as f32;
            unsafe { dst.add(at).write_volatile(v) };
        }
    }

    /// Moves the probe on by `frames`; returns false once it is complete.
    pub fn advance(&self, probe: &mut Probe, frames: usize) -> bool {
        probe.pos = (probe.pos + frames).min(probe.frames);
        let h = self.header();
        h.probe_frames.store(probe.pos as u32, Ordering::Relaxed);
        if probe.pos < probe.frames {
            return true;
        }
        h.probe_done.store(probe.id, Ordering::Release);
        false
    }
}

/// A probe being recorded by one APO instance.
pub struct Probe {
    id: u32,
    pos: usize,
    frames: usize,
}

/// Peak of an interleaved block.
pub fn peak(buf: &[f32]) -> f32 {
    buf.iter()
        .fold(0f32, |m, x| if x.is_finite() { m.max(x.abs()) } else { m })
}
