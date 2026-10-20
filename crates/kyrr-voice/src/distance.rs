//! Distance compensation. A microphone far from the mouth hears the room
//! almost as loud as the voice: every word trails a tail of echo, the bass a
//! close microphone adds is missing, and the voice sounds hollow. The
//! automatic level then lifts the voice and the room alike.
//!
//! [`Detector`] listens for that tail on the raw microphone. Only when it
//! hears it for a while does [`Compensation`] fade in (over `BLEND_S`): it
//! turns the tail down by at most `CUT_MAX_DB`, gives back a little bass and
//! takes some boxiness out. Close to the microphone it does nothing at all,
//! the audio stays the same bit for bit. Neither adds any delay.
//!
//! The listening: when a syllable ends, a dry voice falls into the room's
//! noise within a few tens of milliseconds, by 20 dB and more. Far from the
//! microphone the room keeps ringing, and no level ever falls faster than
//! the room's echo dies away. [`Detector`] collects how far the level falls
//! within `DROP_FRAMES` frames, wherever it starts well above the noise, and
//! looks at the steepest of these falls (the `STEEP` share of them). That
//! is how Wen, Habets and Naylor estimate the reverb time blind; here it
//! also catches how much of the echo reaches the microphone, since the
//! direct sound stopping takes a close microphone's level down at once.
//! Absolute distance from one microphone is not reliable; near or far is.

use crate::{db_to_gain, smoothing, Biquad, Coeffs, BUTTERWORTH_Q};

/// The band the detector listens to: where a voice and its echo are loud,
/// above rumble and below hiss.
const DETECT_LOW_HZ: f64 = 250.0;
const DETECT_HIGH_HZ: f64 = 4000.0;
const FRAME_MS: f64 = 10.0;
/// Noise floor by minimum statistics, as the automatic level finds it.
const FLOOR_PARTS: usize = 8;
const FLOOR_PART_MS: f64 = 250.0;
const DIGITAL_SILENCE_DB: f64 = -100.0;
/// A fall is measured over 40 ms, from frames this far above the noise, so
/// a dry voice has room to show how deep it falls.
const DROP_FRAMES: usize = 4;
const CLEAR_DB: f64 = 25.0;
/// The falls of about the last 15 seconds of speech are kept; the steepest
/// 5 % of them tell near from far, looked at every `CHECK_EVERY` new ones,
/// the first time after `FIRST`.
const DROPS: usize = 1024;
const STEEP: f64 = 0.05;
const CHECK_EVERY: usize = 50;
const FIRST: usize = 300;
/// Far when the steepest falls are gentler than `FAR_DB`, near again when
/// they are steeper than `NEAR_DB`, either only after `HOLD` looks in a row,
/// so it never flutters. With synthetic rooms: a close microphone (direct
/// sound 15 dB above the echo) measures about −12 dB, half a metre in an
/// echoing room (about even) −6 dB, a dry voice −25 dB.
const FAR_DB: f64 = -7.5;
const NEAR_DB: f64 = -9.5;
const HOLD: usize = 3;

/// The compensation splits the voice into four bands at these frequencies
/// and turns each band's tail down on its own, so an S after a vowel
/// passes while the vowel's echo is damped.
const SPLIT_HZ: [f64; 3] = [300.0, 1000.0, 3000.0];
/// Level followers of the bands; the lowest band needs longer to settle.
const ENVELOPE_MS: [f64; 4] = [20.0, 10.0, 10.0, 10.0];
/// How often the bands' gains are worked out.
const TICK_MS: f64 = 1.0;
/// A band's tail is turned down once it fell `CUT_FROM_DB` below the peak
/// of its last onset, fully `CUT_RANGE_DB` further down. A rise of
/// `RISE_DB` above the lowest point since is the next onset.
const CUT_FROM_DB: f64 = 3.0;
const CUT_RANGE_DB: f64 = 8.0;
const RISE_DB: f64 = 6.0;
/// At most this much off the tail: a little room stays, completely dry
/// sounds unnatural (Dolby and Microsoft's listening test liked about 8 dB).
const CUT_MAX_DB: f64 = 8.0;
/// The cut closes in gently and lets go fast, so the next syllable starts
/// at full level.
const CUT_ATTACK_MS: f64 = 15.0;
const CUT_RELEASE_MS: f64 = 4.0;
/// The sound when far: the bass of a close microphone back, less box,
/// a little more presence.
const BASS_HZ: f64 = 150.0;
const BASS_DB: f64 = 2.5;
const BOX_HZ: f64 = 450.0;
const BOX_Q: f64 = 1.2;
const BOX_DB: f64 = -2.5;
const PRESENCE_HZ: f64 = 3500.0;
const PRESENCE_Q: f64 = 1.0;
const PRESENCE_DB: f64 = 1.5;
/// How long the compensation takes to fade in or out.
const BLEND_S: f64 = 2.0;
/// Channels shaped together, as in [`crate::Voice::post`].
const MAX_CHANNELS: usize = 8;

