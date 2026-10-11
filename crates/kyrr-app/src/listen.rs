//! Listening to yourself: the microphone, as apps get it (after kyrr's
//! filter), played live on the default output until stopped. Meant for
//! headphones; on speakers it feeds back.

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};

use crate::recorder;

/// Output waits for this much audio before it starts (and again after it
/// ran dry), so it does not crackle.
const PREFILL_MS: usize = 30;
/// More than this queued (the two devices' clocks drift apart) and the
/// oldest audio is dropped, so the delay stays short.
const MAX_MS: usize = 120;

/// Live until dropped.
pub struct Listening {
    _stop: mpsc::Sender<()>,
}

/// Starts listening on its own thread; `failed` hears why it could not
/// start.
pub fn start(microphone: Option<String>, failed: impl FnOnce(String) + Send + 'static) -> Listening {
    let (stop, stopped) = mpsc::channel::<()>();
    std::thread::spawn(move || match streams(microphone.as_deref()) {
        // Runs until the sender is dropped.
        Ok(streams) => {
            let _ = stopped.recv();
            drop(streams);
        }
        Err(e) => failed(e),
    });
    Listening { _stop: stop }
}

/// Linear resampling of a stream that comes in pieces.
struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Position between `last` (0) and the next input sample (1).
    pos: f64,
    last: f32,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Resampler {
            step: from.max(1) as f64 / to.max(1) as f64,
            pos: 0.0,
            last: 0.0,
        }
    }

    fn push(&mut self, sample: f32, out: &mut VecDeque<f32>) {
        while self.pos < 1.0 {
            out.push_back(self.last + (sample - self.last) * self.pos as f32);
            self.pos += self.step;
        }
        self.pos -= 1.0;
        self.last = sample;
    }
}

/// Mono audio at the output's rate, from the input to the output callback.
struct Queue {
    samples: VecDeque<f32>,
    resampler: Resampler,
    playing: bool,
    prefill: usize,
    max: usize,
}

fn record<T: SizedSample>(data: &[T], channels: usize, convert: fn(T) -> f32, queue: &Mutex<Queue>) {
    let Ok(mut q) = queue.lock() else {
        return;
    };
    let q = &mut *q;
    for frame in data.chunks_exact(channels) {
        let mono = frame.iter().map(|&s| convert(s)).sum::<f32>() / channels as f32;
        q.resampler.push(mono, &mut q.samples);
    }
    if q.samples.len() > q.max {
        let excess = q.samples.len() - q.prefill;
        q.samples.drain(..excess);
    }
}

fn play<T: SizedSample>(out: &mut [T], channels: usize, convert: fn(f32) -> T, queue: &Mutex<Queue>) {
    let Ok(mut q) = queue.lock() else {
        return;
    };
    if !q.playing && q.samples.len() >= q.prefill {
        q.playing = true;
    }
    for frame in out.chunks_exact_mut(channels) {
        let v = if q.playing { q.samples.pop_front() } else { None };
        if v.is_none() {
            q.playing = false;
        }
        frame.fill(convert(v.unwrap_or(0.0)));
    }
}

fn streams(microphone: Option<&str>) -> Result<(cpal::Stream, cpal::Stream), String> {
    let input = recorder::device(microphone).ok_or("no microphone found")?;
    let output = cpal::default_host()
        .default_output_device()
        .ok_or("no speaker or headphones found")?;
    let in_supported = input.default_input_config().map_err(|e| e.to_string())?;
    let out_supported = output.default_output_config().map_err(|e| e.to_string())?;
    let (in_config, out_config) = (in_supported.config(), out_supported.config());
    let in_channels = in_config.channels.max(1) as usize;
    let out_channels = out_config.channels.max(1) as usize;
    let ms = |n: usize| out_config.sample_rate as usize * n / 1000;
    let queue = Arc::new(Mutex::new(Queue {
        samples: VecDeque::with_capacity(ms(MAX_MS) * 2),
        resampler: Resampler::new(in_config.sample_rate, out_config.sample_rate),
        playing: false,
        prefill: ms(PREFILL_MS),
        max: ms(MAX_MS),
    }));

    let q = Arc::clone(&queue);
    let err = |e| eprintln!("kyrr: listening error: {e}");
    let input_stream = match in_supported.sample_format() {
        SampleFormat::F32 => input.build_input_stream(
            in_config,
            move |d: &[f32], _| record(d, in_channels, |s| s, &q),
            err,
            None,
        ),
        SampleFormat::I16 => input.build_input_stream(
            in_config,
            move |d: &[i16], _| record(d, in_channels, |s| s as f32 / 32768.0, &q),
            err,
            None,
        ),
        SampleFormat::I32 => input.build_input_stream(
            in_config,
            move |d: &[i32], _| record(d, in_channels, |s| s as f32 / 2_147_483_648.0, &q),
            err,
            None,
        ),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())?;

    let q = Arc::clone(&queue);
    let output_stream = match out_supported.sample_format() {
        SampleFormat::F32 => output.build_output_stream(
            out_config,
            move |out: &mut [f32], _| play(out, out_channels, |v| v, &q),
            err,
            None,
        ),
        SampleFormat::I16 => output.build_output_stream(
            out_config,
            move |out: &mut [i16], _| play(out, out_channels, |v| (v.clamp(-1.0, 1.0) * 32767.0) as i16, &q),
            err,
            None,
        ),
        SampleFormat::I32 => output.build_output_stream(
            out_config,
            move |out: &mut [i32], _| {
                play(
                    out,
                    out_channels,
                    |v| (v.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0) as i32,
                    &q,
                )
            },
            err,
            None,
        ),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())?;

    input_stream.play().map_err(|e| e.to_string())?;
    output_stream.play().map_err(|e| e.to_string())?;
    Ok((input_stream, output_stream))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_rate() {
        for (from, to) in [(48_000, 44_100), (44_100, 48_000), (16_000, 48_000), (48_000, 48_000)] {
            let mut r = Resampler::new(from, to);
            let mut out = VecDeque::new();
            for i in 0..from {
                r.push((i as f32 * 0.01).sin(), &mut out);
            }
            assert!(out.len().abs_diff(to as usize) <= 1, "{from} → {to}: {}", out.len());
        }
    }

    #[test]
    fn same_rate_passes_samples_through() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = VecDeque::new();
        for s in [0.5, -0.25, 1.0] {
            r.push(s, &mut out);
        }
        // One sample of delay: the first output is the silence before.
        assert_eq!(out, [0.0, 0.5, -0.25]);
    }

    #[test]
    fn the_queue_stays_short() {
        let queue = Mutex::new(Queue {
            samples: VecDeque::new(),
            resampler: Resampler::new(48_000, 48_000),
            playing: false,
            prefill: 10,
            max: 100,
        });
        record(&[0.1f32; 500], 1, |s| s, &queue);
        assert_eq!(queue.lock().unwrap().samples.len(), 10);
        // Waits for the prefill, then plays and pauses again when dry.
        let mut out = [1.0f32; 12];
        play(&mut out, 1, |v| v, &queue);
        assert!(out[..10].iter().all(|&v| v > 0.0) && out[10..].iter().all(|&v| v == 0.0));
        assert!(!queue.lock().unwrap().playing);
    }
}
