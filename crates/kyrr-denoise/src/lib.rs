//! kyrr-denoise: DPDFNet voice filter for any capture format.
//!
//! [`VoiceFilter`] takes interleaved frames at the device's rate and channel
//! count, mixes them down to mono, runs DPDFNet at 48 kHz (resampling when the
//! device runs at another rate) and writes the cleaned signal back to every
//! channel. After construction it never allocates, so it can run on an audio
//! thread; blocks larger than `max_block` are split, not rejected.
//!
//! Loading a model is separate ([`load`]) because it is slow and needs a large
//! stack, which audio threads of hosts like the Windows audio engine lack.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// For tests: switches the INT8 kernels process-wide.
#[cfg(feature = "native")]
#[doc(hidden)]
pub use hushmic_denoiser::use_int8_kernel;
#[cfg(feature = "native")]
pub use hushmic_denoiser::{int8_kernel, Engine, Inference};
pub use hushmic_denoiser::{Denoiser, Mode};
use hushmic_denoiser::{StreamDenoiser, HOP, LATENCY_SAMPLES, SAMPLE_RATE};

/// Rate the DPDFNet models run at.
pub const MODEL_RATE: u32 = SAMPLE_RATE;

/// Largest attenuation; 100 dB removes noise completely (HushMic's default).
pub const MAX_LIMIT_DB: f32 = 100.0;

/// Stack for the loader thread: creating an ONNX Runtime session needs more
/// than the 128 KB some audio threads have.
const LOADER_STACK: usize = 8 * 1024 * 1024;

/// The filter's work is weighed over this much audio...
const LOAD_WINDOW: Duration = Duration::from_millis(100);
/// ...and is too much for the computer once it takes this share of the
/// audio's own time in two windows in a row: the audio thread falls behind,
/// the microphone stutters, and on Linux a real-time thread that never gets
/// to rest is ended by the system, with the program. One slow window alone
/// is a hiccup (another program, a waking processor, memory read back from
/// disk), not a computer too slow for the model.
const TOO_SLOW: f64 = 0.9;
/// A window this far behind is too much at once.
const FAR_TOO_SLOW: f64 = 1.5;
/// A filter left out is tried again after this much audio, then after twice
/// as much each time, up to [`REST_LONGEST`]: the computer may have been
/// busy with something else for a while.
const REST_FIRST: Duration = Duration::from_secs(30);
const REST_LONGEST: Duration = Duration::from_secs(300);

/// How much of the audio's own time the filter takes, to tell a computer
/// that is too slow for a model within a fifth of a second. Never
/// allocates, for the audio thread.
#[derive(Default)]
pub struct Load {
    busy: Duration,
    heard: Duration,
    /// Past the first block of a new filter, whose caches are cold.
    warm: bool,
    /// The window before was too much.
    behind: bool,
    /// Audio passed by while the filter was left out, and how often it was
    /// tried again.
    rested: Duration,
    rests: u32,
}

impl Load {
    /// Adds a block of `frames` at `rate` that the filter took `took` for.
    /// True when the window it completes was too much.
    pub fn add(&mut self, took: Duration, frames: usize, rate: u32) -> bool {
        if !std::mem::replace(&mut self.warm, true) || rate == 0 {
            return false;
        }
        self.busy += took;
        self.heard += Duration::from_secs_f64(frames as f64 / rate as f64);
        if self.heard < LOAD_WINDOW {
            return false;
        }
        let share = self.busy.as_secs_f64() / self.heard.as_secs_f64();
        self.busy = Duration::ZERO;
        self.heard = Duration::ZERO;
        let behind = share > TOO_SLOW;
        let too_slow = share > FAR_TOO_SLOW || (behind && self.behind);
        self.behind = behind;
        too_slow
    }

    /// After a pause in the audio: the next block is slow again and does
    /// not count. How often the filter was left out stays.
    pub fn restart(&mut self) {
        *self = Load {
            rested: self.rested,
            rests: self.rests,
            ..Load::default()
        };
    }

    /// Adds a block of `frames` at `rate` that went by while the filter was
    /// left out. True when it is time to try it again; it is then weighed
    /// afresh, its first block not counting.
    pub fn rest(&mut self, frames: usize, rate: u32) -> bool {
        if rate == 0 {
            return false;
        }
        self.rested += Duration::from_secs_f64(frames as f64 / rate as f64);
        let due = REST_FIRST.saturating_mul(1 << self.rests.min(4)).min(REST_LONGEST);
        if self.rested < due {
            return false;
        }
        *self = Load {
            rests: self.rests + 1,
            ..Load::default()
        };
        true
    }
}

/// The two DPDFNet models shipped with kyrr.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    /// `dpdfnet8`: best quality.
    Quality,
    /// `dpdfnet2`: less than half the CPU of `Quality`.
    Light,
}

