//! Linux: the voice filter runs inside kyrr, on the virtual microphone
//! "kyrr" (see `kyrr_linux::mic`). The chain is the APO's
//! (`kyrr-apo/src/apo.rs`): voice shaping before, DPDFNet, voice shaping
//! after, with the same levels and before/after take.
//!
//! The audio thread never waits: settings and levels are atomics, a new
//! noise model is handed over through a slot it only tries to lock.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kyrr_common::monitor::PROBE_SECONDS;
use kyrr_common::{FilterSettings, TOO_SLOW};
use kyrr_denoise::{Load, Mode, Model, VoiceFilter};
use kyrr_linux::mic::{Microphone, Status, VirtualMic, RATE};
use kyrr_voice::Voice;

use crate::platform::{Levels, Take};

/// Largest block PipeWire hands over (its maximum quantum); larger ones are
/// split by the filter.
const MAX_BLOCK: usize = 8192;
const PROBE_FRAMES: usize = PROBE_SECONDS as usize * RATE as usize;
/// No audio for this long: nobody recorded from "kyrr" in between, and the
/// next block starts a new recording.
const NEW_RECORDING: Duration = Duration::from_millis(500);

/// What the audio thread and the app share.
struct Shared {
    /// The settings as their stored values, in the order of `to_dwords`.
    settings: [AtomicU32; FilterSettings::VALUES],
    /// A freshly loaded noise filter for the audio thread to take.
    incoming: Mutex<Option<VoiceFilter>>,
    /// The filter it replaced, for the app to free off the audio thread.
    outgoing: Mutex<Option<VoiceFilter>>,
    /// Engine and model of the running filter, or why there is none.
    engine: Mutex<String>,
    heartbeat: AtomicU32,
    before: AtomicU32,
    after: AtomicU32,
    gain_db: AtomicU32,
    far: AtomicBool,
    probe: Mutex<Probe>,
    probe_wanted: AtomicBool,
    /// Counts model loads; a load that finishes after a newer one started
    /// is dropped.
    loads: AtomicU32,
    /// The running filter was too slow for this computer and is left out
    /// until another one comes.
    too_slow: AtomicBool,
}

#[derive(Default)]
struct Probe {
    id: u32,
    before: Vec<f32>,
    after: Vec<f32>,
    done: bool,
}

impl Shared {
    fn settings(&self) -> FilterSettings {
        let names = FilterSettings::default().to_dwords();
        FilterSettings::from_dwords(|name| {
            let i = names.iter().position(|(n, _)| *n == name)?;
            Some(self.settings[i].load(Ordering::Relaxed))
        })
    }

    fn set_settings(&self, s: &FilterSettings) {
        for (slot, (_, value)) in self.settings.iter().zip(s.to_dwords()) {
            slot.store(value, Ordering::Relaxed);
        }
    }
}

/// The audio thread's side.
struct Chain {
    shared: Arc<Shared>,
    voice: Voice,
    filter: Option<VoiceFilter>,
    /// Peaks fall off slowly, so a meter polled now and then sees every word.
    before: f32,
    after: f32,
    /// How much of the audio's time the filter takes, and whether it is
    /// left out as too slow for this computer.
    load: Load,
    slow: bool,
    /// When the last block came.
    last: Option<Instant>,
}

impl Chain {
    fn process(&mut self, buf: &mut [f32]) {
        self.take_new_filter();
        let s = self.shared.settings();
        let recording = self.shared.probe_wanted.load(Ordering::Relaxed);
        if recording {
            self.record(buf, true);
        }
        self.before = decay(self.before, peak(buf));

        // A new recording starts afresh, as each one is a new stream on
        // Windows. The voice may come from somewhere else now (the
        // microphone moved): the automatic level takes a fresh look, and the
        // noise filter, which still expects the last room and level, learns
        // anew. It does that within a word; holding on to the old state, it
        // would take seconds and drop quiet words meanwhile. A stall of the
        // audio in the middle of a recording looks the same and costs only
        // that fresh start.
        let now = Instant::now();
        if self.last.is_some_and(|last| now.duration_since(last) > NEW_RECORDING) {
            self.voice.reset();
            if let Some(filter) = self.filter.as_mut() {
                filter.reset();
            }
            // Its first block after a pause (or a sleep) is slow again.
            self.load.restart();
        }
        self.last = Some(now);
        self.voice.set(&s.voice);
        self.voice.pre(buf);
        // Left out a while: tried again, from a clean start.
        if self.slow && self.load.rest(buf.len(), RATE) {
            if let Some(filter) = self.filter.as_mut() {
                filter.reset();
            }
            self.slow = false;
            self.shared.too_slow.store(false, Ordering::Relaxed);
        }
        if let Some(filter) = self.filter.as_mut().filter(|_| !self.slow) {
            filter.set_mode(if s.enabled { Mode::Process } else { Mode::Bypass });
            filter.set_limit_db(s.limit_db as f32);
            let started = Instant::now();
            filter.process_in_place(buf);
            // Left out for a while: PipeWire's real-time thread would fall
            // behind, and the system ends kyrr when it never gets to rest.
            if self.load.add(started.elapsed(), buf.len(), RATE) {
                self.slow = true;
                self.shared.too_slow.store(true, Ordering::Relaxed);
            }
        }
        self.voice.post(buf);

        self.after = decay(self.after, peak(buf));
        if recording {
            self.record(buf, false);
        }
        let shared = &self.shared;
        shared.before.store(self.before.to_bits(), Ordering::Relaxed);
        shared.after.store(self.after.to_bits(), Ordering::Relaxed);
        shared
            .gain_db
            .store((self.voice.level_db() as f32).to_bits(), Ordering::Relaxed);
        shared.far.store(self.voice.far(), Ordering::Relaxed);
        shared.heartbeat.fetch_add(1, Ordering::Relaxed);
    }