/// Listens to the raw microphone and tells whether it is far from the voice.
#[derive(Clone, Debug)]
pub(crate) struct Detector {
    band: [Coeffs; 2],
    filters: [Biquad; 4],
    frame_len: usize,
    sum: f64,
    filled: usize,
    parts: [f64; FLOOR_PARTS],
    part_at: usize,
    part_min: f64,
    part_len: usize,
    part_frames: usize,
    silent: usize,
    /// The last frame levels (dB), the newest at `at - 1`.
    recent: [f64; DROP_FRAMES],
    at: usize,
    /// Falls in dB (negative), a ring; `heard` counts all ever added.
    drops: [f32; DROPS],
    heard: usize,
    score: f64,
    far: bool,
    /// Looks in a row that asked for the other state.
    against: usize,
}

impl Detector {
    pub(crate) fn new(rate: f64) -> Detector {
        let frame_len = (rate * FRAME_MS / 1000.0).round().max(1.0) as usize;
        Detector {
            band: [
                Coeffs::high_pass(rate, DETECT_LOW_HZ, BUTTERWORTH_Q),
                Coeffs::low_pass(rate, DETECT_HIGH_HZ, BUTTERWORTH_Q),
            ],
            filters: [Biquad::default(); 4],
            frame_len,
            sum: 0.0,
            filled: 0,
            parts: [f64::INFINITY; FLOOR_PARTS],
            part_at: 0,
            part_min: f64::INFINITY,
            part_len: 0,
            part_frames: (FLOOR_PART_MS / FRAME_MS).round() as usize,
            silent: 0,
            recent: [f64::NEG_INFINITY; DROP_FRAMES],
            at: 0,
            drops: [0.0; DROPS],
            heard: 0,
            score: 0.0,
            far: false,
            against: 0,
        }
    }

    /// Whether the microphone sounds far from the voice.
    pub(crate) fn far(&self) -> bool {
        self.far
    }

    /// How deep the steepest falls go (dB) and how many falls were heard.
    #[cfg(test)]
    pub(crate) fn score(&self) -> (f64, usize) {
        (self.score, self.heard)
    }

    /// Takes one sample of the raw microphone (mono).
    #[inline]
    pub(crate) fn feed(&mut self, x: f64) {
        let mut v = x;
        for (i, f) in self.filters.iter_mut().enumerate() {
            v = f.run(&self.band[i / 2], v);
        }
        self.sum += v * v;
        self.filled += 1;
        if self.filled == self.frame_len {
            let p = self.sum / self.frame_len as f64;
            self.sum = 0.0;
            self.filled = 0;
            self.frame(10.0 * (p + 1e-20).log10());
        }
    }

    /// Forgets the frame in progress and the last levels (the stream
    /// jumped); what it learned about the room stays.
    pub(crate) fn reset(&mut self) {
        self.filters = [Biquad::default(); 4];
        self.sum = 0.0;
        self.filled = 0;
        self.recent = [f64::NEG_INFINITY; DROP_FRAMES];
    }

    fn frame(&mut self, db: f64) {
        if db < DIGITAL_SILENCE_DB {
            // Muted or a new stream: says nothing about the room.
            self.silent += 1;
            if self.silent == self.part_frames {
                self.parts = [f64::INFINITY; FLOOR_PARTS];
                self.part_min = f64::INFINITY;
                self.part_len = 0;
            }
            self.recent = [f64::NEG_INFINITY; DROP_FRAMES];
            return;
        }
        self.silent = 0;
        self.part_min = self.part_min.min(db);
        self.part_len += 1;
        if self.part_len == self.part_frames {
            self.parts[self.part_at] = self.part_min;
            self.part_at = (self.part_at + 1) % FLOOR_PARTS;
            self.part_min = f64::INFINITY;
            self.part_len = 0;
        }
        let floor = self.parts.iter().fold(self.part_min, |m, &v| m.min(v));
        // `at` points at the oldest level, `DROP_FRAMES` before this one.
        let from = self.recent[self.at];
        self.recent[self.at] = db;
        self.at = (self.at + 1) % self.recent.len();
        if floor.is_finite() && from >= floor + CLEAR_DB {
            self.drops[self.heard % DROPS] = (db - from) as f32;
            self.heard = self.heard.saturating_add(1);
            if self.heard >= FIRST && self.heard.is_multiple_of(CHECK_EVERY) {
                self.check();
            }
        }
    }

