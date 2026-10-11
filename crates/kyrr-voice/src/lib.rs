//! kyrr-voice: shapes the voice once the noise is gone. The chain follows
//! Omavoice DPDFNet: a low cut before the noise filter, then the sound
//! (three EQ bands), a de-esser, a compressor and a limiter. kyrr adds pop
//! protection next to the low cut and an automatic level before the
//! de-esser, so the de-esser and compressor always see a voice at the level
//! their thresholds were made for.
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

/// Pop protection: a P or B blows air at the capsule, a thump with most of
/// its energy below 60 Hz, where a voice has next to nothing (even a deep
/// one starts around 80 Hz). While the band below `POP_SUB_HZ` peaks above
/// the voice's bass (`POP_BASS_LOW_HZ` to `POP_BASS_HIGH_HZ`), everything
/// below `POP_SPLIT_HZ` is turned down, `POP_SLOPE` dB for every dB it is
/// over. In speech that band stays about 20 dB lower (LibriSpeech and MLS,
/// also with the bass boost of a close mic); what crosses it there are
/// thumps, bumps and breaths in the pauses. Measured against the voice, not
/// a level, it works the same for quiet and loud microphones.
///
/// It listens to the raw microphone before the noise filter and turns down
/// after it: the filter's delay lets the cut start `POP_LEAD_MS` before the
/// thump arrives, without adding any delay of its own.
const POP_SUB_HZ: f64 = 60.0;
const POP_BASS_LOW_HZ: f64 = 100.0;
const POP_BASS_HIGH_HZ: f64 = 400.0;
const POP_SPLIT_HZ: f64 = 120.0;
const POP_SLOPE: f64 = 2.0;
const POP_LEAD_MS: f64 = 10.0;
/// At most this much off the bass, so a pop softens instead of leaving a hole.
const POP_MAX_CUT_DB: f64 = 30.0;
/// Below this the sub band is too quiet to thump (peak, dBFS).
const POP_FLOOR_DB: f64 = -50.0;
/// Peak followers of the two bands. The bass holds longer so a pop right
/// after a vowel is still measured against that vowel.
const POP_SUB_RELEASE_MS: f64 = 15.0;
const POP_BASS_RELEASE_MS: f64 = 60.0;
const POP_ATTACK_MS: f64 = 0.5;
const POP_RELEASE_MS: f64 = 80.0;
/// The steady level of the sub band rises slowly and falls fast, so a pop
/// (tens of milliseconds) barely moves it while a fan or traffic sets it.
const POP_RUMBLE_RISE_MS: f64 = 500.0;
const POP_RUMBLE_FALL_MS: f64 = 50.0;
const POP_RUMBLE_MARGIN_DB: f64 = 6.0;
/// Room for one block between `pre` and `post` beyond the filter's delay
/// (frames); audio engine blocks are 10 ms, a few thousand frames at most.
const POP_RING_SLACK: usize = 16384;