    /// Swaps in a newly loaded filter when one waits and the old one can be
    /// handed back (both models have the same delay, so the voice's
    /// look-ahead stays right).
    fn take_new_filter(&mut self) {
        let (Ok(mut incoming), Ok(mut outgoing)) = (self.shared.incoming.try_lock(), self.shared.outgoing.try_lock())
        else {
            return;
        };
        if outgoing.is_none() {
            if let Some(filter) = incoming.take() {
                *outgoing = self.filter.replace(filter);
                self.load = Load::default();
                self.slow = false;
                self.shared.too_slow.store(false, Ordering::Relaxed);
            }
        }
    }

    /// Appends to the probe without allocating: its buffers were reserved
    /// when it was asked for.
    fn record(&mut self, buf: &[f32], before: bool) {
        let Ok(mut probe) = self.shared.probe.try_lock() else {
            return;
        };
        if probe.done {
            return;
        }
        let half = if before { &mut probe.before } else { &mut probe.after };
        let room = half.capacity() - half.len();
        half.extend_from_slice(&buf[..buf.len().min(room)]);
        if !before && probe.after.len() == probe.after.capacity() {
            probe.done = true;
            self.shared.probe_wanted.store(false, Ordering::Relaxed);
        }
    }
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
}

fn decay(held: f32, now: f32) -> f32 {
    now.max(held * 0.8)
}

/// The running filter: the virtual microphone and its chain.
pub struct Engine {
    mic: VirtualMic,
    shared: Arc<Shared>,
    assets: PathBuf,
    model: Mutex<Option<Model>>,
}

impl Engine {
    /// Starts "kyrr" on `microphone` (a PipeWire node name, `None` for the
    /// default) with `settings`; `remembered` is the system's default from
    /// before (see `VirtualMic::start`).
    pub fn start(
        microphone: Option<String>,
        remembered: Option<String>,
        settings: &FilterSettings,
    ) -> Result<Engine, String> {
        let shared = Arc::new(Shared {
            settings: std::array::from_fn(|_| AtomicU32::new(0)),
            incoming: Mutex::new(None),
            outgoing: Mutex::new(None),
            engine: Mutex::new(String::new()),
            heartbeat: AtomicU32::new(0),
            before: AtomicU32::new(0),
            after: AtomicU32::new(0),
            gain_db: AtomicU32::new(0),
            far: AtomicBool::new(false),
            probe: Mutex::new(Probe::default()),
            probe_wanted: AtomicBool::new(false),
            loads: AtomicU32::new(0),
            too_slow: AtomicBool::new(false),
        });
        shared.set_settings(settings);
        let assets = assets_dir();
        let model = model_for(settings.light_model);
        // The first model loads before the microphone starts, so the
        // voice's look-ahead knows the filter's delay.
        let filter = match build_filter(&assets, model) {
            Ok((filter, label)) => {
                *lock(&shared.engine) = label;
                Some(filter)
            }
            Err(e) => {
                eprintln!("kyrr: noise model did not load, the microphone passes through: {e}");
                *lock(&shared.engine) = format!("passthrough: {e}");
                None
            }
        };
        let mut voice = Voice::new(RATE, 1);
        voice.set_delay(filter.as_ref().map_or(0, VoiceFilter::latency_frames), MAX_BLOCK);
        let mut chain = Chain {
            shared: shared.clone(),
            voice,
            filter,
            before: 0.0,
            after: 0.0,
            load: Load::default(),
            slow: false,
            last: None,
        };
        let mic = VirtualMic::start(
            microphone,
            remembered,
            Box::new(move |buf: &mut [f32]| chain.process(buf)),
        )?;
        Ok(Engine {
            mic,
            shared,
            assets,
            model: Mutex::new(Some(model)),
        })
    }

    pub fn microphones(&self) -> Vec<Microphone> {
        self.mic.microphones()
    }

