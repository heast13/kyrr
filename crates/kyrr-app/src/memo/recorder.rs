//! Recording a voice memo: the microphone kyrr is attached to (as for
//! dictation, so the noise filter is in it when it is on), straight into an
//! Opus file. Only a few hundred milliseconds wait in memory at any time,
//! and the start, for the title.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};
use rubato::{Fft, FixedSync, Resampler};

use super::opusfile::{self, Writer};
use crate::recorder;

/// The longest memo; then it ends by itself.
pub const MAX_SECONDS: f64 = 4.0 * 60.0 * 60.0;
/// The start of a memo kept for its title.
const TITLE_SECONDS: usize = 30;
/// How often the writer takes in what the microphone delivered.
const EVERY: Duration = Duration::from_millis(100);

/// Why a memo ended by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Stopped by the user.
    Stopped,
    /// It reached [`MAX_SECONDS`].
    Limit,
    /// The microphone went away.
    MicGone,
    /// The file could not be written (a full disk).
    Disk,
}

/// A finished memo.
pub struct Finished {
    pub path: PathBuf,
    pub seconds: f64,
    pub ending: Ending,
    /// The first seconds at `rate`, for the title.
    pub start: Vec<f32>,
    pub rate: u32,
}

/// A memo being recorded.
pub struct Recording {
    stream: Option<cpal::Stream>,
    stop: Arc<AtomicBool>,
    writer: Option<JoinHandle<Finished>>,
    path: PathBuf,
    started: Instant,
}

impl Recording {
    /// Starts recording microphone `guid` (else the default one) into a new
    /// file at `path`. An error about the microphone starts with
    /// "microphone"; any other one is about the file.
    pub fn start(guid: Option<&str>, path: &Path) -> Result<Recording, String> {
        let device = recorder::device(guid).ok_or("microphone: none found")?;
        let supported = device.default_input_config().map_err(|e| format!("microphone: {e}"))?;
        let config = supported.config();
        let channels = config.channels.max(1) as usize;
        let rate = config.sample_rate;
        let file_rate = if opusfile::RATES.contains(&rate) {
            rate
        } else {
            opusfile::GRANULE_RATE
        };
        let mut resampler = Converter::new(rate, file_rate)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut writer = Writer::create(path, file_rate).map_err(|e| e.to_string())?;

        let incoming = Arc::new(Mutex::new(Vec::<f32>::with_capacity(rate as usize)));
        let lost = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let err = {
            let lost = Arc::clone(&lost);
            move |e: cpal::Error| {
                eprintln!("kyrr: memo recording error: {e}");
                if matches!(
                    e.kind(),
                    cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable
                ) {
                    lost.store(true, Ordering::Relaxed);
                }
            }
        };
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build(&device, &config, channels, Arc::clone(&incoming), |s: f32| s, err),
            SampleFormat::I16 => build(
                &device,
                &config,
                channels,
                Arc::clone(&incoming),
                |s: i16| s as f32 / 32768.0,
                err,
            ),
            SampleFormat::I32 => build(
                &device,
                &config,
                channels,
                Arc::clone(&incoming),
                |s: i32| s as f32 / 2_147_483_648.0,
                err,
            ),
            other => Err(format!("unsupported sample format {other}")),
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                drop(writer);
                let _ = std::fs::remove_file(path);
                return Err(format!("microphone: {e}"));
            }
        };
        recorder::tail_start(rate);
        if let Err(e) = stream.play() {
            recorder::tail_start(0);
            drop(writer);
            let _ = std::fs::remove_file(path);
            return Err(format!("microphone: {e}"));
        }

        let thread_path = path.to_path_buf();
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("kyrr-memo".into())
            .spawn(move || {
                let mut start = Vec::new();
                let mut converted = Vec::new();
                let mut ending = Ending::Stopped;
                loop {
                    let stopping = thread_stop.load(Ordering::Relaxed);
                    let taken = std::mem::take(&mut *incoming.lock().unwrap_or_else(|e| e.into_inner()));
                    let room = (TITLE_SECONDS * rate as usize).saturating_sub(start.len());
                    start.extend_from_slice(&taken[..room.min(taken.len())]);
                    converted.clear();
                    resampler.push(&taken, &mut converted);
                    if stopping {
                        resampler.finish(&mut converted);
                    }
                    if let Err(e) = writer.push(&converted) {
                        eprintln!("kyrr: memo: writing {}: {e}", thread_path.display());
                        ending = Ending::Disk;
                        break;
                    }
                    if stopping {
                        break;
                    }
                    if lost.load(Ordering::Relaxed) {
                        ending = Ending::MicGone;
                        break;
                    }
                    if writer.seconds() >= MAX_SECONDS {
                        ending = Ending::Limit;
                        break;
                    }
                    std::thread::sleep(EVERY);
                }
                let seconds = writer.seconds();
                if let Err(e) = writer.finish() {
                    eprintln!("kyrr: memo: finishing {}: {e}", thread_path.display());
                    ending = Ending::Disk;
                }
                Finished {
                    path: thread_path,
                    seconds,
                    ending,
                    start,
                    rate,
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Recording {
            stream: Some(stream),
            stop,
            writer: Some(handle),
            path: path.to_path_buf(),
            started: Instant::now(),
        })
    }

    /// The file it records into.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Seconds since it started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Whether it ended by itself (limit, microphone gone, disk full):
    /// [`finish`](Self::finish) then says why.
    pub fn ended(&self) -> bool {
        self.writer.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Stops and closes the file.
    pub fn finish(mut self) -> Finished {
        // The microphone first: what it still delivers goes into the file.
        self.stream.take();
        recorder::tail_start(0);
        self.stop.store(true, Ordering::Relaxed);
        let path = self.path.clone();
        self.writer.take().and_then(|w| w.join().ok()).unwrap_or(Finished {
            path,
            seconds: 0.0,
            ending: Ending::Disk,
            start: Vec::new(),
            rate: 0,
        })
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if self.stream.take().is_some() {
            recorder::tail_start(0);
        }
    }
}

fn build<T: SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    incoming: Arc<Mutex<Vec<f32>>>,
    convert: fn(T) -> f32,
    err: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String> {
    let mut mono = Vec::new();
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                mono.clear();
                mono.extend(
                    data.chunks_exact(channels)
                        .map(|frame| frame.iter().map(|&s| convert(s)).sum::<f32>() / channels as f32),
                );
                recorder::tail_push(&mono);
                if let Ok(mut incoming) = incoming.lock() {
                    incoming.extend_from_slice(&mono);
                }
            },
            err,
            None,
        )
        .map_err(|e| e.to_string())
}