/// Automatic level: measures the voice in 10 ms frames, only while
/// someone speaks, and moves the gain slowly towards `LEVEL_TARGET_DB`.
/// Pauses, breaths and short clicks leave the gain alone.
const LEVEL_FRAME_MS: f64 = 10.0;
/// Where speech is brought to: mean power of the spoken frames (dBFS).
const LEVEL_TARGET_DB: f64 = -23.0;
const LEVEL_MAX_BOOST_DB: f64 = 24.0;
const LEVEL_MAX_CUT_DB: f64 = 12.0;
/// A frame is speech when it is this far above the noise floor and above
/// `LEVEL_SILENCE_DB`; it counts once `LEVEL_MIN_RUN` of them follow in a
/// row (a click or a knock is shorter).
const LEVEL_ABOVE_FLOOR_DB: f64 = 15.0;
const LEVEL_SILENCE_DB: f64 = -65.0;
const LEVEL_MIN_RUN: usize = 5;
/// The noise floor is the quietest frame of the last `LEVEL_FLOOR_PARTS`
/// stretches of `LEVEL_FLOOR_PART_MS` (minimum statistics): steady noise
/// is never speech, and after a switch to a noisier room the floor catches
/// up within two seconds. Frames below `LEVEL_DIGITAL_SILENCE_DB` (a muted
/// or starting stream) say nothing about the room and are skipped; after
/// one stretch of them the floor starts over.
const LEVEL_FLOOR_PARTS: usize = 8;
const LEVEL_FLOOR_PART_MS: f64 = 250.0;
const LEVEL_DIGITAL_SILENCE_DB: f64 = -100.0;
/// The speech level averages over about this much speech; the gain waits
/// for `LEVEL_FIRST_MS` of speech before it moves the first time.
const LEVEL_AVERAGE_MS: f64 = 3000.0;
const LEVEL_FIRST_MS: f64 = 500.0;
/// How fast the gain may move: up gently, down quicker.
const LEVEL_UP_DB_PER_S: f64 = 5.0;
const LEVEL_DOWN_DB_PER_S: f64 = 10.0;
/// Smooths the gain between frames and when the switch flips.
const LEVEL_SMOOTH_MS: f64 = 100.0;

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
    /// Pop protection: the sub band (4th-order low pass), the voice's bass
    /// (4th-order high and low pass) and the 4th-order high pass that
    /// splits off what gets turned down.
    pop_sub: [Biquad; 2],
    pop_bass: [Biquad; 4],
    pop_high: [Biquad; 2],
}

/// One-pole smoothing coefficient for a time constant.
fn smoothing(rate: f64, ms: f64) -> f64 {
    (-1.0 / (ms / 1000.0 * rate)).exp()
}

fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// The automatic level's state. Lives apart from the filters: a reset of
/// the stream keeps what it learned about the voice.
#[derive(Clone, Copy, Debug)]
struct Level {
    frame_len: usize,
    /// Power summed over the frame so far, and its length.
    sum: f64,
    filled: usize,
    /// Quietest frame of each finished stretch, and of the one in progress.
    parts: [f64; LEVEL_FLOOR_PARTS],
    part_at: usize,
    part_min: f64,
    part_len: usize,
    part_frames: usize,
    /// Frames of digital silence in a row.
    silent: usize,
    /// Speech frames of the current run that do not count yet.
    run: [f64; LEVEL_MIN_RUN],
    run_len: usize,
    /// Mean power of speech and how many frames went into it.
    speech: f64,
    heard: usize,
    average_frames: f64,
    first_frames: usize,
    up_db: f64,
    down_db: f64,
    gain_db: f64,
    /// The gain applied right now (linear), smoothed per sample.
    applied: f64,
    smooth: f64,
}

impl Level {
    fn new(rate: f64) -> Level {
        let frame_len = (rate * LEVEL_FRAME_MS / 1000.0).round().max(1.0) as usize;
        let frames_per_s = rate / frame_len as f64;
        Level {
            frame_len,
            sum: 0.0,
            filled: 0,
            parts: [f64::INFINITY; LEVEL_FLOOR_PARTS],
            part_at: 0,
            part_min: f64::INFINITY,
            part_len: 0,
            part_frames: (LEVEL_FLOOR_PART_MS / 1000.0 * frames_per_s).round().max(1.0) as usize,
            silent: 0,
            run: [0.0; LEVEL_MIN_RUN],
            run_len: 0,
            speech: 0.0,
            heard: 0,
            average_frames: (LEVEL_AVERAGE_MS / 1000.0 * frames_per_s).max(1.0),
            first_frames: (LEVEL_FIRST_MS / 1000.0 * frames_per_s).ceil() as usize,
            up_db: LEVEL_UP_DB_PER_S / frames_per_s,
            down_db: LEVEL_DOWN_DB_PER_S / frames_per_s,
            gain_db: 0.0,
            applied: 1.0,
            smooth: smoothing(rate, LEVEL_SMOOTH_MS),
        }
    }

