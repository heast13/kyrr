//! Switching the noise model while a stream runs. The new model loads on a
//! thread of its own; the audio thread picks the finished filter up between
//! blocks and hands the old one back, so it never loads, allocates or frees.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kyrr_common::TOO_SLOW;
use kyrr_denoise::{Model, VoiceFilter};

use crate::apo::{build_filter, format_label, report};
use crate::log::log;
use crate::settings::live;

const NO_MODEL: u8 = u8::MAX;

/// How long a loaded filter waits for the audio thread; a stream that
/// stopped meanwhile never takes it.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(5);

fn code(model: Option<Model>) -> u8 {
    match model {
        Some(Model::Quality) => 0,
        Some(Model::Light) => 1,
        None => NO_MODEL,
    }
}

pub fn model_for(light: bool) -> Model {
    if light {
        Model::Light
    } else {
        Model::Quality
    }
}

#[derive(Clone, Copy)]
struct Format {
    rate: u32,
    channels: usize,
    max_frames: usize,
}

/// One APO instance's hand-over slots.
pub struct Swap {
    /// Built for the current format, waiting for the audio thread; tagged
    /// with the stream generation it was built for.
    incoming: Mutex<Option<(VoiceFilter, Model, u32)>>,
    /// Replaced by the audio thread, waiting to be freed off it.
    outgoing: Mutex<Option<VoiceFilter>>,
    /// The stream format while locked for processing.
    format: Mutex<Option<Format>>,
    /// Counts locks and unlocks, so a filter built for an earlier stream
    /// (another rate or channel count) is never swapped into a later one.
    generation: AtomicU32,
    /// Model of the filter running now.
    model: AtomicU8,
    loading: AtomicBool,
    /// The running filter takes most of the audio's time on this computer
    /// and is left out until another one comes; and whether the app was
    /// told (by the settings watcher, see `report_too_slow`).
    too_slow: AtomicBool,
    reported: AtomicBool,
}

/// Every live instance, for the settings watcher.
static SWAPS: Mutex<Vec<Weak<Swap>>> = Mutex::new(Vec::new());

/// Clears `loading` however the loader thread ends, a panic included.
struct Loading<'a>(&'a AtomicBool);

impl Drop for Loading<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Swap {
    pub fn new() -> Arc<Swap> {
        let swap = Arc::new(Swap {
            incoming: Mutex::new(None),
            outgoing: Mutex::new(None),
            format: Mutex::new(None),
            generation: AtomicU32::new(0),
            model: AtomicU8::new(NO_MODEL),
            loading: AtomicBool::new(false),
            too_slow: AtomicBool::new(false),
            reported: AtomicBool::new(false),
        });
        if let Ok(mut all) = SWAPS.lock() {
            all.retain(|w| w.strong_count() > 0);
            all.push(Arc::downgrade(&swap));
        }
        swap
    }

    /// The stream was locked with this format and runs `model`.
    pub fn locked(&self, rate: u32, channels: usize, max_frames: usize, model: Option<Model>) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut f) = self.format.lock() {
            *f = Some(Format {
                rate,
                channels,
                max_frames,
            });
        }
        self.model.store(code(model), Ordering::Release);
        self.too_slow.store(false, Ordering::Release);
        self.reported.store(false, Ordering::Release);
    }

    /// On the audio thread: whether the running filter is left out.
    pub fn too_slow(&self) -> bool {
        self.too_slow.load(Ordering::Acquire)
    }

    /// On the audio thread: the running filter is too slow, from now on.
    pub fn set_too_slow(&self) {
        self.too_slow.store(true, Ordering::Release);
    }

    pub fn unlocked(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut f) = self.format.lock() {
            *f = None;
        }
        if let Ok(mut incoming) = self.incoming.lock() {
            *incoming = None;
        }
    }

    /// On the audio thread: swaps in a filter that finished loading for this
    /// stream. Never waits and never frees: when a slot is busy or the old
    /// filter has not been collected yet, the next block tries again. True
    /// when it swapped.
    pub fn take(&self, filter: &mut Option<VoiceFilter>) -> bool {
        let Ok(mut incoming) = self.incoming.try_lock() else {
            return false;
        };
        let current = self.generation.load(Ordering::Acquire);
        if !matches!(&*incoming, Some((_, _, generation)) if *generation == current) {
            return false;
        }
        let Ok(mut outgoing) = self.outgoing.try_lock() else {
            return false;
        };
        if outgoing.is_some() {
            return false;
        }
        let Some((new, model, _)) = incoming.take() else {
            return false;
        };
        *outgoing = filter.replace(new);
        self.model.store(code(Some(model)), Ordering::Release);
        self.too_slow.store(false, Ordering::Release);
        self.reported.store(false, Ordering::Release);
        true
    }
}

