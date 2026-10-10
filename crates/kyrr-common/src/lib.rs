//! Identifiers and settings shared by kyrr's app, installer and the audio
//! filter running inside the Windows audio engine. Plain data only, so every
//! part can use it without pulling in platform code.

/// COM class of kyrr's audio processing object.
pub const APO_CLSID: u128 = 0xfce4df8f_fc7f_4164_be4a_c3e4a078cfb2;

/// File name of the APO DLL in the install folder.
pub const APO_DLL: &str = "kyrr_apo.dll";

/// Shown in Windows' list of audio effects.
pub const APO_NAME: &str = "kyrr voice filter";

/// Registry key below HKEY_LOCAL_MACHINE that holds everything kyrr writes.
/// The installer grants users write access to `Filter` and the audio service
/// write access to `Status`.
pub const REG_ROOT: &str = r"SOFTWARE\kyrr";

/// Filter settings, written by the app, read live by the APO.
pub const REG_FILTER: &str = r"SOFTWARE\kyrr\Filter";

/// What the APO reports back: state, engine, stream format.
pub const REG_STATUS: &str = r"SOFTWARE\kyrr\Status";

/// One subkey per microphone kyrr is attached to (named by endpoint GUID),
/// holding the driver's original effect classes so they keep running and can
/// be restored on uninstall.
pub const REG_ENDPOINTS: &str = r"SOFTWARE\kyrr\Endpoints";

/// Value names below [`REG_FILTER`] (all DWORD).
pub mod filter {
    /// 1 = filter on, 0 = pass the microphone through (latency stays the same).
    pub const ENABLED: &str = "Enabled";
    /// Maximum attenuation in dB, 0..=100.
    pub const LIMIT_DB: &str = "LimitDb";
    /// 0 = quality model, 1 = light model. Applies when the stream restarts.
    pub const MODEL: &str = "Model";
}

/// Value names below [`REG_STATUS`].
pub mod status {
    /// "running", "passthrough: <reason>" or "error: <reason>".
    pub const STATE: &str = "State";
    /// "native int8" or "onnx", plus the model name.
    pub const ENGINE: &str = "Engine";
    /// For example "48000 Hz, 2 ch".
    pub const FORMAT: &str = "Format";
    /// Unix time of the last update (QWORD).
    pub const UPDATED: &str = "Updated";
}

/// Value names below an endpoint key in [`REG_ENDPOINTS`].
pub mod endpoint {
    /// CLSID string of the driver's stream effect (SFX) kyrr replaced and
    /// calls before its own processing. Absent when there was none.
    pub const CHILD_SFX: &str = "ChildSfx";
}

/// Filter settings with their defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterSettings {
    pub enabled: bool,
    pub limit_db: u32,
    pub light_model: bool,
}

impl Default for FilterSettings {
    fn default() -> Self {
        FilterSettings {
            enabled: true,
            limit_db: 100,
            light_model: false,
        }
    }
}

/// Formats a GUID given as u128 in registry form: `{XXXXXXXX-XXXX-...}`.
pub fn guid_string(guid: u128) -> String {
    let b = guid.to_be_bytes();
    let hex = |r: std::ops::Range<usize>| b[r].iter().map(|x| format!("{x:02X}")).collect::<String>();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        hex(0..4),
        hex(4..6),
        hex(6..8),
        hex(8..10),
        hex(10..16)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_formats_like_the_registry() {
        assert_eq!(guid_string(APO_CLSID), "{FCE4DF8F-FC7F-4164-BE4A-C3E4A078CFB2}");
    }
}