impl Model {
    pub fn file_name(self) -> &'static str {
        match self {
            Model::Quality => "dpdfnet8_48khz_hr.onnx",
            Model::Light => "dpdfnet2_48khz_hr.onnx",
        }
    }

    /// The native engine's packed weights, next to the `.onnx` file.
    pub fn weights_name(self) -> &'static str {
        match self {
            Model::Quality => "dpdfnet8_48khz_hr.weights.f32",
            Model::Light => "dpdfnet2_48khz_hr.weights.f32",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Quality => "quality",
            Model::Light => "light",
        }
    }
}

/// Loads `model` from `dir` on a thread with a large stack. The native INT8
/// engine runs when the CPU has AVX2/FMA and the `.weights.f32` file is
/// present; otherwise ONNX Runtime from `ort_library` (initialized once per
/// process, later paths are ignored). Only ONNX Runtime needs the `.onnx`
/// file itself; the AppImage and the tarball ship just the weights.
pub fn load(dir: &Path, model: Model, ort_library: Option<&Path>) -> Result<Denoiser, String> {
    let path = dir.join(model.file_name());
    let ort = ort_library.map(PathBuf::from);
    std::thread::Builder::new()
        .name("kyrr-denoise-loader".into())
        .stack_size(LOADER_STACK)
        .spawn(move || {
            if !native_possible(&path) {
                let ort = ort.ok_or(
                    "native engine unavailable (needs AVX2/FMA and the .weights.f32 file) and no ONNX Runtime given",
                )?;
                hushmic_denoiser::init_runtime(&ort).map_err(|e| format!("{}: {e:?}", ort.display()))?;
            }
            #[cfg(feature = "native")]
            let denoiser = Denoiser::from_file_with(&path, Inference::Auto);
            #[cfg(not(feature = "native"))]
            let denoiser = Denoiser::from_file(&path);
            denoiser.map_err(|e| format!("{}: {e:?}", path.display()))
        })
        .map_err(|e| format!("loader thread: {e}"))?
        .join()
        .map_err(|_| "loader thread panicked".to_string())?
}

/// Whether the native INT8 engine can run: AVX2 and FMA, and the model's
/// packed weights next to it (the engine verifies their checksum itself).
pub fn native_possible(model_path: &Path) -> bool {
    #[cfg(all(feature = "native", target_arch = "x86_64"))]
    let cpu = std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(all(feature = "native", target_arch = "x86_64")))]
    let cpu = false;
    cpu && model_path.with_extension("weights.f32").is_file()
}

/// Fixed-ratio bridge between the device rate and 48 kHz, in 10 ms chunks.
struct RateBridge {
    down: Fft<f32>,
    up: Fft<f32>,
    /// Device-rate input not yet resampled (less than `down_in` after a call).
    pending: Vec<f32>,
    /// Denoised 48 kHz samples not yet resampled back.
    mid: Vec<f32>,
    buf48: Vec<f32>,
    buf_dev: Vec<f32>,
    down_in: usize,
    down_out: usize,
    up_in: usize,
    up_out: usize,
}

impl RateBridge {
    fn new(rate: u32, max_block: usize) -> Result<Self, String> {
        let dev = rate as usize;
        let model = MODEL_RATE as usize;
        let down = Fft::<f32>::new(dev, model, dev / 100, 1, FixedSync::Both).map_err(|e| format!("resampler: {e}"))?;
        let up = Fft::<f32>::new(model, dev, model / 100, 1, FixedSync::Both).map_err(|e| format!("resampler: {e}"))?;
        let (down_in, down_out) = (down.input_frames_next(), down.output_frames_next());
        let (up_in, up_out) = (up.input_frames_next(), up.output_frames_next());
        Ok(RateBridge {
            pending: Vec::with_capacity(down_in + max_block),
            mid: Vec::with_capacity(up_in + down_out + 2 * HOP),
            buf48: vec![0.0; down_out],
            buf_dev: vec![0.0; up_out],
            down,
            up,
            down_in,
            down_out,
            up_in,
            up_out,
        })
    }

    /// Frames (device rate) that can sit in the bridge's buffers and the
    /// denoiser's hop buffer without having produced output yet.
    fn holdback(&self, rate: u32) -> usize {
        if self.down_out == HOP && self.up_in == HOP {
            // Every complete 10 ms input chunk is one hop and comes straight
            // back out, so only the partial input chunk is held back.
            return self.down_in;
        }
        let hop_dev = (HOP * rate as usize).div_ceil(MODEL_RATE as usize);
        let up_dev = (self.up_in * rate as usize).div_ceil(MODEL_RATE as usize);
        self.down_in + hop_dev + up_dev
    }

