//! Recording the default microphone while the dictation key is held. The
//! stream is only open during a recording, so Windows shows the microphone
//! as in use only then.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};
use kyrr_common::wave::TAIL;

/// Longest dictation: then kyrr types what it heard. The recording is held in
/// memory at the microphone's rate (about 170 MB at 48 kHz for this long)
/// and recognized in pieces, most of them while it runs.
pub const MAX_SECONDS: usize = 15 * 60;

/// The newest audio of the running recording, for the wave on the screen.
struct Tail {
    ring: [f32; TAIL],
    /// Where the next sample goes.
    next: usize,
    rate: u32,
}

static TAIL_OF_RECORDING: Mutex<Tail> = Mutex::new(Tail {
    ring: [0.0; TAIL],
    next: 0,
    rate: 0,
});

/// Fills `out` with the newest samples (newest last) and returns their
/// rate; 0 while nothing is recorded.
pub fn tail(out: &mut [f32]) -> u32 {
    let Ok(t) = TAIL_OF_RECORDING.lock() else {
        return 0;
    };
    let n = out.len().min(TAIL);
    let from = (t.next + TAIL - n) % TAIL;
    let skip = out.len() - n;
    for (i, o) in out[skip..].iter_mut().enumerate() {
        *o = t.ring[(from + i) % TAIL];
    }
    t.rate
}

pub struct Recording {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    /// The stream reported an error: the microphone was unplugged, or the
    /// audio service went away.
    lost: Arc<AtomicBool>,
}

fn mono<T: SizedSample>(data: &[T], channels: usize, convert: fn(T) -> f32, out: &Mutex<Vec<f32>>, max: usize) {
    let Ok(mut out) = out.lock() else {
        return;
    };
    // Never wait for the wave: it just misses this bit.
    let mut tail = TAIL_OF_RECORDING.try_lock().ok();
    for frame in data.chunks_exact(channels) {
        let sample = frame.iter().map(|&s| convert(s)).sum::<f32>() / channels as f32;
        // The wave keeps listening even when the recording is full.
        if let Some(t) = tail.as_mut() {
            let at = t.next;
            t.ring[at] = sample;
            t.next = (at + 1) % TAIL;
        }
        if out.len() < max {
            out.push(sample);
        }
    }
}

/// The input device whose endpoint id ends with `guid` (the microphone kyrr
/// is attached to, so dictation hears the cleaned signal), else the default.
pub fn device(guid: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(guid) = guid.filter(|g| !g.is_empty()) {
        let guid = guid.to_ascii_lowercase();
        let found = host.input_devices().ok().and_then(|mut devices| {
            devices.find(|d| d.id().is_ok_and(|id| id.id().to_ascii_lowercase().ends_with(&guid)))
        });
        if found.is_some() {
            return found;
        }
    }
    host.default_input_device()
}

impl Drop for Recording {
    fn drop(&mut self) {
        if let Ok(mut t) = TAIL_OF_RECORDING.lock() {
            t.rate = 0;
        }
    }
}

impl Recording {
    pub fn start(microphone: Option<&str>) -> Result<Recording, String> {
        let device = device(microphone).ok_or("no microphone found")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels.max(1) as usize;
        let rate = config.sample_rate;
        // A minute more than the longest dictation, as a busy worker may
        // end it a little late. Memory not written to is not used.
        let max = rate as usize * (MAX_SECONDS + 60);
        // Reserved at once: the audio callback must not wait for a copy of a
        // long recording when a vector would otherwise grow. Memory that
        // is not written to is not used yet. If the system refuses that
        // much, the vector grows as it goes.
        let mut buffer = Vec::new();
        if buffer.try_reserve_exact(max).is_err() {
            let _ = buffer.try_reserve_exact(rate as usize * 60);
        }
        let samples = Arc::new(Mutex::new(buffer));
        let sink = Arc::clone(&samples);
        let lost = Arc::new(AtomicBool::new(false));
        let err = {
            let lost = Arc::clone(&lost);
            move |e: cpal::Error| {
                eprintln!("kyrr: recording error: {e}");
                // Not an overrun or a reroute, which the stream gets over.
                if matches!(
                    e.kind(),
                    cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable
                ) {
                    lost.store(true, Ordering::Relaxed);
                }
            }
        };
        let stream = match supported.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |d: &[f32], _| mono(d, channels, |s| s, &sink, max),
                err,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                config,
                move |d: &[i16], _| mono(d, channels, |s| s as f32 / 32768.0, &sink, max),
                err,
                None,
            ),
            SampleFormat::I32 => device.build_input_stream(
                config,
                move |d: &[i32], _| mono(d, channels, |s| s as f32 / 2_147_483_648.0, &sink, max),
                err,
                None,
            ),
            other => return Err(format!("unsupported sample format {other}")),
        }
        .map_err(|e| e.to_string())?;
        if let Ok(mut t) = TAIL_OF_RECORDING.lock() {
            *t = Tail {
                ring: [0.0; TAIL],
                next: 0,
                rate,
            };
        }
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recording {
            _stream: stream,
            samples,
            rate,
            lost,
        })
    }

    /// The recording so far from `from` seconds on, with its rate and its
    /// length in seconds; `from` is worked out from that length.
    pub fn since(&self, from: impl FnOnce(f32) -> f32) -> (u32, Vec<f32>, f32) {
        let samples = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        let rate = self.rate.max(1) as f32;
        let length = samples.len() as f32 / rate;
        let start = ((from(length) * rate) as usize).min(samples.len());
        (self.rate, samples[start..].to_vec(), length)
    }

    /// The microphone's rate.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The recording from sample `from` on.
    pub fn samples_from(&self, from: usize) -> Vec<f32> {
        let samples = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        samples.get(from..).map_or_else(Vec::new, <[f32]>::to_vec)
    }

    /// Whether the microphone went away during the recording.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    /// Stops recording and returns the sample rate and mono samples.
    pub fn finish(self) -> (u32, Vec<f32>) {
        let rate = self.rate;
        let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
        (rate, samples)
    }
}
