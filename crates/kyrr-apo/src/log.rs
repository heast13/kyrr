//! Logging for rare events (load, format, errors), never per audio block.
//!
//! The APO runs inside `audiodg.exe` as LocalService, so the log lands in
//! that account's temp folder:
//! `C:\Windows\ServiceProfiles\LocalService\AppData\Local\Temp\kyrr-apo.log`.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;

/// The log is cut when it grows beyond this, keeping it from filling the disk
/// on a machine where something fails on every stream start.
const MAX_LOG_BYTES: u64 = 512 * 1024;

fn log_path() -> PathBuf {
    std::env::temp_dir().join("kyrr-apo.log")
}

pub fn log(msg: &str) {
    let wide: Vec<u16> = format!("kyrr-apo: {msg}\n").encode_utf16().chain(Some(0)).collect();
    unsafe { OutputDebugStringW(PCWSTR(wide.as_ptr())) };

    let path = log_path();
    let truncate = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(&path);
    if let Ok(mut f) = file {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "{secs} [{}] {msg}", std::process::id());
    }
}
