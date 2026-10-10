//! Restarting the Windows audio services so endpoint effect changes apply.
//! Sound stops for a moment.

use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW, SC_HANDLE,
    SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS,
    SERVICE_STATUS_CURRENT_STATE, SERVICE_STOP, SERVICE_STOPPED,
};

use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;

use crate::reg::wide;

const TIMEOUT: Duration = Duration::from_secs(20);

struct Handle(SC_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn state(service: &Handle) -> Option<SERVICE_STATUS_CURRENT_STATE> {
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(service.0, &mut status) }.ok()?;
    Some(status.dwCurrentState)
}

fn wait_for(service: &Handle, wanted: SERVICE_STATUS_CURRENT_STATE) -> bool {
    let start = Instant::now();
    while start.elapsed() < TIMEOUT {
        if state(service) == Some(wanted) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn open(manager: &Handle, name: &str) -> Result<Handle, String> {
    let n = wide(name);
    unsafe {
        OpenServiceW(
            manager.0,
            PCWSTR(n.as_ptr()),
            SERVICE_STOP | SERVICE_START | SERVICE_QUERY_STATUS,
        )
    }
    .map(Handle)
    .map_err(|e| format!("{name}: {e}"))
}

/// Restarts AudioEndpointBuilder (which reads the endpoint effect settings)
/// and the audio service that depends on it. Whatever goes wrong while
/// stopping, both are started again: a failed restart must never leave
/// Windows without sound.
pub fn restart_audio() -> Result<(), String> {
    let manager = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
        .map(Handle)
        .map_err(|e| format!("service manager: {e}"))?;
    let audio = open(&manager, "Audiosrv")?;
    let builder = open(&manager, "AudioEndpointBuilder")?;
    let mut first_error = None;
    for service in [&audio, &builder] {
        if state(service) != Some(SERVICE_STOPPED) {
            let mut status = SERVICE_STATUS::default();
            let _ = unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) };
            if !wait_for(service, SERVICE_STOPPED) {
                first_error = Some("audio service did not stop".to_string());
                break;
            }
        }
    }
    for service in [&builder, &audio] {
        if state(service) == Some(SERVICE_RUNNING) {
            continue;
        }
        if let Err(e) = unsafe { StartServiceW(service.0, None) } {
            if e.code() != ERROR_SERVICE_ALREADY_RUNNING.to_hresult() {
                first_error.get_or_insert(format!("audio service start: {e}"));
                continue;
            }
        }
        if !wait_for(service, SERVICE_RUNNING) {
            first_error.get_or_insert("audio service did not start".to_string());
        }
    }
    first_error.map_or(Ok(()), Err)
}