    /// Resampler delays expressed in device frames.
    fn delay(&self, rate: u32) -> f64 {
        let down = self.down.output_delay() as f64 * rate as f64 / MODEL_RATE as f64;
        down + self.up.output_delay() as f64
    }

    fn process(&mut self, mono: &[f32], stream: &mut StreamDenoiser, out: &mut VecDeque<f32>) {
        self.pending.extend_from_slice(mono);
        let mut used = 0;
        while self.pending.len() - used >= self.down_in {
            let chunk = &self.pending[used..used + self.down_in];
            used += self.down_in;
            // The adapters only fail on size mismatches, which the chunk
            // sizes above rule out; skip the chunk rather than panic.
            let (Ok(src), Ok(mut dst)) = (
                InterleavedSlice::new(chunk, 1, self.down_in),
                InterleavedSlice::new_mut(&mut self.buf48[..], 1, self.down_out),
            ) else {
                continue;
            };
            if self.down.process_into_buffer(&src, &mut dst, None).is_err() {
                continue;
            }
            self.mid.extend_from_slice(stream.process(&self.buf48));
            let mut done = 0;
            while self.mid.len() - done >= self.up_in {
                let chunk = &self.mid[done..done + self.up_in];
                done += self.up_in;
                let (Ok(src), Ok(mut dst)) = (
                    InterleavedSlice::new(chunk, 1, self.up_in),
                    InterleavedSlice::new_mut(&mut self.buf_dev[..], 1, self.up_out),
                ) else {
                    continue;
                };
                if self.up.process_into_buffer(&src, &mut dst, None).is_ok() {
                    out.extend(self.buf_dev.iter().copied());
                }
            }
            self.mid.drain(..done);
        }
        self.pending.drain(..used);
    }

    fn reset(&mut self) {
        self.down.reset();
        self.up.reset();
        self.pending.clear();
        self.mid.clear();
    }
}

/// Real-time voice filter for one capture stream.
pub struct VoiceFilter {
    rate: u32,
    channels: usize,
    max_block: usize,
    stream: StreamDenoiser,
    bridge: Option<RateBridge>,
    /// Mono mix of the current block.
    mono: Vec<f32>,
    /// Denoised mono samples at the device rate, waiting to be handed out.
    /// Primed with silence so every block can be filled although the engine
    /// only emits whole hops and the resamplers whole chunks.
    out: VecDeque<f32>,
    prime: usize,
    underruns: u64,
}

impl VoiceFilter {
    /// `max_block` is the largest block (in frames) handled without splitting;
    /// pass the host's maximum period.
    pub fn new(denoiser: Denoiser, rate: u32, channels: usize, max_block: usize) -> Result<Self, String> {
        if channels == 0 || rate == 0 || max_block == 0 {
            return Err(format!("unsupported format: {rate} Hz, {channels} channels"));
        }
        let bridge = if rate == MODEL_RATE {
            None
        } else {
            Some(RateBridge::new(rate, max_block)?)
        };
        let prime = match &bridge {
            None => HOP,
            Some(b) => b.holdback(rate),
        };
        let mut filter = VoiceFilter {
            rate,
            channels,
            max_block,
            stream: StreamDenoiser::new(denoiser),
            bridge,
            mono: Vec::with_capacity(max_block),
            out: VecDeque::with_capacity(prime + 2 * max_block + 4 * HOP),
            prime,
            underruns: 0,
        };
        filter.reset();
        Ok(filter)
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Delay from input to output in device frames (rounded).
    pub fn latency_frames(&self) -> usize {
        let model = LATENCY_SAMPLES as f64 * self.rate as f64 / MODEL_RATE as f64;
        let bridge = self.bridge.as_ref().map_or(0.0, |b| b.delay(self.rate));
        (self.prime as f64 + model + bridge).round() as usize
    }

    /// Times the output ran dry and was padded with silence (should stay 0).
    pub fn underruns(&self) -> u64 {
        self.underruns
    }

    pub fn denoiser(&self) -> &Denoiser {
        self.stream.denoiser()
    }

    /// Process, bypass (latency-aligned dry signal) or mute; applies from the next hop.
    pub fn set_mode(&mut self, mode: Mode) {
        if self.stream.denoiser().mode() != mode {
            self.stream.denoiser_mut().set_mode(mode);
        }
    }

    /// Maximum attenuation in dB, clamped to 0..=100.
    pub fn set_limit_db(&mut self, db: f32) {
        let db = if db.is_finite() {
            db.clamp(0.0, MAX_LIMIT_DB)
        } else {
            MAX_LIMIT_DB
        };
        if self.stream.denoiser().attenuation_limit_db() != db {
            self.stream.denoiser_mut().set_attenuation_limit_db(db);
        }
    }

    /// Clears all state and buffered audio, e.g. when the stream restarts.
    pub fn reset(&mut self) {
        self.stream.reset();
        if let Some(b) = self.bridge.as_mut() {
            b.reset();
        }
        self.out.clear();
        self.out.extend(std::iter::repeat_n(0.0, self.prime));
    }

    /// Filters interleaved frames. `input` and `output` must hold the same
    /// number of samples, a multiple of the channel count; extra samples are
    /// copied through. May run in place via [`VoiceFilter::process_in_place`].
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let n = input.len().min(output.len());
        let whole = n - n % self.channels;
        let step = self.max_block * self.channels;
        let mut pos = 0;
        while pos < whole {
            let end = (pos + step).min(whole);
            output[pos..end].copy_from_slice(&input[pos..end]);
            self.process_block(&mut output[pos..end]);
            pos = end;
        }
        output[whole..n].copy_from_slice(&input[whole..n]);
    }

