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
    /// 0 = quality model, 1 = light model. Switches live, without a gap.
    pub const MODEL: &str = "Model";
    /// 1 = shape the voice (sound, S sounds, even volume), 0 = leave it.
    pub const VOICE: &str = "Voice";
    /// The sound, see [`crate::Look`].
    pub const VOICE_LOOK: &str = "VoiceLook";
    /// The custom sound's bands in tenths of a dB, signed (two's complement
    /// in the DWORD), within ±[`crate::CUSTOM_RANGE`].
    pub const VOICE_BODY: &str = "VoiceBody";
    pub const VOICE_PRESENCE: &str = "VoicePresence";
    pub const VOICE_AIR: &str = "VoiceAir";
    /// 1 = cut rumble below the voice.
    pub const VOICE_LOW_CUT: &str = "VoiceLowCut";
    /// How much S sounds are softened, see [`crate::Amount`].
    pub const VOICE_DEESSER: &str = "VoiceDeEsser";
    /// How much the volume is evened out (compressor), see [`crate::Amount`].
    pub const VOICE_COMPRESSOR: &str = "VoiceCompressor";
    /// 1 = soften the thump of P and B (pop protection).
    pub const VOICE_POPS: &str = "VoicePops";
    /// 1 = bring the voice to a steady level, whatever the mic's gain.
    pub const VOICE_AUTO_LEVEL: &str = "VoiceAutoLevel";
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
    pub voice: VoiceSettings,
}

impl Default for FilterSettings {
    fn default() -> Self {
        FilterSettings {
            enabled: true,
            // Medium: removes noise well and keeps speech natural in most
            // rooms; Strong can eat into quiet speech.
            limit_db: 48,
            light_model: false,
            voice: VoiceSettings::default(),
        }
    }
}

impl FilterSettings {
    /// The settings from their stored DWORDs (`get` looks one up by name),
    /// defaults for missing or invalid values.
    pub fn from_dwords(get: impl Fn(&str) -> Option<u32>) -> Self {
        let d = FilterSettings::default();
        let flag = |name, default| get(name).map_or(default, |v| v != 0);
        let band = |name, default| get(name).map_or(default, |v| clamp_band(v as i32));
        let amount = |name, default| get(name).and_then(Amount::from_index).unwrap_or(default);
        FilterSettings {
            enabled: flag(filter::ENABLED, d.enabled),
            limit_db: get(filter::LIMIT_DB).map_or(d.limit_db, |v| v.min(100)),
            light_model: get(filter::MODEL).map_or(d.light_model, |v| v == 1),
            voice: VoiceSettings {
                enabled: flag(filter::VOICE, d.voice.enabled),
                look: get(filter::VOICE_LOOK)
                    .and_then(Look::from_index)
                    .unwrap_or(d.voice.look),
                body: band(filter::VOICE_BODY, d.voice.body),
                presence: band(filter::VOICE_PRESENCE, d.voice.presence),
                air: band(filter::VOICE_AIR, d.voice.air),
                low_cut: flag(filter::VOICE_LOW_CUT, d.voice.low_cut),
                deesser: amount(filter::VOICE_DEESSER, d.voice.deesser),
                compressor: amount(filter::VOICE_COMPRESSOR, d.voice.compressor),
                pops: flag(filter::VOICE_POPS, d.voice.pops),
                auto_level: flag(filter::VOICE_AUTO_LEVEL, d.voice.auto_level),
            },
        }
    }

    /// How many values [`FilterSettings::to_dwords`] stores.
    pub const VALUES: usize = 13;

    /// The DWORDs to store, by value name.
    pub fn to_dwords(&self) -> [(&'static str, u32); Self::VALUES] {
        let v = &self.voice;
        [
            (filter::ENABLED, self.enabled as u32),
            (filter::LIMIT_DB, self.limit_db.min(100)),
            (filter::MODEL, self.light_model as u32),
            (filter::VOICE, v.enabled as u32),
            (filter::VOICE_LOOK, v.look as u32),
            (filter::VOICE_BODY, clamp_band(v.body) as u32),
            (filter::VOICE_PRESENCE, clamp_band(v.presence) as u32),
            (filter::VOICE_AIR, clamp_band(v.air) as u32),
            (filter::VOICE_LOW_CUT, v.low_cut as u32),
            (filter::VOICE_DEESSER, v.deesser as u32),
            (filter::VOICE_COMPRESSOR, v.compressor as u32),
            (filter::VOICE_POPS, v.pops as u32),
            (filter::VOICE_AUTO_LEVEL, v.auto_level as u32),
        ]
    }
}

/// How far each band of the custom sound goes, in tenths of a dB.
pub const CUSTOM_RANGE: i32 = 60;

fn clamp_band(tenths: i32) -> i32 {
    tenths.clamp(-CUSTOM_RANGE, CUSTOM_RANGE)
}

/// Shaping the voice after the noise is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub look: Look,
    /// The custom sound: around 250 Hz, 3 kHz and above 8 kHz, in tenths
    /// of a dB. Kept while another look is chosen.
    pub body: i32,
    pub presence: i32,
    pub air: i32,
    /// High-pass below the voice (80 Hz, 100 Hz for the bright look).
    pub low_cut: bool,
    /// Softens sharp S sounds.
    pub deesser: Amount,
    /// Evens out the volume: quiet words louder, loud ones softer.
    pub compressor: Amount,
    /// Softens the thump of P and B when air hits the capsule.
    pub pops: bool,
    /// Slowly brings the voice to one level, however loud the mic is set.
    pub auto_level: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings {
            enabled: true,
            look: Look::Neutral,
            body: 0,
            presence: 0,
            air: 0,
            low_cut: true,
            deesser: Amount::Light,
            compressor: Amount::Medium,
            pops: false,
            auto_level: true,
        }
    }
}

