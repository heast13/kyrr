//! The COM object around [`KyrrApo`], written by hand because the audio
//! engine creates APOs aggregated: it passes an outer `IUnknown` to
//! `CreateInstance` and asks for our non-delegating `IUnknown`. The
//! `implement` macro cannot do that, and an APO refusing aggregation is
//! silently left out of the stream.
//!
//! Layout: one vtable pointer per interface, the outer object every
//! interface delegates `QueryInterface`/`AddRef`/`Release` to, and our own
//! reference count, which only the non-delegating `IUnknown` touches. Without
//! aggregation the outer object is that non-delegating `IUnknown` itself.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_NOINTERFACE, E_NOTIMPL, E_POINTER};
use windows::Win32::Media::Audio::Apo::{
    IAudioProcessingObject, IAudioProcessingObjectConfiguration, IAudioProcessingObjectConfiguration_Vtbl,
    IAudioProcessingObjectRT, IAudioProcessingObjectRT_Vtbl, IAudioProcessingObject_Vtbl, IAudioSystemEffects,
    IAudioSystemEffects_Vtbl,
};
use windows_core::{ComObject, ComObjectInner, IUnknownImpl};

use crate::apo::KyrrApo;

#[repr(C)]
pub struct Object {
    // Interface pointers point at these fields; the `OFFSET` of each vtable
    // below is the field's index, leading back to the start of the object.
    inner_vtbl: &'static IUnknown_Vtbl,
    apo_vtbl: &'static IAudioProcessingObject_Vtbl,
    rt_vtbl: &'static IAudioProcessingObjectRT_Vtbl,
    config_vtbl: &'static IAudioProcessingObjectConfiguration_Vtbl,
    fx_vtbl: &'static IAudioSystemEffects_Vtbl,
    /// Not counted: the outer object owns us, not the other way round.
    outer: *mut c_void,
    refs: AtomicU32,
    apo: KyrrApo,
}

static INNER_VTBL: IUnknown_Vtbl = IUnknown_Vtbl {
    QueryInterface: inner_query,
    AddRef: inner_add_ref,
    Release: inner_release,
};
static APO_VTBL: IAudioProcessingObject_Vtbl = IAudioProcessingObject_Vtbl::new::<Object, -1>();
static RT_VTBL: IAudioProcessingObjectRT_Vtbl = IAudioProcessingObjectRT_Vtbl::new::<Object, -2>();
static CONFIG_VTBL: IAudioProcessingObjectConfiguration_Vtbl =
    IAudioProcessingObjectConfiguration_Vtbl::new::<Object, -3>();
static FX_VTBL: IAudioSystemEffects_Vtbl = IAudioSystemEffects_Vtbl::new::<Object, -4>();

impl std::ops::Deref for Object {
    type Target = KyrrApo;
    fn deref(&self) -> &KyrrApo {
        &self.apo
    }
}

impl Object {
    fn outer(&self) -> &IUnknown {
        unsafe { IUnknown::from_raw_borrowed(&self.outer) }.expect("outer is never null")
    }

    /// Hands out the interface `iid` and counts it the way that interface
    /// counts: through the outer object, or our own count for `IUnknown`.
    unsafe fn query(&self, iid: &GUID, out: *mut *mut c_void) -> HRESULT {
        let field: *const &'static c_void = if *iid == IUnknown::IID {
            std::ptr::addr_of!(self.inner_vtbl).cast()
        } else if *iid == IAudioProcessingObject::IID {
            std::ptr::addr_of!(self.apo_vtbl).cast()
        } else if *iid == IAudioProcessingObjectRT::IID {
            std::ptr::addr_of!(self.rt_vtbl).cast()
        } else if *iid == IAudioProcessingObjectConfiguration::IID {
            std::ptr::addr_of!(self.config_vtbl).cast()
        } else if *iid == IAudioSystemEffects::IID {
            std::ptr::addr_of!(self.fx_vtbl).cast()
        } else {
            unsafe { *out = std::ptr::null_mut() };
            return E_NOINTERFACE;
        };
        let interface = field as *mut c_void;
        // Every interface starts with IUnknown's three methods.
        let vtbl = unsafe { *(interface as *const *const IUnknown_Vtbl) };
        unsafe { ((*vtbl).AddRef)(interface) };
        unsafe { *out = interface };
        HRESULT(0)
    }
}

unsafe extern "system" fn inner_query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let this = unsafe { &*(this as *const Object) };
    unsafe { this.query(&*iid, out) }
}

unsafe extern "system" fn inner_add_ref(this: *mut c_void) -> u32 {
    let this = unsafe { &*(this as *const Object) };
    this.refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn inner_release(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *const Object) };
    let refs = object.refs.fetch_sub(1, Ordering::Release) - 1;
    if refs == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        // The filter, its engine and the driver's effect go here; a panic on
        // the way must not unwind into the audio engine (what is left leaks).
        let object = unsafe { Box::from_raw(this as *mut Object) };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(object)));
    }
    refs
}

/// Creates the APO for `IClassFactory::CreateInstance`. With an outer object,
/// COM rules allow only `IUnknown`, and we return our non-delegating one.
///
/// # Safety
/// `outer` is null or a valid `IUnknown`; `iid` and `out` are valid.
pub unsafe fn create(outer: *mut c_void, iid: &GUID, out: *mut *mut c_void) -> HRESULT {
    if !outer.is_null() && *iid != IUnknown::IID {
        return CLASS_E_NOAGGREGATION;
    }
    let object = Box::into_raw(Box::new(Object {
        inner_vtbl: &INNER_VTBL,
        apo_vtbl: &APO_VTBL,
        rt_vtbl: &RT_VTBL,
        config_vtbl: &CONFIG_VTBL,
        fx_vtbl: &FX_VTBL,
        outer,
        refs: AtomicU32::new(1),
        apo: KyrrApo::new(),
    }));
    if outer.is_null() {
        unsafe { (*object).outer = object as *mut c_void };
    }
    let hr = unsafe { inner_query(object as *mut c_void, iid, out) };
    unsafe { inner_release(object as *mut c_void) };
    hr
}

