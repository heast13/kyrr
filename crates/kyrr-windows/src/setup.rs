//! Installing kyrr's audio filter into Windows, attaching it to a
//! microphone, and undoing all of it. Needs an elevated process.
//!
//! What install changes, and uninstall restores exactly:
//! - the COM class and APO registration of `kyrr_apo.dll`
//! - `DisableProtectedAudioDG = 1`, which lets the audio engine load APOs
//!   not signed by Microsoft (Equalizer APO sets the same value); the prior
//!   state is recorded and put back
//! - `HKLM\SOFTWARE\kyrr` with the settings key users may write and the
//!   status key the audio service may write
//!
//! Attaching a microphone puts kyrr into the endpoint's stream-effect slot
//! and records the driver's effect it replaced, which kyrr then runs first.

use kyrr_common::{endpoint, guid_string, APO_CLSID, APO_NAME, REG_ENDPOINTS, REG_FILTER, REG_ROOT, REG_STATUS};
use windows::core::Interface;
use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use crate::privilege::enable_backup_restore;
use crate::reg::{delete_tree, Key};

const AUDIO_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Audio";
const PROTECTED_AUDIO: &str = "DisableProtectedAudioDG";
/// DisableProtectedAudioDG before kyrr touched it; `ABSENT` when there was none.
const PRIOR_PROTECTED_AUDIO: &str = "PriorDisableProtectedAudioDG";
const ABSENT: u32 = u32::MAX;

const CAPTURE_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture";
/// Effect slots of an endpoint's FxProperties: 5 is the stream effect (SFX)
/// since Windows 8.1, 1 the older pre-mix effect (LFX) some drivers still use.
const FX_GUID: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}";
/// PKEY_AudioEndpoint_Disable_SysFx: 1 = "audio enhancements" switched off.
const DISABLE_SYSFX: &str = "{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5";

/// Below an endpoint key in `REG_ENDPOINTS`.
const SLOT: &str = "Slot";
const PRIOR_DISABLE_SYSFX: &str = "PriorDisableSysFx";

/// Users may set filter values; the audio service (LocalService) may write
/// its status. Administrators and SYSTEM keep full control.
const FILTER_SDDL: &str = "D:P(A;OICI;KA;;;SY)(A;OICI;KA;;;BA)(A;OICI;KRKW;;;BU)(A;OICI;KR;;;LS)";
const STATUS_SDDL: &str = "D:P(A;OICI;KA;;;SY)(A;OICI;KA;;;BA)(A;OICI;KR;;;BU)(A;OICI;KRKW;;;LS)";

fn clsid() -> String {
    guid_string(APO_CLSID)
}

fn com_key() -> String {
    format!(r"SOFTWARE\Classes\CLSID\{}", clsid())
}

fn apo_key() -> String {
    format!(r"SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\{}", clsid())
}

fn fx_value(slot: u32) -> String {
    format!("{FX_GUID},{slot}")
}

/// Registers the APO DLL at `dll` and prepares kyrr's registry keys.
pub fn install(dll: &std::path::Path) -> Result<(), String> {
    let dll = dll.to_str().ok_or("install path is not valid Unicode")?;

    let com = Key::create(HKEY_LOCAL_MACHINE, &com_key(), None)?;
    com.set_string("", APO_NAME)?;
    let server = Key::create(HKEY_LOCAL_MACHINE, &format!(r"{}\InprocServer32", com_key()), None)?;
    server.set_string("", dll)?;
    server.set_string("ThreadingModel", "Both")?;

    // The values RegisterAPO would write (APO_REG_PROPERTIES).
    let apo = Key::create(HKEY_LOCAL_MACHINE, &apo_key(), None)?;
    apo.set_string("FriendlyName", APO_NAME)?;
    apo.set_string("Copyright", "Copyright (c) 2026 heast13")?;
    for (name, value) in [
        ("MajorVersion", 1),
        ("MinorVersion", 0),
        ("Flags", 15), // APO_FLAG_DEFAULT | APO_FLAG_INPLACE
        ("MinInputConnections", 1),
        ("MaxInputConnections", 1),
        ("MinOutputConnections", 1),
        ("MaxOutputConnections", 1),
        ("MaxInstances", u32::MAX),
        ("NumAPOInterfaces", 1),
    ] {
        apo.set_dword(name, value)?;
    }
    apo.set_string("APOInterface0", &guid_string(IAudioProcessingObject::IID.to_u128()))?;

    let root = Key::create(HKEY_LOCAL_MACHINE, REG_ROOT, None)?;
    let audio = Key::create(HKEY_LOCAL_MACHINE, AUDIO_KEY, None)?;
    if root.get_dword(PRIOR_PROTECTED_AUDIO).is_none() {
        root.set_dword(
            PRIOR_PROTECTED_AUDIO,
            audio.get_dword(PROTECTED_AUDIO).unwrap_or(ABSENT),
        )?;
    }
    audio.set_dword(PROTECTED_AUDIO, 1)?;

    Key::create(HKEY_LOCAL_MACHINE, REG_FILTER, Some(FILTER_SDDL))?;
    Key::create(HKEY_LOCAL_MACHINE, REG_STATUS, Some(STATUS_SDDL))?;
    Key::create(HKEY_LOCAL_MACHINE, REG_ENDPOINTS, None)?;
    Ok(())
}