    /// Looks at the steepest falls and decides.
    fn check(&mut self) {
        let mut sorted = self.drops;
        let n = self.heard.min(DROPS);
        let k = ((n as f64 * STEEP) as usize).min(n - 1);
        let (_, nth, _) = sorted[..n].select_nth_unstable_by(k, f32::total_cmp);
        self.score = *nth as f64;
        let other = if self.far {
            self.score < NEAR_DB
        } else {
            self.score > FAR_DB
        };
        self.against = if other { self.against + 1 } else { 0 };
        if self.against >= HOLD {
            self.far = !self.far;
            self.against = 0;
        }
    }
}

/// Filter state of one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    split: [Biquad; 3],
    sound: [Biquad; 3],
}

/// Where one band is in its syllable, for all channels together.
#[derive(Clone, Copy, Debug)]
struct Band {
    /// Smoothing of the level follower.
    keep: f64,
    /// Mean power, followed.
    power: f64,
    /// Level (dB) at the last onset, and the lowest since.
    peak: f64,
    low: f64,
    /// The gain the band heads for, and has now (linear).
    target: f64,
    gain: f64,
}

impl Band {
    fn new(keep: f64) -> Band {
        Band {
            keep,
            power: 0.0,
            peak: f64::NEG_INFINITY,
            low: f64::NEG_INFINITY,
            target: 1.0,
            gain: 1.0,
        }
    }

    /// Takes the band's mean power of one sample frame and moves its gain.
    #[inline]
    fn follow(&mut self, power: f64, attack: f64, release: f64) {
        self.power = self.keep * self.power + (1.0 - self.keep) * power;
        let a = if self.target < self.gain { attack } else { release };
        self.gain = a * self.gain + (1.0 - a) * self.target;
    }

    /// A new onset resets the peak; the further the band fell below it
    /// since, the deeper the cut, up to `max_db`.
    fn tick(&mut self, max_db: f64) {
        let db = 10.0 * (self.power + 1e-20).log10();
        if db > self.peak || db > self.low + RISE_DB {
            self.peak = db;
            self.low = db;
        } else {
            self.low = self.low.min(db);
        }
        let cut = ((self.peak - db - CUT_FROM_DB) / CUT_RANGE_DB).clamp(0.0, 1.0);
        self.target = db_to_gain(-cut * max_db);
    }
}

/// Damps the room's tail and corrects the sound, faded in while the
/// detector says the microphone is far.
#[derive(Clone, Debug)]
pub(crate) struct Compensation {
    split: [Coeffs; 3],
    sound: [Coeffs; 3],
    state: Vec<Channel>,
    bands: [Band; 4],
    tick_len: usize,
    tick_at: usize,
    attack: f64,
    release: f64,
    /// How much of the compensation is in: 0 is off, bit for bit.
    blend: f64,
    blend_step: f64,
}

impl Compensation {
    pub(crate) fn new(rate: f64, channels: usize) -> Compensation {
        Compensation {
            split: SPLIT_HZ.map(|hz| Coeffs::low_pass(rate, hz, BUTTERWORTH_Q)),
            sound: [
                Coeffs::low_shelf(rate, BASS_HZ, BASS_DB),
                Coeffs::peak(rate, BOX_HZ, BOX_Q, BOX_DB),
                Coeffs::peak(rate, PRESENCE_HZ, PRESENCE_Q, PRESENCE_DB),
            ],
            state: vec![Channel::default(); channels.clamp(1, MAX_CHANNELS)],
            bands: ENVELOPE_MS.map(|ms| Band::new(smoothing(rate, ms))),
            tick_len: (rate * TICK_MS / 1000.0).round().max(1.0) as usize,
            tick_at: 0,
            attack: smoothing(rate, CUT_ATTACK_MS),
            release: smoothing(rate, CUT_RELEASE_MS),
            blend: 0.0,
            blend_step: 1.0 / (BLEND_S * rate),
        }
    }

    /// How much of the compensation is in right now, 0 to 1.
    pub(crate) fn blend(&self) -> f64 {
        self.blend
    }

