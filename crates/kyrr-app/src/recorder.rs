//! Recording the default microphone while the dictation key is held. The
//! stream is only open during a recording, so Windows shows the microphone
//! as in use only then.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};

/// Longest recording kept; dictation is meant for sentences, not lectures.
pub const MAX_SECONDS: usize = 120;

pub struct Recording {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

fn mono<T: SizedSample>(data: &[T], channels: usize, convert: fn(T) -> f32, out: &Mutex<Vec<f32>>, max: usize) {
    let Ok(mut out) = out.lock() else {
        return;
    };
    for frame in data.chunks_exact(channels) {
        if out.len() >= max {
            break;
        }
        out.push(frame.iter().map(|&s| convert(s)).sum::<f32>() / channels as f32);
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

impl Recording {
    pub fn start(microphone: Option<&str>) -> Result<Recording, String> {
        let device = device(microphone).ok_or("no microphone found")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels.max(1) as usize;
        let rate = config.sample_rate;
        let max = rate as usize * MAX_SECONDS;
        let samples = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 10)));
        let sink = Arc::clone(&samples);
        let err = |e| eprintln!("kyrr: recording error: {e}");
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
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recording {
            _stream: stream,
            samples,
            rate,
        })
    }

    /// Stops recording and returns the sample rate and mono samples.
    pub fn finish(self) -> (u32, Vec<f32>) {
        let rate = self.rate;
        let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
        drop(self);
        (rate, samples)
    }
}
