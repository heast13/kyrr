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

use kyrr_common::{
    endpoint, guid_string, status, APO_CLSID, APO_DLL, APO_NAME, REG_ENDPOINTS, REG_FILTER, REG_ROOT, REG_STATUS,
};
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

/// The APO DLL next to this executable, if that is an install folder only
/// administrators can write to (Program Files). Setup commands take no
/// path: a path from the command line would let any program that gets the
/// user to accept a "kyrr" permission prompt plant its own DLL in the
/// audio engine.
pub fn installed_dll() -> Result<std::path::PathBuf, String> {
    use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no install folder")?;
    let program_files =
        unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None) }.map_err(|e| e.to_string())?;
    let program_files = std::path::PathBuf::from(unsafe { program_files.to_string() }.map_err(|e| e.to_string())?);
    if !dir.starts_with(&program_files) {
        return Err(format!("kyrr is not installed under {}", program_files.display()));
    }
    let dll = dir.join(APO_DLL);
    if !dll.is_file() {
        return Err(format!("{} missing", dll.display()));
    }
    Ok(dll)
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

/// Equalizer APO's settings key: when it is installed, it needs
/// `DisableProtectedAudioDG` as much as kyrr did.
const EQUALIZER_APO_KEY: &str = r"SOFTWARE\EqualizerAPO";