    /// Off at once; it starts from a clean state when it fades in again.
    pub(crate) fn reset(&mut self) {
        self.blend = 0.0;
    }

    /// One frame of the shaped channels. `far` fades the compensation in,
    /// otherwise out; while it is all out, the frame is left untouched.
    #[inline]
    pub(crate) fn run(&mut self, frame: &mut [f64], far: bool) {
        if self.blend == 0.0 {
            if !far {
                return;
            }
            // The filters held whatever they had when it faded out.
            self.state.fill(Channel::default());
            self.bands = self.bands.map(|b| Band::new(b.keep));
            self.tick_at = 0;
        }
        self.blend = if far {
            (self.blend + self.blend_step).min(1.0)
        } else {
            (self.blend - self.blend_step).max(0.0)
        };
        if self.blend == 0.0 {
            return;
        }

        let mut power = [0f64; 4];
        for (v, st) in frame.iter_mut().zip(self.state.iter_mut()) {
            // Each band is what is left minus its low pass, so the bands
            // add up to the input exactly.
            let x = *v;
            let b0 = st.split[0].run(&self.split[0], x);
            let rest = x - b0;
            let b1 = st.split[1].run(&self.split[1], rest);
            let rest = rest - b1;
            let b2 = st.split[2].run(&self.split[2], rest);
            let mut y = 0.0;
            for ((p, band), part) in power.iter_mut().zip(&self.bands).zip([b0, b1, b2, rest - b2]) {
                *p += part * part;
                y += band.gain * part;
            }
            let mut sound = y;
            for (s, c) in st.sound.iter_mut().zip(self.sound.iter()) {
                sound = s.run(c, sound);
            }
            *v = y + self.blend * (sound - y);
        }

        let n = frame.len().clamp(1, self.state.len()) as f64;
        for (band, p) in self.bands.iter_mut().zip(power) {
            band.follow(p / n, self.attack, self.release);
        }
        self.tick_at += 1;
        if self.tick_at == self.tick_len {
            self.tick_at = 0;
            let max_db = CUT_MAX_DB * self.blend;
            for band in self.bands.iter_mut() {
                band.tick(max_db);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Voice;
    use kyrr_common::{Amount, VoiceSettings};

    const RATE: u32 = 48_000;

    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0 as f64 / u32::MAX as f64 * 2.0 - 1.0
        }
    }

    /// Talking: syllables of 120 to 250 ms on a voice from 100 to 180 Hz,
    /// with pauses of 40 to 300 ms, over a quiet room (−70 dBFS).
    fn talk(seconds: f64) -> Vec<f32> {
        let frames = (seconds * RATE as f64) as usize;
        let rate = RATE as f64;
        let mut dice = Noise(7);
        let mut room = Noise(11);
        let mut out = Vec::with_capacity(frames);
        let mut phase = 0.0;
        while out.len() < frames {
            let len = ((0.12 + 0.13 * (dice.next() + 1.0) / 2.0) * rate) as usize;
            let gap = ((0.04 + 0.26 * (dice.next() + 1.0) / 2.0) * rate) as usize;
            let pitch = 140.0 + 40.0 * dice.next();
            let loud = 0.15 + 0.1 * dice.next();
            let (attack, release) = (0.015 * rate, 0.025 * rate);
            for i in 0..len + gap {
                let swell = if i >= len {
                    0.0
                } else {
                    let i = i as f64;
                    let up = (i / attack).min(1.0);
                    let down = ((len as f64 - i) / release).min(1.0);
                    (std::f64::consts::FRAC_PI_2 * up.min(down)).sin().powi(2)
                };
                phase += 2.0 * std::f64::consts::PI * pitch / rate;
                let voice: f64 = (1..=25).map(|k| (k as f64 * phase).sin() / k as f64).sum();
                out.push((loud * swell * voice + 0.0003 * room.next()) as f32);
            }
        }
        out.truncate(frames);
        out
    }

    /// A room: four combs and two all-passes (Schroeder), the echo dying
    /// away in `t60` seconds. The direct sound is `drr` dB louder than the
    /// echo: positive is close to the microphone, negative far from it.
    fn room(dry: &[f32], t60: f64, drr: f64) -> Vec<f32> {
        let rate = RATE as f64;
        let combs: Vec<usize> = [0.0297, 0.0371, 0.0411, 0.0437]
            .iter()
            .map(|s| (s * rate) as usize)
            .collect();
        let gains: Vec<f64> = combs
            .iter()
            .map(|&d| 10f64.powf(-3.0 * d as f64 / rate / t60))
            .collect();
        let mut lines: Vec<Vec<f64>> = combs.iter().map(|&d| vec![0.0; d]).collect();
        let passes = [(0.005 * rate) as usize, (0.0017 * rate) as usize];
        let mut pass_lines: Vec<Vec<f64>> = passes.iter().map(|&d| vec![0.0; d]).collect();
        let mut wet = Vec::with_capacity(dry.len());
        for (n, &x) in dry.iter().enumerate() {
            let mut sum = 0.0;
            for ((line, &d), &g) in lines.iter_mut().zip(&combs).zip(&gains) {
                let y = line[n % d];
                line[n % d] = x as f64 + g * y;
                sum += y;
            }
            for (line, &d) in pass_lines.iter_mut().zip(&passes) {
                let delayed = line[n % d];
                let v = sum + 0.7 * delayed;
                line[n % d] = v;
                sum = delayed - 0.7 * v;
            }
            wet.push(sum);
        }
        // The echo's energy `drr` below the dry voice's.
        let energy = |v: &mut dyn Iterator<Item = f64>| v.map(|x| x * x).sum::<f64>();
        let scale = (energy(&mut dry.iter().map(|&x| x as f64)) / energy(&mut wet.iter().copied())).sqrt()
            * 10f64.powf(-drr / 20.0);
        dry.iter()
            .zip(&wet)
            .map(|(&x, &w)| (x as f64 + scale * w) as f32)
            .collect()
    }

    fn settings(distance: bool) -> VoiceSettings {
        VoiceSettings {
            low_cut: false,
            deesser: Amount::Off,
            compressor: Amount::Off,
            auto_level: false,
            distance,
            ..VoiceSettings::default()
        }
    }

    /// Runs `input` through a voice in 10 ms blocks (no noise filter in
    /// between).
    fn run(voice: &mut Voice, input: &[f32]) -> Vec<f32> {
        let mut out = input.to_vec();
        for block in out.chunks_mut(480) {
            voice.pre(block);
            voice.post(block);
        }
        out
    }

    #[test]
    fn close_up_the_audio_stays_the_same_bit_for_bit() {
        for input in [talk(30.0), room(&talk(30.0), 0.6, 15.0)] {
            let mut on = Voice::new(RATE, 1);
            on.set(&settings(true));
            let mut off = Voice::new(RATE, 1);
            off.set(&settings(false));
            assert_eq!(run(&mut on, &input), run(&mut off, &input));
            assert!(!on.far(), "score {:?}", on.room.score());
            assert_eq!(on.distance_blend(), 0.0);
        }
    }

    #[test]
    fn far_in_an_echoing_room_the_tail_comes_down() {
        let dry = talk(60.0);
        let far = room(&dry, 0.8, -4.0);
        let mut on = Voice::new(RATE, 1);
        on.set(&settings(true));
        let mut off = Voice::new(RATE, 1);
        off.set(&settings(false));
        let (a, b) = (run(&mut off, &far), run(&mut on, &far));
        assert!(on.far(), "score {:?}", on.room.score());
        assert_eq!(on.distance_blend(), 1.0);
        // In the pauses of the dry voice, from 100 ms after a syllable on,
        // only the room is left: it has to be clearly quieter. (The tail's
        // loud start is cut least, so its energy drops less than the 6 to
        // 7 dB the gains average there.)
        let mut since = usize::MAX / 2;
        let (mut tail_off, mut tail_on) = (0.0, 0.0);
        for (i, x) in dry.iter().enumerate().skip(45 * RATE as usize) {
            since = if x.abs() > 0.01 { 0 } else { since + 1 };
            if since > RATE as usize / 10 {
                tail_off += (a[i] as f64).powi(2);
                tail_on += (b[i] as f64).powi(2);
            }
        }
        let cut = 10.0 * (tail_on / tail_off).log10();
        assert!(cut < -3.0, "tail {cut:.1} dB");
    }

    #[test]
    fn switched_off_it_fades_out() {
        let far = room(&talk(40.0), 0.8, -4.0);
        let mut voice = Voice::new(RATE, 1);
        voice.set(&settings(true));
        run(&mut voice, &far[..30 * RATE as usize]);
        assert!(voice.distance_blend() > 0.0);
        voice.set(&settings(false));
        run(&mut voice, &far[30 * RATE as usize..]);
        assert_eq!(voice.distance_blend(), 0.0);
        assert!(!voice.far(), "and the hint with it");
    }
}
