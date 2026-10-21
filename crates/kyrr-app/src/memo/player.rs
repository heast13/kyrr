//! Playing a voice memo on the default output device, as it is read from
//! the file: a long memo is never all in memory.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};

use super::opusfile::{Reader, GRANULE_RATE};

/// How much decoded sound waits for the device, in seconds.
const AHEAD: f64 = 0.5;

/// A memo playing.
pub struct Player {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    /// Samples the device played, at its rate, and where it started.
    played: Arc<AtomicU64>,
    rate: Arc<AtomicU64>,
    from: f64,
}

impl Player {
    /// Starts playing the memo at `path`, `from` seconds in.
    pub fn start(path: &Path, from: f64) -> Player {
        let player = Player {
            path: path.to_path_buf(),
            stop: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
            played: Arc::new(AtomicU64::new(0)),
            rate: Arc::new(AtomicU64::new(0)),
            from,
        };
        let (path, stop, done) = (player.path.clone(), Arc::clone(&player.stop), Arc::clone(&player.done));
        let (played, rate) = (Arc::clone(&player.played), Arc::clone(&player.rate));
        let spawned = std::thread::Builder::new()
            .name("kyrr-memo-play".into())
            .spawn(move || {
                if let Err(e) = play(&path, from, &stop, &played, &rate) {
                    eprintln!("kyrr: memo: playing {}: {e}", path.display());
                }
                done.store(true, Ordering::Relaxed);
            });
        if spawned.is_err() {
            player.done.store(true, Ordering::Relaxed);
        }
        player
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where in the memo it is, in seconds.
    pub fn seconds(&self) -> f64 {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return self.from;
        }
        self.from + self.played.load(Ordering::Relaxed) as f64 / rate as f64
    }

    /// Whether it played to the end (or could not play).
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn play(path: &Path, from: f64, stop: &AtomicBool, played: &Arc<AtomicU64>, rate: &AtomicU64) -> Result<(), String> {
    let mut reader = Reader::open(path)?;
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no speaker or headphones found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config = supported.config();
    let channels = config.channels.max(1) as usize;
    let out_rate = config.sample_rate.max(1);
    let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let err = |e| eprintln!("kyrr: memo playback error: {e}");
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build(&device, &config, channels, &queue, played, |v| v, err),
        SampleFormat::I16 => build(
            &device,
            &config,
            channels,
            &queue,
            played,
            |v| (v * 32767.0) as i16,
            err,
        ),
        SampleFormat::I32 => build(
            &device,
            &config,
            channels,
            &queue,
            played,
            |v| (v as f64 * 2_147_483_647.0) as i32,
            err,
        ),
        other => return Err(format!("unsupported sample format {other}")),
    }?;

    // To the start: decoding is quick, a minute of it takes a blink.
    let mut skip = (from.max(0.0) * GRANULE_RATE as f64) as usize;
    let mut decoded = Vec::new();
    while skip > 0 {
        decoded.clear();
        if !reader.next(&mut decoded) || stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let dropped = skip.min(decoded.len());
        skip -= dropped;
        decoded.drain(..dropped);
    }
    let mut linear = Linear::new(GRANULE_RATE, out_rate);
    let mut converted = Vec::new();
    linear.push(&decoded, &mut converted);
    queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(converted.drain(..));
    rate.store(out_rate as u64, Ordering::Relaxed);
    stream.play().map_err(|e| e.to_string())?;

    let ahead = (AHEAD * out_rate as f64) as usize;
    let mut ended = false;
    let mut last_played = (0, Instant::now());
    while !stop.load(Ordering::Relaxed) {
        let waiting = queue.lock().unwrap_or_else(|e| e.into_inner()).len();
        if ended && waiting == 0 {
            // The device's own buffer still plays this long.
            std::thread::sleep(Duration::from_millis(100));
            break;
        }
        if !ended && waiting < ahead {
            decoded.clear();
            ended = !reader.next(&mut decoded);
            converted.clear();
            linear.push(&decoded, &mut converted);
            queue.lock().unwrap_or_else(|e| e.into_inner()).extend(converted.iter());
            continue;
        }
        // A device that went away stops asking: give up after a while.
        let now = played.load(Ordering::Relaxed);
        if now != last_played.0 {
            last_played = (now, Instant::now());
        } else if last_played.1.elapsed() > Duration::from_secs(3) {
            return Err("the output device stopped".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

fn build<T: SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    queue: &Arc<Mutex<VecDeque<f32>>>,
    played: &Arc<AtomicU64>,
    convert: fn(f32) -> T,
    err: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String> {
    let (queue, played) = (Arc::clone(queue), Arc::clone(played));
    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _| {
                let mut queue = queue.try_lock().ok();
                let mut taken = 0;
                for frame in out.chunks_exact_mut(channels) {
                    let v = queue.as_mut().and_then(|q| q.pop_front());
                    taken += u64::from(v.is_some());
                    frame.fill(convert(v.unwrap_or(0.0)));
                }
                played.fetch_add(taken, Ordering::Relaxed);
            },
            err,
            None,
        )
        .map_err(|e| e.to_string())
}

/// Linear conversion from one rate to another as the sound comes in:
/// plenty for a voice.
struct Linear {
    step: f64,
    /// Where the next output sample lies in `pending`.
    at: f64,
    pending: Vec<f32>,
}

impl Linear {
    fn new(from: u32, to: u32) -> Linear {
        Linear {
            step: from as f64 / to.max(1) as f64,
            at: 0.0,
            pending: Vec::new(),
        }
    }

    fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        if self.step == 1.0 {
            out.extend_from_slice(samples);
            return;
        }
        self.pending.extend_from_slice(samples);
        while self.at + 1.0 < self.pending.len() as f64 {
            let i = self.at as usize;
            let frac = (self.at - i as f64) as f32;
            let (a, b) = (self.pending[i], self.pending[i + 1]);
            out.push(a + (b - a) * frac);
            self.at += self.step;
        }
        let used = (self.at as usize).min(self.pending.len());
        self.pending.drain(..used);
        self.at -= used as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converting_keeps_the_length() {
        let mut linear = Linear::new(48_000, 44_100);
        let mut out = Vec::new();
        for piece in vec![0.25f32; 48_000].chunks(960) {
            linear.push(piece, &mut out);
        }
        assert!((44_099..=44_100).contains(&out.len()), "{}", out.len());
        assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-6));
        let mut same = Linear::new(48_000, 48_000);
        let mut out = Vec::new();
        same.push(&[0.5; 10], &mut out);
        assert_eq!(out.len(), 10);
    }
}