/// Detaches every microphone and removes everything [`install`] added.
/// Every step runs even when an earlier one failed, so a single problem
/// does not leave a half-removed filter behind; the errors are reported
/// together.
pub fn uninstall() -> Result<(), String> {
    let mut errors = Vec::new();
    let mut step = |r: Result<(), String>| {
        if let Err(e) = r {
            errors.push(e);
        }
    };
    for guid in attached() {
        step(detach(&guid));
    }
    step(delete_tree(HKEY_LOCAL_MACHINE, &apo_key()));
    step(delete_tree(HKEY_LOCAL_MACHINE, &com_key()));
    step(restore_protected_audio());
    step(delete_tree(HKEY_LOCAL_MACHINE, REG_ROOT));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Puts `DisableProtectedAudioDG` back as it was before kyrr, unless
/// someone changed it since or another audio tool (Equalizer APO) relies
/// on it now.
fn restore_protected_audio() -> Result<(), String> {
    let Some(prior) = Key::open(HKEY_LOCAL_MACHINE, REG_ROOT).and_then(|r| r.get_dword(PRIOR_PROTECTED_AUDIO)) else {
        return Ok(());
    };
    let audio = Key::create(HKEY_LOCAL_MACHINE, AUDIO_KEY, None)?;
    if audio.get_dword(PROTECTED_AUDIO) != Some(1) || Key::open(HKEY_LOCAL_MACHINE, EQUALIZER_APO_KEY).is_some() {
        return Ok(());
    }
    match prior {
        ABSENT => audio.delete_value(PROTECTED_AUDIO),
        1 => Ok(()),
        v => audio.set_dword(PROTECTED_AUDIO, v),
    }
}

/// Whether `s` is an endpoint GUID as the registry names it:
/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`. Setup commands take nothing else.
pub fn is_endpoint_guid(s: &str) -> bool {
    let Some(inner) = s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return false;
    };
    let groups: Vec<&str> = inner.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Whether microphone `guid`'s effect slot still holds kyrr. Windows
/// updates, driver updates or a sound-settings reset can take it out.
/// Reads only, no admin rights needed.
pub fn slot_holds_kyrr(guid: &str) -> bool {
    let Some(record) = Key::open(HKEY_LOCAL_MACHINE, &format!(r"{REG_ENDPOINTS}\{guid}")) else {
        return false;
    };
    let slot = record.get_dword(SLOT).unwrap_or(5);
    Key::open(HKEY_LOCAL_MACHINE, &format!(r"{CAPTURE_KEY}\{guid}\FxProperties"))
        .and_then(|fx| fx.get_string(&fx_value(slot)))
        .is_some_and(|v| v.eq_ignore_ascii_case(&clsid()))
}

/// Forgets what the filter last reported, so a status from another
/// microphone or an earlier run is not taken for the new one.
fn clear_status() {
    if let Some(key) = Key::open_writable(HKEY_LOCAL_MACHINE, REG_STATUS) {
        for name in [status::STATE, status::ENGINE, status::FORMAT, status::UPDATED] {
            let _ = key.delete_value(name);
        }
    }
}

/// Moves the filter to microphone `guid`: detaches it from every other one
/// and attaches it there, in one elevated step.
pub fn move_to(guid: &str) -> Result<(), String> {
    for old in attached().into_iter().filter(|g| !g.eq_ignore_ascii_case(guid)) {
        detach(&old)?;
    }
    attach(guid)
}

/// Endpoint GUIDs kyrr is attached to.
pub fn attached() -> Vec<String> {
    Key::open(HKEY_LOCAL_MACHINE, REG_ENDPOINTS).map_or_else(Vec::new, |k| k.subkeys())
}

/// Whether this PC can run the filter. The audio engine is native code and
/// kyrr's DLL is x64: Windows on ARM runs kyrr.exe emulated, but its audio
/// engine cannot load the DLL, and a microphone with it attached could stop
/// working altogether.
pub fn filter_supported() -> bool {
    use windows::Win32::System::SystemInformation::{IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_ARM64};
    use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    let (mut process, mut native) = (IMAGE_FILE_MACHINE::default(), IMAGE_FILE_MACHINE::default());
    let known = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) }.is_ok();
    !(known && native == IMAGE_FILE_MACHINE_ARM64)
}

/// Whether the APO is registered (the app checks this before attaching).
pub fn is_installed() -> bool {
    Key::open(HKEY_LOCAL_MACHINE, &apo_key()).is_some()
}

/// Puts kyrr into the stream-effect slot of microphone `guid`.
pub fn attach(guid: &str) -> Result<(), String> {
    if !is_endpoint_guid(guid) {
        return Err(format!("not a microphone id: {guid}"));
    }
    if !filter_supported() {
        return Err("kyrr's noise filter does not run on ARM processors yet".into());
    }
    if !is_installed() {
        return Err("kyrr's audio filter is not installed".into());
    }
    if Key::open(HKEY_LOCAL_MACHINE, &format!(r"{CAPTURE_KEY}\{guid}")).is_none() {
        return Err(format!("no such microphone: {guid}"));
    }
    clear_status();
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
/// A microphone Windows no longer knows is only forgotten: no keys are
/// created for it.
pub fn detach(guid: &str) -> Result<(), String> {
    if !is_endpoint_guid(guid) {
        return Err(format!("not a microphone id: {guid}"));
    }
    let Some(record) = Key::open(HKEY_LOCAL_MACHINE, &format!(r"{REG_ENDPOINTS}\{guid}")) else {
        return Ok(());
    };
    clear_status();
    let base = format!(r"{CAPTURE_KEY}\{guid}");
    if Key::open(HKEY_LOCAL_MACHINE, &format!(r"{base}\FxProperties")).is_none() {
        drop(record);
        return delete_tree(HKEY_LOCAL_MACHINE, &format!(r"{REG_ENDPOINTS}\{guid}"));
    }
    enable_backup_restore()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows names the real processor here, also to emulated x64 programs.
    #[test]
    fn filter_supported_except_on_arm() {
        let arm = std::env::var("PROCESSOR_IDENTIFIER").is_ok_and(|p| p.to_ascii_uppercase().contains("ARM"));
        assert_eq!(filter_supported(), !arm);
    }

    #[test]
    fn endpoint_guids_only() {
        assert!(is_endpoint_guid("{f6416a4b-9da5-464c-826e-b821adca8e78}"));
        assert!(is_endpoint_guid("{F6416A4B-9DA5-464C-826E-B821ADCA8E78}"));
        assert!(!is_endpoint_guid("f6416a4b-9da5-464c-826e-b821adca8e78"));
        assert!(!is_endpoint_guid("{f6416a4b-9da5-464c-826e-b821adca8e7}"));
        assert!(!is_endpoint_guid(r"{f6416a4b-9da5-464c-826e-b821adca8e78}\..\x"));
        assert!(!is_endpoint_guid("{zz416a4b-9da5-464c-826e-b821adca8e78}"));
    }
}
