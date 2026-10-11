//! The before/after probe: kyrr opens the microphone for a few seconds, the
//! audio filter records the same take unprocessed and processed, and kyrr
//! plays either half when asked. Nothing is saved; the take lives in memory
//! until the next probe or quit.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kyrr_common::monitor::PROBE_SECONDS;

use crate::platform::{Monitor, Take};
use crate::player;
use crate::recorder::Recording;

/// The stream needs a moment to start and the filter a moment to claim
/// the probe; give up well after the take should be done.
const TIMEOUT: Duration = Duration::from_secs(PROBE_SECONDS as u64 + 6);

#[derive(Debug)]
pub enum Event {
    Recording,
    Before,
    After,
    Done(Arc<Take>),
    Failed(String),
}

/// Which half of a take to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    Before,
    After,
}

/// Records a probe on `microphone` on its own thread, without playing it:
/// the before and after buttons do that. `notify` gets every step.
pub fn record(microphone: Option<String>, notify: impl Fn(Event) + Send + 'static) {
    std::thread::spawn(move || match take(microphone.as_deref(), &notify) {
        Ok(take) => notify(Event::Done(Arc::new(take))),
        Err(e) => notify(Event::Failed(e)),
    });
}

/// Plays one half of a take on its own thread.
pub fn replay(take: Arc<Take>, half: Half, notify: impl Fn(Event) + Send + 'static) {
    std::thread::spawn(move || {
        play(&take, half, &notify);
        notify(Event::Done(take));
    });
}

fn take(microphone: Option<&str>, notify: &impl Fn(Event)) -> Result<Take, String> {
    // Opening the microphone starts the filter, which creates the monitor.
    let recording = Recording::start(microphone)?;
    notify(Event::Recording);
    let start = Instant::now();
    let mut asked: Option<(Monitor, u32)> = None;
    while start.elapsed() < TIMEOUT {
        std::thread::sleep(Duration::from_millis(50));
        match &asked {
            None => {
                if let Some(monitor) = Monitor::open() {
                    let id = monitor.request_probe();
                    asked = Some((monitor, id));
                }
            }
            Some((monitor, id)) => {
                if let Some(take) = monitor.probe(*id) {
                    drop(recording);
                    return Ok(take);
                }
            }
        }
    }
    drop(recording);
    Err(if asked.is_none() {
        "the audio filter is not running on this microphone".into()
    } else {
        "the audio filter did not record the probe".into()
    })
}

fn play(take: &Take, half: Half, notify: &impl Fn(Event)) {
    let (event, samples) = match half {
        Half::Before => (Event::Before, &take.before),
        Half::After => (Event::After, &take.after),
    };
    notify(event);
    if let Err(e) = player::play(take.rate, samples, &AtomicBool::new(false)) {
        notify(Event::Failed(e));
    }
}
