//! Microphones (active capture endpoints) as Windows lists them.

use windows::core::PCWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToBSTR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED, STGM_READ,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    /// Full endpoint id, e.g. `{0.0.1.00000000}.{6f1e...}`.
    pub id: String,
    /// The endpoint GUID, the name of its key in the registry.
    pub guid: String,
    pub name: String,
    pub is_default: bool,
}

/// The GUID part of an endpoint id.
pub fn guid_of(id: &str) -> &str {
    id.rsplit_once("}.").map_or(id, |(_, g)| g)
}

fn id_of(device: &IMMDevice) -> Option<String> {
    let pwstr = unsafe { device.GetId() }.ok()?;
    let id = unsafe { pwstr.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(pwstr.0 as *const _)) };
    id
}

fn name_of(device: &IMMDevice) -> Option<String> {
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
    let value = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }.ok()?;
    Some(unsafe { PropVariantToBSTR(&value) }.ok()?.to_string())
}

/// Active microphones, the default one first.
pub fn microphones() -> Result<Vec<Microphone>, String> {
    unsafe {
        // Called on the UI thread, which must stay single-threaded (STA): winit's
        // OleInitialize fails later otherwise. Already initialized is fine.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|e| format!("devices: {e}"))?;
    let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }
        .ok()
        .and_then(|d| id_of(&d));
    let collection =
        unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }.map_err(|e| format!("devices: {e}"))?;
    let count = unsafe { collection.GetCount() }.map_err(|e| format!("devices: {e}"))?;
    let mut mics = Vec::new();
    for i in 0..count {
        let Ok(device) = (unsafe { collection.Item(i) }) else {
            continue;
        };
        let Some(id) = id_of(&device) else {
            continue;
        };
        mics.push(Microphone {
            guid: guid_of(&id).to_string(),
            name: name_of(&device).unwrap_or_else(|| id.clone()),
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
        });
    }
    mics.sort_by_key(|m| !m.is_default);
    Ok(mics)
}

/// The microphone's volume control (Windows' own mute switch and level).
fn volume_of(id: &str) -> Option<IAudioEndpointVolume> {
    unsafe {
        // The UI thread is already STA (see `microphones`); already initialized is fine.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    let device = unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }.ok()?;
    unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()
}

/// Whether Windows has the microphone (full endpoint id) muted; None when it
/// cannot be told.
pub fn is_muted(id: &str) -> Option<bool> {
    Some(unsafe { volume_of(id)?.GetMute() }.ok()?.as_bool())
}

/// Switches the microphone's Windows mute off. True when it worked.
pub fn unmute(id: &str) -> bool {
    volume_of(id).is_some_and(|v| unsafe { v.SetMute(false, std::ptr::null()) }.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a microphone: reads the mute state of every active one and
    /// switches it off (it is not on).
    #[test]
    #[ignore = "needs audio hardware"]
    fn reads_the_mute_state_of_every_microphone() {
        for m in microphones().unwrap() {
            let muted = is_muted(&m.id);
            println!("{}: muted {muted:?}", m.name);
            assert!(muted.is_some(), "{}", m.name);
            // Switching mute off is harmless where it is off already.
            assert!(unmute(&m.id), "{}", m.name);
        }
    }
}