/// Called by the settings watcher every second: tells the app about a
/// stream whose filter turned out too slow, once, so it switches to the
/// light model.
pub fn report_too_slow() {
    let swaps: Vec<Arc<Swap>> = match SWAPS.lock() {
        Ok(all) => all.iter().filter_map(Weak::upgrade).collect(),
        Err(_) => return,
    };
    for swap in swaps {
        if !swap.too_slow.load(Ordering::Acquire) || swap.reported.swap(true, Ordering::AcqRel) {
            continue;
        }
        let Some(format) = swap.format.lock().ok().and_then(|f| *f) else {
            continue;
        };
        let model = match swap.model.load(Ordering::Acquire) {
            0 => Model::Quality.label(),
            1 => Model::Light.label(),
            _ => "",
        };
        let label = format_label(format.rate, format.channels);
        log(&format!(
            "the {model} model is too slow for this computer, audio passes through: {label}"
        ));
        report(&format!("{TOO_SLOW} for the {model} model"), "", &label);
    }
}

/// Called by the settings watcher: loads the chosen model for every
/// running stream that has the other one.
pub fn settings_changed(light: bool) {
    let wanted = model_for(light);
    let swaps: Vec<Arc<Swap>> = match SWAPS.lock() {
        Ok(all) => all.iter().filter_map(Weak::upgrade).collect(),
        Err(_) => return,
    };
    for swap in swaps {
        let generation = swap.generation.load(Ordering::Acquire);
        let Some(format) = swap.format.lock().ok().and_then(|f| *f) else {
            continue;
        };
        if swap.model.load(Ordering::Acquire) == code(Some(wanted)) || swap.loading.swap(true, Ordering::AcqRel) {
            continue;
        }
        let spawned = std::thread::Builder::new().name("kyrr-apo-model".into()).spawn({
            let swap = Arc::clone(&swap);
            move || {
                let switched = {
                    let _loading = Loading(&swap.loading);
                    load(&swap, format, generation, wanted)
                };
                // The user may have switched back while this model loaded.
                if switched && model_for(live().get().light_model) != wanted {
                    settings_changed(live().get().light_model);
                }
            }
        });
        if spawned.is_err() {
            swap.loading.store(false, Ordering::Release);
        }
    }
}

/// Builds the filter with `model` and hands it to the audio thread; true
/// once the stream runs it.
fn load(swap: &Swap, format: Format, generation: u32, model: Model) -> bool {
    let label = format_label(format.rate, format.channels);
    let (filter, engine) = match build_filter(format.rate, format.channels, format.max_frames, model) {
        Ok(built) => built,
        Err(e) => {
            log(&format!(
                "switching to the {} model failed, keeping the old one: {e}",
                model.label()
            ));
            return false;
        }
    };
    match swap.incoming.lock() {
        Ok(mut incoming) if swap.generation.load(Ordering::Acquire) == generation => {
            *incoming = Some((filter, model, generation))
        }
        _ => return false,
    }
    let start = Instant::now();
    while start.elapsed() < HANDOVER_TIMEOUT {
        std::thread::sleep(Duration::from_millis(20));
        let taken = swap.incoming.lock().map_or(true, |i| i.is_none());
        if taken {
            if let Ok(mut outgoing) = swap.outgoing.lock() {
                drop(outgoing.take());
            }
            if swap.model.load(Ordering::Acquire) == code(Some(model)) {
                log(&format!("switched: {label}, engine {engine}"));
                report("running", &engine, &label);
                return true;
            }
            return false;
        }
    }
    if let Ok(mut incoming) = swap.incoming.lock() {
        *incoming = None;
    }
    false
}