// What the generated vtables call for IUnknown: always the outer object.
impl IUnknownImpl for Object {
    type Impl = KyrrApo;

    fn get_impl(&self) -> &KyrrApo {
        &self.apo
    }

    fn get_impl_mut(&mut self) -> &mut KyrrApo {
        &mut self.apo
    }

    fn into_inner(self) -> KyrrApo {
        self.apo
    }

    unsafe fn QueryInterface(&self, iid: *const GUID, interface: *mut *mut c_void) -> HRESULT {
        unsafe { self.outer().query(iid, interface) }
    }

    fn AddRef(&self) -> u32 {
        let outer = self.outer();
        unsafe { (outer.vtable().AddRef)(outer.as_raw()) }
    }

    unsafe fn Release(self_: *mut Self) -> u32 {
        let outer = unsafe { (*self_).outer };
        unsafe { ((**(outer as *const *const IUnknown_Vtbl)).Release)(outer) }
    }

    fn is_reference_count_one(&self) -> bool {
        self.refs.load(Ordering::Acquire) == 1
    }

    unsafe fn GetTrustLevel(&self, _value: *mut i32) -> HRESULT {
        E_NOTIMPL
    }

    fn to_object(&self) -> ComObject<KyrrApo>
    where
        for<'a> KyrrApo: ComObjectInner<Outer = Self>,
    {
        unreachable!("kyrr's APO is not a ComObject")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::Audio::Apo::APO_FLAG_INPLACE;
    use windows::Win32::System::Com::CoTaskMemFree;

    /// An outer object the way the audio engine aggregates an APO: it holds
    /// the inner `IUnknown` and forwards queries for other interfaces to it.
    #[repr(C)]
    struct Outer {
        vtbl: &'static IUnknown_Vtbl,
        refs: AtomicU32,
        inner: *mut c_void,
    }

    static OUTER_VTBL: IUnknown_Vtbl = IUnknown_Vtbl {
        QueryInterface: outer_query,
        AddRef: outer_add_ref,
        Release: outer_release,
    };

    unsafe extern "system" fn outer_query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let this = unsafe { &*(this as *const Outer) };
        if unsafe { *iid } == IUnknown::IID {
            this.refs.fetch_add(1, Ordering::Relaxed);
            unsafe { *out = this as *const Outer as *mut c_void };
            return HRESULT(0);
        }
        unsafe { ((**(this.inner as *const *const IUnknown_Vtbl)).QueryInterface)(this.inner, iid, out) }
    }

    unsafe extern "system" fn outer_add_ref(this: *mut c_void) -> u32 {
        unsafe { &*(this as *const Outer) }.refs.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn outer_release(this: *mut c_void) -> u32 {
        unsafe { &*(this as *const Outer) }.refs.fetch_sub(1, Ordering::Relaxed) - 1
    }

    #[test]
    fn works_aggregated_like_in_the_audio_engine() {
        let mut outer = Outer {
            vtbl: &OUTER_VTBL,
            refs: AtomicU32::new(1),
            inner: std::ptr::null_mut(),
        };
        let outer_ptr = &mut outer as *mut Outer as *mut c_void;

        // Aggregation hands out only the non-delegating IUnknown.
        let mut refused = std::ptr::null_mut();
        let hr = unsafe { create(outer_ptr, &IAudioProcessingObject::IID, &mut refused) };
        assert_eq!(hr, CLASS_E_NOAGGREGATION);
        assert!(refused.is_null());

        let hr = unsafe { create(outer_ptr, &IUnknown::IID, &mut outer.inner) };
        assert_eq!(hr, HRESULT(0));
        assert!(!outer.inner.is_null());

        // Interfaces come through the outer object and count on it.
        let outer_unknown = unsafe { IUnknown::from_raw_borrowed(&(outer_ptr)) }.unwrap();
        let apo: IAudioProcessingObject = outer_unknown.cast().unwrap();
        assert_eq!(outer.refs.load(Ordering::Relaxed), 2);
        let props = unsafe { apo.GetRegistrationProperties() }.unwrap();
        let flags = unsafe { (*props).Flags };
        unsafe { CoTaskMemFree(Some(props as *const c_void)) };
        assert_ne!(flags.0 & APO_FLAG_INPLACE.0, 0);
        let _rt: IAudioProcessingObjectRT = apo.cast().unwrap();
        let _config: IAudioProcessingObjectConfiguration = apo.cast().unwrap();
        let _fx: IAudioSystemEffects = apo.cast().unwrap();
        assert_eq!(outer.refs.load(Ordering::Relaxed), 5);
        drop((apo, _rt, _config, _fx));
        assert_eq!(outer.refs.load(Ordering::Relaxed), 1);

        // Releasing the inner IUnknown frees the APO.
        assert_eq!(unsafe { inner_release(outer.inner) }, 0);
    }

    #[test]
    fn works_on_its_own() {
        let mut raw = std::ptr::null_mut();
        let hr = unsafe { create(std::ptr::null_mut(), &IAudioProcessingObject::IID, &mut raw) };
        assert_eq!(hr, HRESULT(0));
        let apo = unsafe { IAudioProcessingObject::from_raw(raw) };
        let unknown: IUnknown = apo.cast().unwrap();
        assert_eq!(unknown.as_raw(), unknown.cast::<IUnknown>().unwrap().as_raw());
        assert!(unsafe { apo.GetLatency() }.unwrap() > 0);
    }
}