/// The sound of the voice, as in Omavoice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Neutral = 0,
    Warm = 1,
    Clear = 2,
    Bright = 3,
    Custom = 4,
}

impl Look {
    pub const ALL: [Look; 5] = [Look::Neutral, Look::Warm, Look::Clear, Look::Bright, Look::Custom];

    pub fn from_index(i: u32) -> Option<Look> {
        Look::ALL.get(i as usize).copied()
    }
}

/// How strongly the de-esser or the compressor works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Off = 0,
    Light = 1,
    Medium = 2,
    Strong = 3,
}

impl Amount {
    pub const ALL: [Amount; 4] = [Amount::Off, Amount::Light, Amount::Medium, Amount::Strong];

    pub fn from_index(i: u32) -> Option<Amount> {
        Amount::ALL.get(i as usize).copied()
    }
}

/// Live data the APO shares with the app through a named file mapping: the
/// microphone level and the before/after probe. The APO (audio service)
/// creates it, signed-in users may read it and ask for a probe.
pub mod monitor {
    use std::sync::atomic::AtomicU32;

    /// Name and magic change with the layout, so an app and a filter of
    /// different versions do not read each other's buffers.
    pub const NAME: &str = r"Global\kyrr-monitor-3";
    pub const MAGIC: u32 = u32::from_le_bytes(*b"kyr3");
    /// Length of the probe recording.
    pub const PROBE_SECONDS: u32 = 10;
    /// Mono samples per probe buffer: the probe at up to 96 kHz.
    pub const PROBE_CAPACITY: usize = PROBE_SECONDS as usize * 96_000;
    /// The header, then the probe before kyrr, then after kyrr (f32 each).
    pub const HEADER_SIZE: usize = 64;
    pub const SIZE: usize = HEADER_SIZE + 2 * PROBE_CAPACITY * 4;

    #[repr(C)]
    pub struct Header {
        pub magic: AtomicU32,
        /// Counts processed blocks; standing still means no app uses the
        /// microphone.
        pub heartbeat: AtomicU32,
        /// Peak level before and after kyrr, linear `f32` bits, falling off
        /// slowly so a meter polled now and then sees every word.
        pub level_in: AtomicU32,
        pub level_out: AtomicU32,
        /// The app asks for a probe by setting a new number here.
        pub probe_request: AtomicU32,
        /// The probe an APO instance has started recording.
        pub probe_claimed: AtomicU32,
        /// The probe whose samples are complete.
        pub probe_done: AtomicU32,
        pub probe_rate: AtomicU32,
        pub probe_frames: AtomicU32,
        /// What the automatic volume adds right now, dB as `f32` bits.
        pub level_gain: AtomicU32,
    }

    const _: () = assert!(std::mem::size_of::<Header>() <= HEADER_SIZE);
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
    fn settings_survive_the_registry() {
        let s = FilterSettings {
            limit_db: 48,
            voice: VoiceSettings {
                enabled: true,
                look: Look::Custom,
                body: -25,
                presence: 60,
                air: 15,
                low_cut: false,
                deesser: Amount::Strong,
                compressor: Amount::Medium,
                pops: false,
                auto_level: true,
            },
            ..FilterSettings::default()
        };
        let stored = s.to_dwords();
        let back = FilterSettings::from_dwords(|name| stored.iter().find(|(n, _)| *n == name).map(|(_, v)| *v));
        assert_eq!(back, s);
    }

    #[test]
    fn missing_or_bad_values_fall_back() {
        assert_eq!(FilterSettings::from_dwords(|_| None), FilterSettings::default());
        let bad = FilterSettings::from_dwords(|name| match name {
            filter::VOICE_LOOK | filter::VOICE_DEESSER => Some(9),
            filter::VOICE_BODY => Some(-500i32 as u32),
            _ => None,
        });
        assert_eq!(bad.voice.look, Look::Neutral);
        assert_eq!(bad.voice.deesser, Amount::Light);
        assert_eq!(bad.voice.body, -CUSTOM_RANGE);
    }

    #[test]
    fn monitor_layout_fits() {
        use monitor::*;
        assert_eq!(std::mem::align_of::<Header>(), 4);
        assert!(std::mem::size_of::<Header>() <= HEADER_SIZE && HEADER_SIZE.is_multiple_of(4));
        assert_eq!(SIZE, HEADER_SIZE + 2 * PROBE_CAPACITY * 4);
    }

    #[test]
    fn clsid_formats_like_the_registry() {
        assert_eq!(guid_string(APO_CLSID), "{FCE4DF8F-FC7F-4164-BE4A-C3E4A078CFB2}");
    }
}
