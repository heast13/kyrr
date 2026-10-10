//! The driver's own stream effect, which kyrr replaced in the endpoint's
//! effect slot. kyrr creates it, forwards every call and runs it before its
//! own processing, so driver features such as beamforming keep working.

use windows::core::{Interface, GUID};
use windows::Win32::Media::Audio::Apo::{
    APOInitBaseStruct, IAudioMediaType, IAudioProcessingObject, IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectRT, APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

pub struct Child {
    apo: IAudioProcessingObject,
    rt: IAudioProcessingObjectRT,
    config: IAudioProcessingObjectConfiguration,
}

// The interfaces are only used under the owning APO's lock, the way the
// audio engine would call the child directly.
unsafe impl Send for Child {}

impl Child {
    /// Creates the effect `clsid` and initializes it with the engine's data,
    /// with the class id in the init header replaced by the child's.
    pub fn create(clsid: GUID, init: &[u8]) -> windows::core::Result<Child> {
        let apo: IAudioProcessingObject = unsafe { CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER)? };
        let rt = apo.cast()?;
        let config = apo.cast()?;
        let mut data = init.to_vec();
        let offset = std::mem::offset_of!(APOInitBaseStruct, clsid);
        if data.len() >= offset + std::mem::size_of::<GUID>() {
            unsafe { std::ptr::write_unaligned(data.as_mut_ptr().add(offset).cast::<GUID>(), clsid) };
        }
        unsafe { apo.Initialize(&data)? };
        Ok(Child { apo, rt, config })
    }

    pub fn is_input_supported(
        &self,
        opposite: Option<&IAudioMediaType>,
        requested: Option<&IAudioMediaType>,
    ) -> windows::core::Result<IAudioMediaType> {
        unsafe { self.apo.IsInputFormatSupported(opposite, requested) }
    }

    pub fn is_output_supported(
        &self,
        opposite: Option<&IAudioMediaType>,
        requested: Option<&IAudioMediaType>,
    ) -> windows::core::Result<IAudioMediaType> {
        unsafe { self.apo.IsOutputFormatSupported(opposite, requested) }
    }

    pub fn latency(&self) -> i64 {
        unsafe { self.apo.GetLatency() }.unwrap_or(0)
    }

    pub fn input_channels(&self) -> windows::core::Result<u32> {
        unsafe { self.apo.GetInputChannelCount() }
    }

    pub fn reset(&self) {
        let _ = unsafe { self.apo.Reset() };
    }

    pub fn lock(
        &self,
        inputs: &[*const APO_CONNECTION_DESCRIPTOR],
        outputs: &[*const APO_CONNECTION_DESCRIPTOR],
    ) -> windows::core::Result<()> {
        unsafe { self.config.LockForProcess(inputs, outputs) }
    }

    pub fn unlock(&self) {
        let _ = unsafe { self.config.UnlockForProcess() };
    }

    /// # Safety
    /// Same contract as `IAudioProcessingObjectRT::APOProcess`.
    pub unsafe fn process(
        &self,
        n_in: u32,
        inputs: *const *const APO_CONNECTION_PROPERTY,
        n_out: u32,
        outputs: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        unsafe { self.rt.APOProcess(n_in, inputs, n_out, outputs) }
    }
}
