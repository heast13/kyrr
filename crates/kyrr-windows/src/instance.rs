//! One kyrr per user session. A second start asks the running one to show
//! its window and exits.

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
};

pub enum Instance {
    /// This is the first instance. `show_requests` fires when another start
    /// wants the window shown.
    First {
        show_requests: std::sync::mpsc::Receiver<kyrr_common::Request>,
    },
    /// Another instance runs and was asked to show itself.
    Other,
}

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

pub fn claim() -> Instance {
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\kyrr-instance")) };
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if mutex.is_ok() && exists {
        if let Ok(event) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\kyrr-show")) } {
            let _ = unsafe { SetEvent(event) };
        }
        return Instance::Other;
    }
    // The mutex handle stays open for the life of the process.
    std::mem::forget(mutex);
    let (tx, rx) = std::sync::mpsc::channel();
    if let Ok(event) = unsafe { CreateEventW(None, false, false, w!("Local\\kyrr-show")) } {
        let event = SendHandle(event);
        let _ = std::thread::Builder::new().name("kyrr-instance".into()).spawn(move || {
            let event = event;
            loop {
                unsafe { WaitForSingleObject(event.0, INFINITE) };
                if tx.send(kyrr_common::Request::Show).is_err() {
                    break;
                }
            }
        });
    }
    Instance::First { show_requests: rx }
}