    /// Filters interleaved frames in place.
    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        let whole = buf.len() - buf.len() % self.channels;
        let step = self.max_block * self.channels;
        for block in buf[..whole].chunks_mut(step) {
            self.process_block(block);
        }
    }

    fn process_block(&mut self, buf: &mut [f32]) {
        let ch = self.channels;
        let frames = buf.len() / ch;
        // Mono mix; a single NaN/Inf would poison the model's recurrent
        // state, so non-finite samples count as silence.
        self.mono.clear();
        for frame in buf.chunks_exact(ch) {
            let sum: f32 = frame.iter().map(|&x| if x.is_finite() { x } else { 0.0 }).sum();
            self.mono.push(sum / ch as f32);
        }

        match self.bridge.as_mut() {
            // Hop-sized pieces keep the stream's buffers at their initial
            // size, so a large first block does not allocate on the audio
            // thread.
            None => {
                for piece in self.mono.chunks(HOP) {
                    self.out.extend(self.stream.process(piece).iter().copied());
                }
            }
            Some(b) => b.process(&self.mono, &mut self.stream, &mut self.out),
        }

        let mut bad = false;
        for frame in buf.chunks_exact_mut(ch) {
            let y = match self.out.pop_front() {
                Some(y) if y.is_finite() => y,
                Some(_) => {
                    bad = true;
                    0.0
                }
                None => {
                    self.underruns += 1;
                    0.0
                }
            };
            frame.fill(y);
        }
        debug_assert_eq!(frames * ch, buf.len());
        if bad {
            // Never hand NaN/Inf on; restart the engine from a clean state.
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of 1024 frames at 48 kHz is about 21 ms of audio.
    const BLOCK: usize = 1024;
    const RATE: u32 = 48_000;

    /// Blocks that each take `share` of their own time; true if any was
    /// too much.
    fn feed(load: &mut Load, share: f64, blocks: usize) -> bool {
        let audio = BLOCK as f64 / RATE as f64;
        (0..blocks)
            .map(|_| load.add(Duration::from_secs_f64(audio * share), BLOCK, RATE))
            .fold(false, |a, b| a | b)
    }

    #[test]
    fn a_filter_that_keeps_up_is_fine() {
        assert!(!feed(&mut Load::default(), 0.6, 500));
    }

    #[test]
    fn a_filter_slower_than_the_audio_shows_within_two_windows() {
        assert!(!feed(&mut Load::default(), 1.1, 7));
        assert!(feed(&mut Load::default(), 1.1, 11));
        assert!(feed(&mut Load::default(), 0.95, 11));
        // Far behind: at once.
        assert!(feed(&mut Load::default(), 2.0, 7));
    }

    #[test]
    fn a_filter_left_out_is_tried_again_less_often_each_time() {
        let mut load = Load::default();
        let block = |load: &mut Load| load.rest(BLOCK, RATE);
        // Blocks of about 21 ms until it is due.
        let mut waits = Vec::new();
        for _ in 0..6 {
            let mut blocks = 1;
            while !block(&mut load) {
                blocks += 1;
            }
            waits.push((blocks as f64 * BLOCK as f64 / RATE as f64).round() as u32);
            // Tried again: weighed afresh, the first block free.
            assert!(!load.add(Duration::from_millis(500), BLOCK, RATE));
        }
        assert_eq!(waits, [30, 60, 120, 240, 300, 300]);
    }

    #[test]
    fn one_slow_window_is_a_hiccup() {
        let mut load = Load::default();
        assert!(!feed(&mut load, 0.4, 20));
        assert!(!feed(&mut load, 1.2, 5));
        assert!(!feed(&mut load, 0.4, 20));
        assert!(!feed(&mut load, 1.2, 5));
        assert!(!feed(&mut load, 0.4, 20));
    }

    #[test]
    fn a_slow_first_block_after_loading_does_not_count() {
        let mut load = Load::default();
        assert!(!load.add(Duration::from_millis(500), BLOCK, RATE));
        assert!(!feed(&mut load, 0.5, 10));
    }
}