    /// Starts from a gain learned before, as if the voice had been heard at
    /// the level that gain fits.
    fn resume(&mut self, gain_db: f64) {
        if !gain_db.is_finite() {
            return;
        }
        self.gain_db = gain_db.clamp(-LEVEL_MAX_CUT_DB, LEVEL_MAX_BOOST_DB);
        self.speech = 10f64.powf((LEVEL_TARGET_DB - self.gain_db) / 10.0);
        self.heard = self.first_frames;
    }

    /// Takes the power of one sample frame (mean over channels).
    #[inline]
    fn feed(&mut self, power: f64) {
        self.sum += power;
        self.filled += 1;
        if self.filled == self.frame_len {
            let p = self.sum / self.frame_len as f64;
            self.sum = 0.0;
            self.filled = 0;
            self.frame(p);
        }
    }

    fn frame(&mut self, p: f64) {
        let db = 10.0 * (p + 1e-20).log10();
        if db < LEVEL_DIGITAL_SILENCE_DB {
            self.run_len = 0;
            // A whole stretch of it (muted, or a new stream): forget the
            // room, whatever comes next sets the floor afresh.
            self.silent += 1;
            if self.silent == self.part_frames {
                self.parts = [f64::INFINITY; LEVEL_FLOOR_PARTS];
                self.part_min = f64::INFINITY;
                self.part_len = 0;
            }
            return;
        }
        self.silent = 0;
        self.part_min = self.part_min.min(db);
        self.part_len += 1;
        if self.part_len == self.part_frames {
            self.parts[self.part_at] = self.part_min;
            self.part_at = (self.part_at + 1) % LEVEL_FLOOR_PARTS;
            self.part_min = f64::INFINITY;
            self.part_len = 0;
        }
        let floor_db = self.parts.iter().fold(self.part_min, |m, &v| m.min(v));
        let speech = db > floor_db + LEVEL_ABOVE_FLOOR_DB && db > LEVEL_SILENCE_DB;
        if !speech {
            self.run_len = 0;
            return;
        }
        if self.run_len < LEVEL_MIN_RUN {
            self.run[self.run_len] = p;
            self.run_len += 1;
            if self.run_len == LEVEL_MIN_RUN {
                for i in 0..LEVEL_MIN_RUN {
                    self.learn(self.run[i]);
                }
            }
        } else {
            self.learn(p);
        }
    }

    /// One frame of speech: update its level and move the gain a step.
    fn learn(&mut self, p: f64) {
        self.heard = self.heard.saturating_add(1);
        // A plain mean at first, then a moving one.
        let w = (1.0 / self.heard as f64).max(1.0 / self.average_frames);
        self.speech += w * (p - self.speech);
        if self.heard < self.first_frames {
            return;
        }
        let level_db = 10.0 * (self.speech + 1e-20).log10();
        let want = (LEVEL_TARGET_DB - level_db).clamp(-LEVEL_MAX_CUT_DB, LEVEL_MAX_BOOST_DB);
        self.gain_db += (want - self.gain_db).clamp(-self.down_db, self.up_db);
    }

    /// Forgets the frame in progress (the stream jumped).
    fn reset(&mut self) {
        self.sum = 0.0;
        self.filled = 0;
        self.run_len = 0;
    }
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
    pop_sub: Coeffs,
    pop_bass: [Coeffs; 2],
    pop_high: Coeffs,
    pop: bool,
    pop_min_gain: f64,
    pop_floor: f64,
    pop_sub_release: f64,
    pop_rumble_rise: f64,
    pop_rumble_fall: f64,
    pop_rumble_margin: f64,
    pop_bass_release: f64,
    pop_attack: f64,
    pop_release: f64,
    auto_level: bool,
    comp: Option<CompressorSettings>,
    comp_rms: f64,
    comp_attack: f64,
    comp_release: f64,
    limit_release: f64,