/// Detaches every microphone and removes everything [`install`] added.
pub fn uninstall() -> Result<(), String> {
    for guid in attached() {
        detach(&guid)?;
    }
    delete_tree(HKEY_LOCAL_MACHINE, &apo_key())?;
    delete_tree(HKEY_LOCAL_MACHINE, &com_key())?;
    if let Some(root) = Key::open(HKEY_LOCAL_MACHINE, REG_ROOT) {
        if let Some(prior) = root.get_dword(PRIOR_PROTECTED_AUDIO) {
            let audio = Key::create(HKEY_LOCAL_MACHINE, AUDIO_KEY, None)?;
            match prior {
                ABSENT => audio.delete_value(PROTECTED_AUDIO)?,
                v => audio.set_dword(PROTECTED_AUDIO, v)?,
            }
        }
    }
    delete_tree(HKEY_LOCAL_MACHINE, REG_ROOT)
}

/// Endpoint GUIDs kyrr is attached to.
pub fn attached() -> Vec<String> {
    Key::open(HKEY_LOCAL_MACHINE, REG_ENDPOINTS).map_or_else(Vec::new, |k| k.subkeys())
}

/// Whether the APO is registered (the app checks this before attaching).
pub fn is_installed() -> bool {
    Key::open(HKEY_LOCAL_MACHINE, &apo_key()).is_some()
}

/// Puts kyrr into the stream-effect slot of microphone `guid`.
pub fn attach(guid: &str) -> Result<(), String> {
    if !is_installed() {
        return Err("kyrr's audio filter is not installed".into());
    }
    enable_backup_restore()?;
    let ours = clsid();
    let base = format!(r"{CAPTURE_KEY}\{guid}");
    let fx = Key::create_privileged(HKEY_LOCAL_MACHINE, &format!(r"{base}\FxProperties"))?;
    let record = Key::create(HKEY_LOCAL_MACHINE, &format!(r"{REG_ENDPOINTS}\{guid}"), None)?;

    if record.get_dword(SLOT).is_none() {
        let current = |slot| fx.get_string(&fx_value(slot)).filter(|v| !v.is_empty());
        let (slot, child) = match (current(5), current(1)) {
            (Some(v), _) => (5, Some(v)),
            (None, Some(v)) => (1, Some(v)),
            (None, None) => (5, None),
        };
        if let Some(child) = child.filter(|c| !c.eq_ignore_ascii_case(&ours)) {
            record.set_string(endpoint::CHILD_SFX, &child)?;
        }
        record.set_dword(SLOT, slot)?;
    }
    let slot = record.get_dword(SLOT).unwrap_or(5);
    fx.set_string(&fx_value(slot), &ours)?;

    // Effects only run with "audio enhancements" on for the microphone.
    let props = Key::create_privileged(HKEY_LOCAL_MACHINE, &format!(r"{base}\Properties"))?;
    if let Some(v) = props.get_dword(DISABLE_SYSFX).filter(|&v| v != 0) {
        if record.get_dword(PRIOR_DISABLE_SYSFX).is_none() {
            record.set_dword(PRIOR_DISABLE_SYSFX, v)?;
        }
        props.set_dword(DISABLE_SYSFX, 0)?;
    }
    Ok(())
}

/// Takes kyrr out of microphone `guid` and puts the driver's effect back.
pub fn detach(guid: &str) -> Result<(), String> {
    let Some(record) = Key::open(HKEY_LOCAL_MACHINE, &format!(r"{REG_ENDPOINTS}\{guid}")) else {
        return Ok(());
    };
    enable_backup_restore()?;
    let base = format!(r"{CAPTURE_KEY}\{guid}");
    let fx = Key::create_privileged(HKEY_LOCAL_MACHINE, &format!(r"{base}\FxProperties"))?;
    let slot = record.get_dword(SLOT).unwrap_or(5);
    let name = fx_value(slot);
    let is_ours = fx.get_string(&name).is_some_and(|v| v.eq_ignore_ascii_case(&clsid()));
    if is_ours {
        match record.get_string(endpoint::CHILD_SFX) {
            Some(child) => fx.set_string(&name, &child)?,
            None => fx.delete_value(&name)?,
        }
    }
    if let Some(prior) = record.get_dword(PRIOR_DISABLE_SYSFX) {
        let props = Key::create_privileged(HKEY_LOCAL_MACHINE, &format!(r"{base}\Properties"))?;
        props.set_dword(DISABLE_SYSFX, prior)?;
    }
    drop(record);
    delete_tree(HKEY_LOCAL_MACHINE, &format!(r"{REG_ENDPOINTS}\{guid}"))
}
