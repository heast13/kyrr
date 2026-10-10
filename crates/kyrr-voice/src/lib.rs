//! kyrr-voice: shapes the voice once the noise is gone. The chain follows
//! Omavoice DPDFNet: a low cut before the noise filter, then the sound
//! (three EQ bands), a de-esser, a compressor and a limiter.
//!
//! [`Voice`] works on interleaved frames in place. After construction it
//! never allocates and takes new settings between blocks, so it can run on
//! the real-time thread of the Windows audio engine. Every channel is
//! filtered on its own; the de-esser, compressor and limiter react to all
//! channels together, so the stereo image stays put.

use kyrr_common::{Amount, Look, VoiceSettings};

/// Band centres of the sound: body (peak), presence (peak), air (shelf).
const BODY_HZ: f64 = 250.0;
const BODY_Q: f64 = 0.9;
const PRESENCE_HZ: f64 = 3000.0;
const PRESENCE_Q: f64 = 1.0;
const AIR_HZ: f64 = 8000.0;
const LOW_CUT_HZ: f64 = 80.0;
/// The bright look also cuts a little higher.
const LOW_CUT_BRIGHT_HZ: f64 = 100.0;
const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The de-esser compresses everything above `DEESS_SPLIT_HZ` when the band
/// above `DEESS_KEY_HZ` gets loud (LSP's de-esser as Omavoice sets it up).
const DEESS_SPLIT_HZ: f64 = 4000.0;
const DEESS_KEY_HZ: f64 = 4500.0;
const DEESS_ATTACK_MS: f64 = 1.0;
const DEESS_RELEASE_MS: f64 = 40.0;

/// Compressor: RMS level over 10 ms, 6 dB soft knee.
const COMP_RMS_MS: f64 = 10.0;
const COMP_KNEE_DB: f64 = 6.0;

/// Limiter ceiling (−1 dBFS) and how fast it lets go again.
const LIMIT: f64 = 0.891;
const LIMIT_RELEASE_MS: f64 = 50.0;

/// Gains of the sound in dB and the low-cut corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub low_cut_hz: f64,
    pub body_db: f64,
    pub presence_db: f64,
    pub air_db: f64,
}

/// What a look does (the values from Omavoice).
pub fn shape(s: &VoiceSettings) -> Shape {
    let (body, presence, air) = match s.look {
        Look::Neutral => (0.0, 0.0, 0.0),
        Look::Warm => (2.0, -1.5, 0.0),
        Look::Clear => (-2.5, 2.0, 0.0),
        Look::Bright => (0.0, 1.0, 2.5),
        Look::Custom => (s.body as f64 / 10.0, s.presence as f64 / 10.0, s.air as f64 / 10.0),
    };
    Shape {
        low_cut_hz: if s.look == Look::Bright {
            LOW_CUT_BRIGHT_HZ
        } else {
            LOW_CUT_HZ
        },
        body_db: body,
        presence_db: presence,
        air_db: air,
    }
}

/// De-esser threshold (dBFS, on the key band's peak) and ratio.
fn deesser(amount: Amount) -> Option<(f64, f64)> {
    match amount {
        Amount::Off => None,
        Amount::Light => Some((-24.0, 2.0)),
        Amount::Medium => Some((-30.0, 4.0)),
        Amount::Strong => Some((-36.0, 8.0)),
    }
}

/// Compressor settings for an amount.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CompressorSettings {
    threshold_db: f64,
    ratio: f64,
    makeup_db: f64,
    attack_ms: f64,
    release_ms: f64,
}

/// Light is close to Omavoice's compressor and barely audible; Medium
/// suits calls; Strong keeps a voice at nearly one volume (streams, noisy
/// rooms). Makeup brings speech back to about its old loudness.
fn compressor(amount: Amount) -> Option<CompressorSettings> {
    let (threshold_db, ratio, makeup_db, attack_ms, release_ms) = match amount {
        Amount::Off => return None,
        Amount::Light => (-18.0, 2.0, 3.0, 15.0, 150.0),
        Amount::Medium => (-24.0, 3.0, 6.0, 10.0, 120.0),
        Amount::Strong => (-30.0, 4.0, 9.0, 5.0, 100.0),
    };
    Some(CompressorSettings {
        threshold_db,
        ratio,
        makeup_db,
        attack_ms,
        release_ms,
    })
}