/// Converts the microphone's rate to the file's, as it comes in; nothing to
/// do when they are the same.
struct Converter {
    resampler: Option<Fft<f32>>,
    pending: Vec<f32>,
    out: Vec<f32>,
}

impl Converter {
    fn new(from: u32, to: u32) -> Result<Converter, String> {
        let resampler = (from != to)
            .then(|| Fft::<f32>::new(from as usize, to as usize, 1024, 1, FixedSync::Input))
            .transpose()
            .map_err(|e| e.to_string())?;
        let out = vec![0.0; resampler.as_ref().map_or(0, |r| r.output_frames_max())];
        Ok(Converter {
            resampler,
            pending: Vec::new(),
            out,
        })
    }

    fn push(&mut self, samples: &[f32], to: &mut Vec<f32>) {
        let Some(resampler) = self.resampler.as_mut() else {
            to.extend_from_slice(samples);
            return;
        };
        self.pending.extend_from_slice(samples);
        let mut used = 0;
        loop {
            let need = resampler.input_frames_next();
            if self.pending.len() - used < need {
                break;
            }
            let capacity = self.out.len();
            let (Ok(input), Ok(mut output)) = (
                InterleavedSlice::new(&self.pending[used..used + need], 1, need),
                InterleavedSlice::new_mut(&mut self.out[..], 1, capacity),
            ) else {
                break;
            };
            let Ok((taken, made)) = resampler.process_into_buffer(&input, &mut output, None) else {
                break;
            };
            to.extend_from_slice(&self.out[..made]);
            used += taken.max(1);
        }
        self.pending.drain(..used);
    }

    /// The last bit, filled up with silence to a whole chunk.
    fn finish(&mut self, to: &mut Vec<f32>) {
        let Some(need) = self.resampler.as_ref().map(|r| r.input_frames_next()) else {
            return;
        };
        if !self.pending.is_empty() {
            let rest = need - self.pending.len().min(need);
            self.push(&vec![0.0; rest], to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converting_keeps_the_length() {
        let mut converter = Converter::new(44_100, 48_000).unwrap();
        let mut out = Vec::new();
        for piece in vec![0.1f32; 44_100].chunks(441) {
            converter.push(piece, &mut out);
        }
        converter.finish(&mut out);
        // A second, give or take the last chunk's padding.
        assert!((48_000..48_000 + 1200).contains(&out.len()), "{}", out.len());
        let mut same = Converter::new(48_000, 48_000).unwrap();
        let mut out = Vec::new();
        same.push(&[0.5; 100], &mut out);
        same.finish(&mut out);
        assert_eq!(out.len(), 100);
    }
}
