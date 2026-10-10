//! The dictation worker: owns the speech model and the recording, reacts to
//! the push-to-talk key and reports its state to the window.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use kyrr_dictate::download::{download, DownloadError};
use kyrr_dictate::{Dictation, Language};

use crate::recorder::Recording;

/// Keeps recording this long after the key is released, so the last
/// syllable is not cut off.
const TAIL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    NoModel,
    Downloading,
    Loading,
    Ready,
    Recording,
    Transcribing,
    Error,
}

pub enum Event {
    State(State),
    Progress(f32),
    /// Text that was typed.
    Typed(String),
    Error(String),
}

pub enum Command {
    Enable(bool),
    Language(Language),
    Download,
    CancelDownload,
    KeyDown,
    KeyUp,
    Quit,
    /// Internal: download number `.0` finished.
    Downloaded(u64, Result<(), String>),
}

pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;

/// Types the text into the focused window. Returns false if that failed.
pub type Typer = fn(&str) -> bool;

struct Worker {
    root: PathBuf,
    language: Language,
    enabled: bool,
    model: Option<Dictation>,
    recording: Option<Recording>,
    download_cancel: Option<Arc<AtomicBool>>,
    /// Number of the current download, so a cancelled one finishing late
    /// does not end a newer one.
    download_id: u64,
    tx: mpsc::Sender<Command>,
    notify: Notify,
    typer: Typer,
    state: State,
}

pub fn spawn(root: PathBuf, language: Language, enabled: bool, notify: Notify, typer: Typer) -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let mut worker = Worker {
        root,
        language,
        enabled,
        model: None,
        recording: None,
        download_cancel: None,
        download_id: 0,
        tx: tx.clone(),
        notify,
        typer,
        state: State::Off,
    };
    std::thread::Builder::new()
        .name("kyrr-dictation".into())
        .spawn(move || {
            worker.refresh();
            while let Ok(cmd) = rx.recv() {
                if matches!(cmd, Command::Quit) {
                    break;
                }
                worker.handle(cmd);
            }
            if let Some(cancel) = &worker.download_cancel {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .expect("dictation thread starts");
    tx
}

impl Worker {
    fn set_state(&mut self, state: State) {
        self.state = state;
        (self.notify)(Event::State(state));
    }

    fn error(&mut self, msg: String) {
        (self.notify)(Event::Error(msg));
        self.set_state(State::Error);
    }

    /// Brings the model in line with the settings: loads it when enabled and
    /// installed, drops it when disabled.
    fn refresh(&mut self) {
        if self.download_cancel.is_some() {
            return;
        }
        if !self.enabled {
            self.model = None;
            self.recording = None;
            self.set_state(State::Off);
            return;
        }
        if self.model.as_ref().is_some_and(|m| m.language() == self.language) {
            self.set_state(State::Ready);
            return;
        }
        self.model = None;
        if !self.language.model().is_installed(&self.root) {
            self.set_state(State::NoModel);
            return;
        }
        self.set_state(State::Loading);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
        match Dictation::load(&self.root, self.language, threads) {
            Ok(model) => {
                self.model = Some(model);
                self.set_state(State::Ready);
            }
            Err(e) => self.error(e),
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Enable(on) => {
                self.enabled = on;
                self.refresh();
            }
            Command::Language(language) => {
                if language != self.language {
                    self.language = language;
                    self.cancel_download();
                    self.refresh();
                }
            }
            Command::Download => self.start_download(),
            Command::CancelDownload => {
                self.cancel_download();
                self.refresh();
            }
            Command::Downloaded(id, result) => {
                if id != self.download_id {
                    return;
                }
                self.download_cancel = None;
                match result {
                    Ok(()) => self.refresh(),
                    Err(e) if e == "cancelled" => self.refresh(),
                    Err(e) => self.error(e),
                }
            }
            Command::KeyDown => {
                if self.state != State::Ready {
                    return;
                }
                match Recording::start() {
                    Ok(rec) => {
                        self.recording = Some(rec);
                        self.set_state(State::Recording);
                    }
                    Err(e) => self.error(e),
                }
            }
            Command::KeyUp => {
                let Some(rec) = self.recording.take() else {
                    return;
                };
                std::thread::sleep(TAIL);
                let (rate, samples) = rec.finish();
                self.set_state(State::Transcribing);
                let text = self
                    .model
                    .as_ref()
                    .map(|m| m.transcribe(rate, &samples))
                    .unwrap_or_default();
                if !text.is_empty() {
                    let typed = format!("{text} ");
                    if (self.typer)(&typed) {
                        (self.notify)(Event::Typed(text));
                    }
                }
                self.set_state(State::Ready);
            }
            Command::Quit => {}
        }
    }

    fn start_download(&mut self) {
        if self.download_cancel.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.download_cancel = Some(Arc::clone(&cancel));
        self.download_id += 1;
        let id = self.download_id;
        self.set_state(State::Downloading);
        let (root, spec) = (self.root.clone(), self.language.model());
        let (tx, notify) = (self.tx.clone(), Arc::clone(&self.notify));
        std::thread::spawn(move || {
            let mut last = -1.0f32;
            let result = download(
                spec,
                &root,
                &mut |p| {
                    let f = p.done as f32 / p.total.max(1) as f32;
                    if f - last >= 0.005 {
                        last = f;
                        notify(Event::Progress(f));
                    }
                },
                &cancel,
            );
            let result = result.map_err(|e| match e {
                DownloadError::Cancelled => "cancelled".to_string(),
                DownloadError::Failed(e) => e,
            });
            let _ = tx.send(Command::Downloaded(id, result));
        });
    }

    fn cancel_download(&mut self) {
        if let Some(cancel) = self.download_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
