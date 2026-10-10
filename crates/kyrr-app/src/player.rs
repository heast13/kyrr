//! Plays the before/after probe on the default output device.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};

/// Linear resampling from `from` to `to` Hz: plenty for a voice check.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 || to == 0 {
        return samples.to_vec();
    }
    let len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..len)
        .map(|i| {
            let pos = i as f64 * step;
            let at = pos as usize;
            let frac = (pos - at as f64) as f32;
            let a = samples[at.min(samples.len() - 1)];
            let b = samples[(at + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

fn fill<T: SizedSample>(out: &mut [T], channels: usize, samples: &[f32], pos: &AtomicUsize, convert: fn(f32) -> T) {
    let mut at = pos.load(Ordering::Relaxed);
    for frame in out.chunks_exact_mut(channels) {
        let v = samples.get(at).copied().unwrap_or(0.0);
        frame.fill(convert(v));
        at += 1;
    }
    pos.store(at, Ordering::Relaxed);
}

/// Plays mono `samples` at `rate` Hz and returns when done or `stop` is set.
pub fn play(rate: u32, samples: &[f32], stop: &AtomicBool) -> Result<(), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no speaker or headphones found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config = supported.config();
    let channels = config.channels.max(1) as usize;
    let samples = Arc::new(resample(samples, rate, config.sample_rate));
    let pos = Arc::new(AtomicUsize::new(0));
    let err = |e| eprintln!("kyrr: playback error: {e}");
    let (s, p) = (Arc::clone(&samples), Arc::clone(&pos));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_output_stream(
            config,
            move |out: &mut [f32], _| fill(out, channels, &s, &p, |v| v),
            err,
            None,
        ),
        SampleFormat::I16 => device.build_output_stream(
            config,
            move |out: &mut [i16], _| fill(out, channels, &s, &p, |v| (v * 32767.0) as i16),
            err,
            None,
        ),
        SampleFormat::I32 => device.build_output_stream(
            config,
            move |out: &mut [i32], _| fill(out, channels, &s, &p, |v| (v as f64 * 2_147_483_647.0) as i32),
            err,
            None,
        ),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    // Let the device's buffer run out before the stream closes; give up if
    // the device vanished and stopped asking for samples.
    let deadline = std::time::Instant::now()
        + Duration::from_secs_f64(samples.len() as f64 / config.sample_rate.max(1) as f64)
        + Duration::from_secs(2);
    while pos.load(Ordering::Relaxed) < samples.len() + config.sample_rate as usize / 10 {
        if stop.load(Ordering::Relaxed) || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_duration() {
        let one_second: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        assert_eq!(resample(&one_second, 48_000, 44_100).len(), 44_100);
        assert_eq!(resample(&one_second, 48_000, 48_000), one_second);
    }
}