    /// The system's default microphone from before "kyrr", to remember.
    pub fn previous_default(&self) -> Option<String> {
        self.mic.previous_default()
    }

    /// PipeWire went away under the filter: it needs a new start.
    pub fn gone(&self) -> bool {
        self.mic.gone()
    }

    pub fn status(&self) -> Status {
        self.mic.status()
    }

    pub fn node_id(&self, name: &str) -> Option<u32> {
        self.mic.node_id(name)
    }

    /// The microphone the system would use without "kyrr" (see
    /// [`VirtualMic::system_default`]).
    pub fn system_default(&self) -> Option<String> {
        self.mic.system_default()
    }

    /// Engine and model, e.g. "native, quality"; empty while loading.
    pub fn engine(&self) -> String {
        let label = lock(&self.shared.engine).clone();
        if self.shared.too_slow.load(Ordering::Relaxed) {
            format!("{TOO_SLOW} for {label}")
        } else {
            label
        }
    }

    pub fn use_microphone(&self, name: &str) {
        self.mic.use_microphone(name);
    }

    pub fn set_settings(&self, s: &FilterSettings) {
        self.shared.set_settings(s);
        self.load_model(s.light_model);
    }

    /// Loads the large or small noise model on a thread of its own, unless
    /// it already runs.
    fn load_model(&self, light: bool) {
        let model = model_for(light);
        {
            let mut current = lock(&self.model);
            if *current == Some(model) {
                return;
            }
            *current = Some(model);
        }
        let shared = self.shared.clone();
        let assets = self.assets.clone();
        let load = shared.loads.fetch_add(1, Ordering::Relaxed) + 1;
        std::thread::spawn(move || {
            let built = build_filter(&assets, model);
            // Under the slot's lock, so of two loads finishing together only
            // the newer one lands.
            let mut incoming = lock(&shared.incoming);
            if shared.loads.load(Ordering::Relaxed) != load {
                return;
            }
            match built {
                Ok((filter, label)) => {
                    // An older load still waiting, and the filter the audio
                    // thread gave back, are freed here.
                    let waiting = incoming.replace(filter);
                    drop(incoming);
                    let given_back = lock(&shared.outgoing).take();
                    drop((waiting, given_back));
                    *lock(&shared.engine) = label;
                }
                Err(e) => *lock(&shared.engine) = format!("passthrough: {e}"),
            }
        });
    }

    pub fn levels(&self) -> Levels {
        let s = &self.shared;
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        Levels {
            heartbeat: s.heartbeat.load(Ordering::Relaxed),
            before: f(&s.before),
            after: f(&s.after),
            gain_db: f(&s.gain_db),
            far: s.far.load(Ordering::Relaxed),
        }
    }

    /// Starts a before/after take of the next [`PROBE_SECONDS`] seconds.
    pub fn request_probe(&self) -> u32 {
        let mut probe = lock(&self.shared.probe);
        let id = probe.id.wrapping_add(1).max(1);
        *probe = Probe {
            id,
            before: Vec::with_capacity(PROBE_FRAMES),
            after: Vec::with_capacity(PROBE_FRAMES),
            done: false,
        };
        self.shared.probe_wanted.store(true, Ordering::Relaxed);
        id
    }

    /// The take once it is complete; it is handed out once.
    pub fn probe(&self, id: u32) -> Option<Take> {
        let mut probe = lock(&self.shared.probe);
        if probe.id != id || !probe.done {
            return None;
        }
        let take = Take {
            rate: RATE,
            before: std::mem::take(&mut probe.before),
            after: std::mem::take(&mut probe.after),
        };
        probe.done = false;
        Some(take)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn model_for(light: bool) -> Model {
    if light {
        Model::Light
    } else {
        Model::Quality
    }
}

fn build_filter(assets: &Path, model: Model) -> Result<(VoiceFilter, String), String> {
    let denoiser = kyrr_denoise::load(assets, model, None)?;
    let label = format!("{}, {}", denoiser.engine(), model.label());
    let filter = VoiceFilter::new(denoiser, RATE, 1, MAX_BLOCK)?;
    Ok((filter, label))
}

/// Where the noise models are: `$KYRR_ASSETS`, else `../share/kyrr` next to
/// the program (packages, AppImage), else the program's own folder, else the
/// repository's `assets/` (development builds). Recognised by the packed
/// weights: the AppImage and the tarball leave out the `.onnx` files, which
/// only ONNX Runtime reads.
fn assets_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KYRR_ASSETS") {
        return PathBuf::from(dir);
    }
    let model = Model::Quality.weights_name();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    let candidates = exe_dir
        .iter()
        .flat_map(|d| [d.join("../share/kyrr"), d.clone()])
        .chain([PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")]);
    for dir in candidates {
        if dir.join(model).is_file() {
            return dir;
        }
    }
    PathBuf::from("/usr/share/kyrr")
}
