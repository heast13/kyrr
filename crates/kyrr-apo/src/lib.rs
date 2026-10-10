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
mod registry;
mod settings;

use std::ffi::c_void;

use windows::core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};

pub use apo::parse_guid;

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(&self, punkouter: Ref<IUnknown>, riid: *const GUID, ppvobject: *mut *mut c_void) -> Result<()> {
        if ppvobject.is_null() || riid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppvobject = std::ptr::null_mut() };
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let unknown: IUnknown = apo::KyrrApo::new().into();
        unsafe { unknown.query(riid, ppvobject).ok() }
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

/// COM entry point: the DLL may unload once no APO object is alive.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    apo::can_unload()
}