/// Biquad coefficients (RBJ cookbook), normalized to a0 = 1.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coeffs {
    const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Coeffs {
        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// `(cos w0, sin w0)`, the frequency kept below Nyquist.
    fn angle(rate: f64, hz: f64) -> (f64, f64) {
        let w0 = 2.0 * std::f64::consts::PI * hz.min(rate * 0.45) / rate;
        (w0.cos(), w0.sin())
    }

    fn high_pass(rate: f64, hz: f64, q: f64) -> Coeffs {
        let (cos, sin) = Coeffs::angle(rate, hz);
        let alpha = sin / (2.0 * q);
        Coeffs::normalized(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn low_pass(rate: f64, hz: f64, q: f64) -> Coeffs {
        let (cos, sin) = Coeffs::angle(rate, hz);
        let alpha = sin / (2.0 * q);
        Coeffs::normalized(
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn peak(rate: f64, hz: f64, q: f64, db: f64) -> Coeffs {
        if db == 0.0 {
            return Coeffs::IDENTITY;
        }
        let a = 10f64.powf(db / 40.0);
        let (cos, sin) = Coeffs::angle(rate, hz);
        let alpha = sin / (2.0 * q);
        Coeffs::normalized(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        )
    }

    /// High shelf with slope 1.
    fn high_shelf(rate: f64, hz: f64, db: f64) -> Coeffs {
        if db == 0.0 {
            return Coeffs::IDENTITY;
        }
        let a = 10f64.powf(db / 40.0);
        let (cos, sin) = Coeffs::angle(rate, hz);
        let k = 2.0 * a.sqrt() * sin / 2.0 * std::f64::consts::SQRT_2;
        Coeffs::normalized(
            a * ((a + 1.0) + (a - 1.0) * cos + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - k),
            (a + 1.0) - (a - 1.0) * cos + k,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - k,
        )
    }
}

/// Transposed direct form II state.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    fn run(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// Filter state of one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    low_cut: Biquad,
    body: Biquad,
    presence: Biquad,
    air: Biquad,
    /// Two stages of the 4th-order low pass that splits off the S band.
    deess_low: [Biquad; 2],
    deess_key: Biquad,
}

/// One-pole smoothing coefficient for a time constant.
fn smoothing(rate: f64, ms: f64) -> f64 {
    (-1.0 / (ms / 1000.0 * rate)).exp()
}

fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Shapes the voice of one capture stream.
pub struct Voice {
    rate: f64,
    channels: usize,
    /// The settings the coefficients below were made for.
    settings: Option<VoiceSettings>,

    low_cut: Option<Coeffs>,
    body: Coeffs,
    presence: Coeffs,
    air: Coeffs,
    deess_low: Coeffs,
    deess_key: Coeffs,
    /// Threshold (linear), threshold (dB) and `1 - 1/ratio`.
    deess: Option<(f64, f64, f64)>,
    deess_attack: f64,
    deess_release: f64,
    comp: Option<CompressorSettings>,
    comp_rms: f64,
    comp_attack: f64,
    comp_release: f64,
    limit_release: f64,

    state: Vec<Channel>,
    deess_env: f64,
    comp_power: f64,
    comp_gain_db: f64,
    limit_gain: f64,
}

impl Voice {
    /// For interleaved frames with `channels` channels at `rate` Hz. Does
    /// nothing until [`Voice::set`] switches it on.
    pub fn new(rate: u32, channels: usize) -> Voice {
        let rate = rate.max(1) as f64;
        Voice {
            rate,
            channels: channels.max(1),
            settings: None,
            low_cut: None,
            body: Coeffs::IDENTITY,
            presence: Coeffs::IDENTITY,
            air: Coeffs::IDENTITY,
            deess_low: Coeffs::low_pass(rate, DEESS_SPLIT_HZ, BUTTERWORTH_Q),
            deess_key: Coeffs::high_pass(rate, DEESS_KEY_HZ, BUTTERWORTH_Q),
            deess: None,
            deess_attack: smoothing(rate, DEESS_ATTACK_MS),
            deess_release: smoothing(rate, DEESS_RELEASE_MS),
            comp: None,
            comp_rms: smoothing(rate, COMP_RMS_MS),
            comp_attack: 0.0,
            comp_release: 0.0,
            limit_release: smoothing(rate, LIMIT_RELEASE_MS),
            state: vec![Channel::default(); channels.max(1)],
            deess_env: 0.0,
            comp_power: 0.0,
            comp_gain_db: 0.0,
            limit_gain: 1.0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.settings.is_some_and(|s| s.enabled)
    }

    /// Takes new settings; cheap when nothing changed, so it can be called
    /// before every block. Switching on starts from a clean state.
    pub fn set(&mut self, s: &VoiceSettings) {
        if self.settings == Some(*s) {
            return;
        }
        if s.enabled && !self.enabled() {
            self.reset();
        }
        let shape = shape(s);
        let rate = self.rate;
        self.low_cut = s
            .low_cut
            .then(|| Coeffs::high_pass(rate, shape.low_cut_hz, BUTTERWORTH_Q));
        self.body = Coeffs::peak(rate, BODY_HZ, BODY_Q, shape.body_db);
        self.presence = Coeffs::peak(rate, PRESENCE_HZ, PRESENCE_Q, shape.presence_db);
        self.air = Coeffs::high_shelf(rate, AIR_HZ, shape.air_db);
        self.deess = deesser(s.deesser).map(|(db, ratio)| (db_to_gain(db), db, 1.0 - 1.0 / ratio));
        self.comp = compressor(s.compressor);
        if let Some(c) = self.comp {
            self.comp_attack = smoothing(rate, c.attack_ms);
            self.comp_release = smoothing(rate, c.release_ms);
        }
        self.settings = Some(*s);
    }

    /// Clears all filter and level state.
    pub fn reset(&mut self) {
        self.state.fill(Channel::default());
        self.deess_env = 0.0;
        self.comp_power = 0.0;
        self.comp_gain_db = 0.0;
        self.limit_gain = 1.0;
    }

    /// The low cut, before the noise filter: rumble would only cost the
    /// noise filter effort.
    pub fn pre(&mut self, buf: &mut [f32]) {
        if !self.enabled() {
            return;
        }
        let Some(c) = self.low_cut else {
            return;
        };
        for frame in buf.chunks_exact_mut(self.channels) {
            for (x, st) in frame.iter_mut().zip(self.state.iter_mut()) {
                let v = if x.is_finite() { *x as f64 } else { 0.0 };
                *x = st.low_cut.run(&c, v) as f32;
            }
        }
    }

    /// Sound, de-esser, compressor and limiter, after the noise filter.
    pub fn post(&mut self, buf: &mut [f32]) {
        if !self.enabled() {
            return;
        }
        let ch = self.channels;
        // At most 8 channels are shaped together on the stack; more than
        // that never reaches a microphone APO, the rest pass unchanged.
        let mut frame_buf = [0f64; 8];
        let n = ch.min(frame_buf.len());
        for frame in buf.chunks_exact_mut(ch) {
            let y = &mut frame_buf[..n];

            // The sound.
            let mut key_peak = 0f64;
            for ((out, x), st) in y.iter_mut().zip(frame.iter()).zip(self.state.iter_mut()) {
                let mut v = if x.is_finite() { *x as f64 } else { 0.0 };
                v = st.body.run(&self.body, v);
                v = st.presence.run(&self.presence, v);
                v = st.air.run(&self.air, v);
                *out = v;
                if self.deess.is_some() {
                    key_peak = key_peak.max(st.deess_key.run(&self.deess_key, v).abs());
                }
            }

            // De-esser: turn the band above 4 kHz down by the gain the key
            // band asks for. The band is the signal minus its low pass, so
            // at gain 1 the sum is exactly the input; the split runs always
            // so it has no start-up.
            if let Some((threshold, threshold_db, slope)) = self.deess {
                let a = if key_peak > self.deess_env {
                    self.deess_attack
                } else {
                    self.deess_release
                };
                self.deess_env = a * self.deess_env + (1.0 - a) * key_peak;
                let gain = if self.deess_env > threshold {
                    let over = 20.0 * self.deess_env.log10() - threshold_db;
                    db_to_gain(-over * slope)
                } else {
                    1.0
                };
                for (v, st) in y.iter_mut().zip(self.state.iter_mut()) {
                    let low = st.deess_low[0].run(&self.deess_low, *v);
                    let low = st.deess_low[1].run(&self.deess_low, low);
                    *v = low + gain * (*v - low);
                }
            }

            // Compressor on the RMS level of all channels.
            let mut gain = 1.0;
            if let Some(c) = self.comp {
                let power = y.iter().map(|v| v * v).sum::<f64>() / n as f64;
                self.comp_power = self.comp_rms * self.comp_power + (1.0 - self.comp_rms) * power;
                let level = 10.0 * (self.comp_power + 1e-20).log10();
                let over = level - c.threshold_db;
                let slope = 1.0 / c.ratio - 1.0;
                let target = if 2.0 * over < -COMP_KNEE_DB {
                    0.0
                } else if 2.0 * over.abs() <= COMP_KNEE_DB {
                    slope * (over + COMP_KNEE_DB / 2.0).powi(2) / (2.0 * COMP_KNEE_DB)
                } else {
                    slope * over
                };
                let a = if target < self.comp_gain_db {
                    self.comp_attack
                } else {
                    self.comp_release
                };
                self.comp_gain_db = a * self.comp_gain_db + (1.0 - a) * target;
                gain = db_to_gain(self.comp_gain_db + c.makeup_db);
            }

            // Limiter: catches peaks the compressor's attack lets through
            // and boosts of the sound, so nothing clips.
            let peak = y.iter().fold(0f64, |m, v| m.max(v.abs())) * gain;
            let needed = if peak > LIMIT { LIMIT / peak } else { 1.0 };
            if needed < self.limit_gain {
                self.limit_gain = needed;
            } else {
                self.limit_gain = self.limit_release * self.limit_gain + (1.0 - self.limit_release) * needed;
            }
            gain *= self.limit_gain;

            for (x, v) in frame.iter_mut().zip(y.iter()) {
                *x = (v * gain).clamp(-1.0, 1.0) as f32;
            }
            // Channels beyond the shaped ones keep their sound but get the
            // same level, and never NaN or clipping.
            for x in frame.iter_mut().skip(n) {
                let v = if x.is_finite() { *x as f64 } else { 0.0 };
                *x = (v * gain).clamp(-1.0, 1.0) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn settings(change: impl FnOnce(&mut VoiceSettings)) -> VoiceSettings {
        let mut s = VoiceSettings {
            enabled: true,
            look: Look::Neutral,
            low_cut: false,
            deesser: Amount::Off,
            compressor: Amount::Off,
            ..VoiceSettings::default()
        };
        change(&mut s);
        s
    }

    fn sine(hz: f64, amplitude: f64, seconds: f64, channels: usize) -> Vec<f32> {
        let frames = (seconds * RATE as f64) as usize;
        (0..frames)
            .flat_map(|i| {
                let v = (amplitude * (2.0 * std::f64::consts::PI * hz * i as f64 / RATE as f64).sin()) as f32;
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn rms_db(buf: &[f32]) -> f64 {
        let p = buf.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / buf.len() as f64;
        10.0 * p.log10()
    }

    /// Gain in dB a tone gets, measured on the second half (settled).
    fn gain_db(s: &VoiceSettings, hz: f64, amplitude: f64) -> f64 {
        let mut voice = Voice::new(RATE, 1);
        voice.set(s);
        let dry = sine(hz, amplitude, 1.0, 1);
        let mut wet = dry.clone();
        for block in wet.chunks_mut(480) {
            voice.pre(block);
            voice.post(block);
        }
        let half = dry.len() / 2;
        rms_db(&wet[half..]) - rms_db(&dry[half..])
    }

    #[test]
    fn off_leaves_audio_alone() {
        let dry = sine(1000.0, 0.5, 0.1, 2);
        let mut wet = dry.clone();
        let mut voice = Voice::new(RATE, 2);
        voice.set(&VoiceSettings {
            enabled: false,
            ..VoiceSettings::default()
        });
        voice.pre(&mut wet);
        voice.post(&mut wet);
        assert_eq!(wet, dry);
    }

    #[test]
    fn neutral_without_dynamics_is_transparent() {
        let s = settings(|_| {});
        for hz in [100.0, 1000.0, 7000.0] {
            assert!(gain_db(&s, hz, 0.3).abs() < 0.05, "{hz} Hz");
        }
    }

    #[test]
    fn low_cut_removes_rumble_and_keeps_the_voice() {
        let s = settings(|s| s.low_cut = true);
        assert!(gain_db(&s, 30.0, 0.3) < -15.0);
        assert!(gain_db(&s, 1000.0, 0.3).abs() < 0.1);
    }

    #[test]
    fn looks_lift_and_cut_their_bands() {
        let clear = settings(|s| s.look = Look::Clear);
        assert!((gain_db(&clear, 250.0, 0.1) + 2.5).abs() < 0.3);
        assert!((gain_db(&clear, 3000.0, 0.1) - 2.0).abs() < 0.3);
        let bright = settings(|s| s.look = Look::Bright);
        assert!((gain_db(&bright, 16000.0, 0.1) - 2.5).abs() < 0.3);
        let custom = settings(|s| {
            s.look = Look::Custom;
            s.body = 60;
        });
        assert!((gain_db(&custom, 250.0, 0.1) - 6.0).abs() < 0.3);
    }

    #[test]
    fn deesser_tames_s_sounds_only() {
        let mut last = 0.0;
        for amount in [Amount::Light, Amount::Medium, Amount::Strong] {
            let s = settings(|s| s.deesser = amount);
            let s_sound = gain_db(&s, 7000.0, 0.5);
            assert!(s_sound < last - 2.0, "{amount:?}: {s_sound} dB");
            last = s_sound;
            assert!(gain_db(&s, 500.0, 0.5).abs() < 0.5, "{amount:?} touches 500 Hz");
        }
    }

    #[test]
    fn compressor_evens_out_the_volume() {
        let mut last_spread = 60.0;
        for amount in [Amount::Light, Amount::Medium, Amount::Strong] {
            let s = settings(|s| s.compressor = amount);
            let loud = gain_db(&s, 1000.0, 0.5) + 20.0 * 0.5f64.log10();
            let quiet = gain_db(&s, 1000.0, 0.01) + 20.0 * 0.01f64.log10();
            let spread = loud - quiet;
            assert!(spread < last_spread - 3.0, "{amount:?}: {spread} dB apart");
            last_spread = spread;
        }
    }

    #[test]
    fn nothing_goes_over_the_ceiling() {
        let s = settings(|s| {
            s.look = Look::Custom;
            s.presence = 60;
            s.air = 60;
            s.compressor = Amount::Strong;
        });
        let mut buf = sine(3000.0, 1.0, 0.5, 2);
        buf[1000] = f32::NAN;
        buf[1001] = f32::INFINITY;
        let mut voice = Voice::new(RATE, 2);
        voice.set(&s);
        for block in buf.chunks_mut(512) {
            voice.post(block);
        }
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= LIMIT as f32 + 1e-6));
    }

    #[test]
    fn many_channels_stay_finite() {
        let mut voice = Voice::new(RATE, 10);
        voice.set(&settings(|s| s.compressor = Amount::Strong));
        let mut buf = vec![f32::NAN; 10 * 256];
        voice.post(&mut buf);
        assert!(buf.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn works_at_other_rates() {
        for rate in [16_000, 44_100, 96_000] {
            let mut voice = Voice::new(rate, 1);
            voice.set(&settings(|s| {
                s.look = Look::Bright;
                s.low_cut = true;
                s.deesser = Amount::Strong;
            }));
            let mut buf = vec![0.1f32; 4096];
            voice.pre(&mut buf);
            voice.post(&mut buf);
            assert!(buf.iter().all(|x| x.is_finite()), "{rate} Hz");
        }
    }
}