    state: Vec<Channel>,
    deess_env: f64,
    pop_sub_env: f64,
    /// The sub band's steady level.
    pop_rumble: f64,
    pop_bass_env: f64,
    pop_gain: f64,
    /// The cut each frame asks for (dB), from `pre` to `post`: written
    /// `pop_delay` frames ahead of the audio that reaches `post`.
    pop_ring: Vec<f32>,
    pop_written: u64,
    pop_read: u64,
    pop_delay: usize,
    pop_lead: usize,
    level: Level,
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
            pop_sub: Coeffs::low_pass(rate, POP_SUB_HZ, BUTTERWORTH_Q),
            pop_bass: [
                Coeffs::high_pass(rate, POP_BASS_LOW_HZ, BUTTERWORTH_Q),
                Coeffs::low_pass(rate, POP_BASS_HIGH_HZ, BUTTERWORTH_Q),
            ],
            pop_high: Coeffs::high_pass(rate, POP_SPLIT_HZ, BUTTERWORTH_Q),
            pop: false,
            pop_min_gain: db_to_gain(-POP_MAX_CUT_DB),
            pop_floor: db_to_gain(POP_FLOOR_DB),
            pop_sub_release: smoothing(rate, POP_SUB_RELEASE_MS),
            pop_rumble_rise: smoothing(rate, POP_RUMBLE_RISE_MS),
            pop_rumble_fall: smoothing(rate, POP_RUMBLE_FALL_MS),
            pop_rumble_margin: db_to_gain(POP_RUMBLE_MARGIN_DB),
            pop_bass_release: smoothing(rate, POP_BASS_RELEASE_MS),
            pop_attack: smoothing(rate, POP_ATTACK_MS),
            pop_release: smoothing(rate, POP_RELEASE_MS),
            auto_level: false,
            comp: None,
            comp_rms: smoothing(rate, COMP_RMS_MS),
            comp_attack: 0.0,
            comp_release: 0.0,
            limit_release: smoothing(rate, LIMIT_RELEASE_MS),
            state: vec![Channel::default(); channels.max(1)],
            deess_env: 0.0,
            pop_sub_env: 0.0,
            pop_rumble: 0.0,
            pop_bass_env: 0.0,
            pop_gain: 1.0,
            pop_ring: vec![1.0; POP_RING_SLACK],
            pop_written: 0,
            pop_read: 0,
            pop_delay: 0,
            pop_lead: 0,
            level: Level::new(rate),
            comp_power: 0.0,
            comp_gain_db: 0.0,
            limit_gain: 1.0,
        }
    }

    /// Whether anything runs: the shaping of the voice, the automatic
    /// level, or both.
    pub fn enabled(&self) -> bool {
        self.settings.is_some_and(|s| s.enabled || s.auto_level)
    }

    /// Takes new settings; cheap when nothing changed, so it can be called
    /// before every block. Switching on starts from a clean state. With the
    /// shaping off and the automatic level on, every shaping stage passes
    /// the audio through unchanged; the limiter still guards the level.
    pub fn set(&mut self, s: &VoiceSettings) {
        if self.settings == Some(*s) {
            return;
        }
        if (s.enabled || s.auto_level) && !self.enabled() {
            self.reset();
        }
        let shaping = s.enabled;
        let shape = shape(s);
        let rate = self.rate;
        self.low_cut = (shaping && s.low_cut).then(|| Coeffs::high_pass(rate, shape.low_cut_hz, BUTTERWORTH_Q));
        let band = |c: Coeffs| if shaping { c } else { Coeffs::IDENTITY };
        self.body = band(Coeffs::peak(rate, BODY_HZ, BODY_Q, shape.body_db));
        self.presence = band(Coeffs::peak(rate, PRESENCE_HZ, PRESENCE_Q, shape.presence_db));
        self.air = band(Coeffs::high_shelf(rate, AIR_HZ, shape.air_db));
        self.deess = deesser(s.deesser)
            .filter(|_| shaping)
            .map(|(db, ratio)| (db_to_gain(db), db, 1.0 - 1.0 / ratio));
        self.pop = shaping && s.pops;
        self.auto_level = s.auto_level;
        self.comp = compressor(s.compressor).filter(|_| shaping);
        if let Some(c) = self.comp {
            self.comp_attack = smoothing(rate, c.attack_ms);
            self.comp_release = smoothing(rate, c.release_ms);
        }
        self.settings = Some(*s);
    }

    /// The automatic level's gain in dB, to carry over to the next stream
    /// with [`Voice::resume_level`].
    pub fn level_db(&self) -> f64 {
        self.level.gain_db
    }

    /// Starts the automatic level from a gain an earlier stream learned, so
    /// a new call does not begin at the microphone's raw level.
    pub fn resume_level(&mut self, gain_db: f64) {
        self.level.resume(gain_db);
    }

    /// Tells how many frames the noise filter between [`Voice::pre`] and
    /// [`Voice::post`] delays the audio, so pop protection can use that
    /// time to look ahead, and the largest block. Allocates; call it when
    /// the stream starts, not on the audio thread.
    pub fn set_delay(&mut self, frames: usize, max_block: usize) {
        self.pop_delay = frames;
        self.pop_lead = frames.min((self.rate * POP_LEAD_MS / 1000.0).round() as usize);
        self.pop_ring = vec![1.0; frames + max_block.max(POP_RING_SLACK)];
        self.pop_written = 0;
        self.pop_read = 0;
    }

    /// Clears all filter and dynamics state. The automatic level keeps
    /// what it learned about the voice.
    pub fn reset(&mut self) {
        self.state.fill(Channel::default());
        self.deess_env = 0.0;
        self.pop_sub_env = 0.0;
        self.pop_rumble = 0.0;
        self.pop_bass_env = 0.0;
        self.pop_gain = 1.0;
        self.pop_ring.fill(1.0);
        self.pop_written = 0;
        self.pop_read = 0;
        self.level.reset();
        self.comp_power = 0.0;
        self.comp_gain_db = 0.0;
        self.limit_gain = 1.0;
    }

    /// The low cut, before the noise filter: rumble would only cost the
    /// noise filter effort. Pop protection listens here, on the raw
    /// microphone, and turns down in [`Voice::post`].
    pub fn pre(&mut self, buf: &mut [f32]) {
        if !self.enabled() {
            return;
        }
        let ch = self.channels;
        let mut low_cut_buf = [0f64; 8];
        let n = ch.min(low_cut_buf.len());
        for frame in buf.chunks_exact_mut(ch) {
            // Pop protection listens to the raw microphone, so it works the
            // same with the low cut off. The filters always run, so
            // switching it on has no start-up.
            let mut sub_peak = 0f64;
            let mut bass_peak = 0f64;
            for ((x, st), out) in frame.iter().zip(self.state.iter_mut()).zip(low_cut_buf.iter_mut()) {
                let v = if x.is_finite() { *x as f64 } else { 0.0 };
                let sub = st.pop_sub[0].run(&self.pop_sub, v);
                sub_peak = sub_peak.max(st.pop_sub[1].run(&self.pop_sub, sub).abs());
                let mut bass = v;
                for (i, s) in st.pop_bass.iter_mut().enumerate() {
                    bass = s.run(&self.pop_bass[i / 2], bass);
                }
                bass_peak = bass_peak.max(bass.abs());
                *out = match &self.low_cut {
                    Some(c) => st.low_cut.run(c, v),
                    None => v,
                };
            }
            let attack = self.pop_attack;
            let follow = |env: f64, peak: f64, release: f64| {
                let a = if peak > env { attack } else { release };
                a * env + (1.0 - a) * peak
            };
            self.pop_sub_env = follow(self.pop_sub_env, sub_peak, self.pop_sub_release);
            self.pop_bass_env = follow(self.pop_bass_env, bass_peak, self.pop_bass_release);
            // Steady rumble (a fan, traffic) is not a pop: the sub band has
            // to jump `POP_RUMBLE_MARGIN` above its own steady level too.
            let a = if self.pop_sub_env > self.pop_rumble {
                self.pop_rumble_rise
            } else {
                self.pop_rumble_fall
            };
            self.pop_rumble = a * self.pop_rumble + (1.0 - a) * self.pop_sub_env;
            let reference = self.pop_bass_env.max(self.pop_rumble * self.pop_rumble_margin);
            let wanted = if self.pop_sub_env > self.pop_floor && self.pop_sub_env > reference {
                let over_db = 20.0 * (self.pop_sub_env / reference.max(1e-12)).log10();
                db_to_gain(-over_db * POP_SLOPE).max(self.pop_min_gain)
            } else {
                1.0
            };
            let len = self.pop_ring.len();
            self.pop_ring[(self.pop_written % len as u64) as usize] = wanted as f32;
            self.pop_written += 1;

            for (x, v) in frame.iter_mut().zip(low_cut_buf.iter()) {
                *x = *v as f32;
            }
            // Channels beyond the shaped ones get the low cut only.
            if let Some(c) = &self.low_cut {
                for (x, st) in frame.iter_mut().zip(self.state.iter_mut()).skip(n) {
                    let v = if x.is_finite() { *x as f64 } else { 0.0 };
                    *x = st.low_cut.run(c, v) as f32;
                }
            }
        }
    }

    /// The pop cut `pre` asked for, for the frame reaching `post` now,
    /// looking `pop_lead` frames ahead. Before anything was written (or long
    /// after), nothing is cut.
    fn pop_wanted(&mut self) -> f64 {
        let at = self.pop_read as i64 + self.pop_lead as i64 - self.pop_delay as i64;
        self.pop_read += 1;
        let len = self.pop_ring.len() as i64;
        if at < 0 || at >= self.pop_written as i64 || at < self.pop_written as i64 - len {
            return 1.0;
        }
        self.pop_ring[(at % len) as usize] as f64
    }

    /// Sound, pop protection, automatic level, de-esser, compressor and
    /// limiter, after the noise filter.
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

            // Pop protection: below the split is the signal minus its high
            // pass, so at gain 1 the sum is exactly the input. The split
            // always runs, so switching on has no start-up.
            let asked = self.pop_wanted();
            let wanted = if self.pop { asked } else { 1.0 };
            let a = if wanted < self.pop_gain {
                self.pop_attack
            } else {
                self.pop_release
            };
            self.pop_gain = a * self.pop_gain + (1.0 - a) * wanted;
            let cut = 1.0 - self.pop_gain;

            // The sound.
            for ((out, x), st) in y.iter_mut().zip(frame.iter()).zip(self.state.iter_mut()) {
                let mut v = if x.is_finite() { *x as f64 } else { 0.0 };
                let high = st.pop_high[0].run(&self.pop_high, v);
                let high = st.pop_high[1].run(&self.pop_high, high);
                v -= cut * (v - high);
                v = st.body.run(&self.body, v);
                v = st.presence.run(&self.presence, v);
                v = st.air.run(&self.air, v);
                *out = v;
            }

            // Automatic level: measured before its own gain, so it learns
            // the voice as it comes in.
            if self.auto_level {
                self.level.feed(y.iter().map(|v| v * v).sum::<f64>() / n as f64);
            }
            let want = if self.auto_level {
                db_to_gain(self.level.gain_db)
            } else {
                1.0
            };
            self.level.applied = self.level.smooth * self.level.applied + (1.0 - self.level.smooth) * want;
            let level = self.level.applied;
            if level != 1.0 {
                for v in y.iter_mut() {
                    *v *= level;
                }
            }

            let mut key_peak = 0f64;
            if self.deess.is_some() {
                for (v, st) in y.iter().zip(self.state.iter_mut()) {
                    key_peak = key_peak.max(st.deess_key.run(&self.deess_key, *v).abs());
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
                *x = (v * level * gain).clamp(-1.0, 1.0) as f32;
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
            pops: false,
            auto_level: false,
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

    /// Runs `pre`, a pure delay standing in for the noise filter, and
    /// `post`, in 10 ms blocks; the result is aligned with the input.
    fn through(voice: &mut Voice, input: &[f32], delay: usize) -> Vec<f32> {
        voice.set_delay(delay, 480);
        let mut line = std::collections::VecDeque::from(vec![0f32; delay]);
        let mut out = input.to_vec();
        for block in out.chunks_mut(480) {
            voice.pre(block);
            for x in block.iter_mut() {
                line.push_back(*x);
                *x = line.pop_front().unwrap();
            }
            voice.post(block);
        }
        out.drain(..delay);
        out
    }

    /// A voiced sound: 120 Hz with its harmonics up to 3 kHz, falling off
    /// like a voice, in syllables that swell and fade four times a second,
    /// over a quiet room (−75 dBFS).
    fn voice_like(amplitude: f64, seconds: f64) -> Vec<f32> {
        let frames = (seconds * RATE as f64) as usize;
        let mut seed = 3u32;
        (0..frames)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                let phase = (t * 4.0).fract() / 0.6;
                let swell = if phase < 1.0 {
                    (std::f64::consts::PI * phase).sin().powi(2)
                } else {
                    0.0
                };
                let v: f64 = (1..=25)
                    .map(|k| (2.0 * std::f64::consts::PI * 120.0 * k as f64 * t).sin() / k as f64)
                    .sum();
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let room = 0.0003 * (seed as f64 / u32::MAX as f64 * 2.0 - 1.0);
                (amplitude * swell * v / 2.0 + room) as f32
            })
            .collect()
    }

    /// Bass below 120 Hz in dB, over a stretch.
    fn bass_db(x: &[f32]) -> f64 {
        let c = Coeffs::low_pass(RATE as f64, 120.0, BUTTERWORTH_Q);
        let (mut a, mut b) = (Biquad::default(), Biquad::default());
        let y: Vec<f32> = x.iter().map(|&v| b.run(&c, a.run(&c, v as f64)) as f32).collect();
        rms_db(&y)
    }

    #[test]
    fn pop_protection_leaves_a_voice_alone() {
        let dry = voice_like(0.3, 3.0);
        let mut off = Voice::new(RATE, 1);
        off.set(&settings(|_| {}));
        let mut on = Voice::new(RATE, 1);
        on.set(&settings(|s| s.pops = true));
        let (a, b) = (through(&mut off, &dry, 960), through(&mut on, &dry, 960));
        assert_eq!(a, b, "a voice is never cut");
    }

    #[test]
    fn pop_protection_takes_out_the_thump() {
        // A voice, and halfway a pressure pulse as loud as the voice's peak.
        let mut dry = voice_like(0.3, 2.0);
        let at = RATE as usize;
        let sigma = 0.008;
        for i in 0..(8.0 * sigma * RATE as f64) as usize {
            let t = i as f64 / RATE as f64 - 4.0 * sigma;
            dry[at + i] += (-0.6 * t / sigma * (-(t * t) / (2.0 * sigma * sigma)).exp() / 0.6065) as f32;
        }
        let window = at..at + 4800;
        for delay in [0, 960] {
            let mut off = Voice::new(RATE, 1);
            off.set(&settings(|s| s.low_cut = true));
            let mut on = Voice::new(RATE, 1);
            on.set(&settings(|s| {
                s.low_cut = true;
                s.pops = true;
            }));
            let (a, b) = (through(&mut off, &dry, delay), through(&mut on, &dry, delay));
            let cut = bass_db(&b[window.clone()]) - bass_db(&a[window.clone()]);
            // With the noise filter's delay to look ahead it cuts deeper.
            let needed = if delay == 0 { -4.0 } else { -12.0 };
            assert!(cut < needed, "delay {delay}: bass {cut:.1} dB");
        }
    }

    #[test]
    fn automatic_level_brings_quiet_and_loud_mics_to_the_target() {
        let mut levels = Vec::new();
        for amplitude in [0.02, 0.05, 0.5] {
            let mut voice = Voice::new(RATE, 1);
            voice.set(&settings(|s| s.auto_level = true));
            let dry = voice_like(amplitude, 20.0);
            let wet = through(&mut voice, &dry, 0);
            // Mean power of the 10 ms frames within 30 dB of the loudest,
            // over the last five seconds.
            let frames: Vec<f64> = wet[15 * RATE as usize..]
                .chunks(480)
                .map(|c| 10f64.powf(rms_db(c) / 10.0))
                .collect();
            let loudest = frames.iter().cloned().fold(0.0, f64::max);
            let loud: Vec<f64> = frames.into_iter().filter(|p| *p > loudest / 1000.0).collect();
            levels.push(10.0 * (loud.iter().sum::<f64>() / loud.len() as f64).log10());
        }
        // 28 dB apart going in, the same coming out, close to the target.
        for level in &levels {
            assert!((level - levels[0]).abs() < 1.5, "{levels:.1?}");
            assert!((level - LEVEL_TARGET_DB).abs() < 3.0, "{levels:.1?}");
        }
    }

    #[test]
    fn automatic_level_ignores_steady_noise_and_clicks() {
        let mut seed = 1u32;
        let mut noise = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        for amplitude in [0.0005, 0.005, 0.03] {
            let mut voice = Voice::new(RATE, 1);
            voice.set(&settings(|s| s.auto_level = true));
            let dry: Vec<f32> = (0..30 * RATE as usize).map(|_| amplitude * noise()).collect();
            through(&mut voice, &dry, 960);
            assert_eq!(voice.level_db(), 0.0, "noise at {amplitude}");
        }
        // 15 ms clicks every 300 ms over near silence.
        let dry: Vec<f32> = (0..30 * RATE as usize)
            .map(|i| {
                if i % 14_400 < 720 {
                    0.3 * noise()
                } else {
                    0.00003 * noise()
                }
            })
            .collect();
        let mut voice = Voice::new(RATE, 1);
        voice.set(&settings(|s| s.auto_level = true));
        through(&mut voice, &dry, 0);
        assert_eq!(voice.level_db(), 0.0, "clicks");
    }

    #[test]
    fn automatic_level_holds_through_pauses_and_carries_over() {
        let mut voice = Voice::new(RATE, 1);
        voice.set(&settings(|s| s.auto_level = true));
        through(&mut voice, &voice_like(0.02, 15.0), 0);
        let learned = voice.level_db();
        assert!(learned > 10.0, "learned {learned:.1} dB");
        through(&mut voice, &vec![0.0; 10 * RATE as usize], 0);
        assert_eq!(voice.level_db(), learned, "silence moves nothing");

        // A new stream starts where the last one stopped.
        let mut next = Voice::new(RATE, 1);
        next.set(&settings(|s| s.auto_level = true));
        next.resume_level(learned);
        let wet = through(&mut next, &voice_like(0.02, 1.0), 0);
        let first: Vec<f32> = wet[RATE as usize / 2..]
            .iter()
            .copied()
            .filter(|v| v.abs() > 0.001)
            .collect();
        assert!(
            (rms_db(&first) - LEVEL_TARGET_DB).abs() < 3.0,
            "{:.1} dBFS",
            rms_db(&first)
        );
    }

    #[test]
    fn automatic_level_works_without_the_shaping() {
        // Shaping off with loud choices left in it: only the level may act.
        let s = VoiceSettings {
            enabled: false,
            auto_level: true,
            look: Look::Custom,
            body: 60,
            low_cut: true,
            pops: true,
            deesser: Amount::Strong,
            compressor: Amount::Strong,
            ..VoiceSettings::default()
        };
        let mut voice = Voice::new(RATE, 1);
        voice.set(&s);
        let dry = voice_like(0.05, 15.0);
        let wet = through(&mut voice, &dry, 960);
        assert!(voice.level_db() > 5.0, "learned {:.1} dB", voice.level_db());
        // The last second is the input times one gain, nothing else.
        let ratios: Vec<f64> = dry[14 * RATE as usize..]
            .iter()
            .zip(&wet[14 * RATE as usize..])
            .filter(|(d, _)| d.abs() > 0.01)
            .map(|(d, w)| (*w / *d) as f64)
            .collect();
        let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
        let spread = ratios.iter().fold(0f64, |m, r| m.max((r - mean).abs())) / mean;
        assert!(spread < 0.02, "gain varies by {:.1} %", spread * 100.0);
    }
}
