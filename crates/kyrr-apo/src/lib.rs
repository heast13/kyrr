//! kyrr's voice filter as a Windows audio processing object (APO).
//!
//! The DLL is loaded by the audio engine (`audiodg.exe`) for the microphone
//! kyrr is attached to. The installer registers the COM class and puts it
//! into the endpoint's stream-effect slot; the driver's original effect, if
//! any, keeps running as a child (see `child.rs`).

#![cfg(windows)]

mod apo;
mod child;
mod log;
mod monitor;
mod object;
mod registry;
mod settings;
mod swap;

use std::ffi::c_void;

use windows::core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_POINTER};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};

pub use apo::parse_guid;

use crate::log::log;

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(&self, punkouter: Ref<IUnknown>, riid: *const GUID, ppvobject: *mut *mut c_void) -> Result<()> {
        if ppvobject.is_null() || riid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppvobject = std::ptr::null_mut() };
        let outer = punkouter.as_ref().map_or(std::ptr::null_mut(), |o| o.as_raw());
        let hr = unsafe { object::create(outer, &*riid, ppvobject) };
        log(&format!(
            "created for {:?}, aggregated: {}, result {hr}",
            unsafe { *riid },
            if outer.is_null() { "no" } else { "yes" }
        ));
        hr.ok()
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM entry point: hands out the class factory for kyrr's CLSID.
///
/// # Safety
/// Called by COM with valid pointers.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = std::ptr::null_mut() };
    if unsafe { *rclsid } != GUID::from_u128(kyrr_common::APO_CLSID) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = ClassFactory.into();
    unsafe { factory.query(riid, ppv) }
}

/// Keeps the DLL loaded for the rest of the process (called on the first
/// object): its settings watcher and model loaders run its code.
pub(crate) fn pin_module() {
    use std::sync::Once;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    };
    static PIN: Once = Once::new();
    PIN.call_once(|| {
        let mut module = HMODULE::default();
        let anchor = pin_module as *const () as *const u16;
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN;
        let _ = unsafe { GetModuleHandleExW(flags, windows::core::PCWSTR(anchor), &mut module) };
    });
}

/// COM entry point: the DLL may unload once no APO object is alive (it
/// stays pinned once an object existed, see `pin_module`).
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    apo::can_unload()
}
