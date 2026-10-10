//! The dictation worker: owns the speech model and the recording, reacts to
//! the push-to-talk key and reports its state to the window.
//!
//! Problems with one dictation (microphone busy, nothing recognised, typing
//! blocked) are reported and the worker stays ready for the next; only a
//! missing or broken model is a lasting error, with a retry.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use kyrr_dictate::download::{download, DownloadError};
use kyrr_dictate::{Dictation, Language};

use crate::recorder::{Recording, MAX_SECONDS};

/// Keeps recording this long after the key is released, so the last
/// syllable is not cut off.
const TAIL: Duration = Duration::from_millis(250);

/// The model (about 1 GB) is let go after this long without dictation and
/// loaded again on the next key press, while the recording already runs.
const IDLE_UNLOAD: Duration = Duration::from_secs(15 * 60);

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
    /// Something to tell the user; `None` clears an earlier message.
    Message(Option<String>),
}

pub enum Command {
    Enable(bool),
    Language(Language),
    Download,
    CancelDownload,
    /// After an error: download again, or load the model again.
    Retry,
    /// Endpoint GUID of the microphone to record (the one the filter is on).
    Microphone(Option<String>),
    KeyDown,
    KeyUp,
    Quit,
    /// Internal: download number `.0` finished.
    Downloaded(u64, Result<(), DownloadError>),
}

pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;

/// Types the text into the focused window. Returns false if that failed.
pub type Typer = fn(&str) -> bool;

/// Puts text on the clipboard. Returns false if that failed.
pub type Clipboard = fn(&str) -> bool;

/// What the worker says, in the dictation language (which is also the
/// language of the window).
fn say(language: Language, what: &str) -> String {
    let german = language == Language::German;
    match (what, german) {
        ("offline", true) => "Kein Internet oder der Server antwortet nicht. Nochmal versuchen?",
        ("offline", false) => "No internet, or the server does not answer. Try again?",
        ("server", true) => "Der Server hat das Sprachmodell gerade nicht. Später nochmal versuchen.",
        ("server", false) => "The server does not have the speech model right now. Try again later.",
        ("corrupt", true) => "Der Download kam beschädigt an. Nochmal versuchen?",
        ("corrupt", false) => "The download arrived damaged. Try again?",
        ("disk", true) => "Konnte das Sprachmodell nicht speichern. Ist die Festplatte voll?",
        ("disk", false) => "Could not save the speech model. Is the disk full?",
        ("load", true) => "Das Sprachmodell ließ sich nicht laden. Nochmal versuchen?",
        ("load", false) => "The speech model would not load. Try again?",
        ("mic", true) => "Konnte das Mikrofon nicht öffnen. Steckt es noch?",
        ("mic", false) => "Could not open the microphone. Is it still plugged in?",
        ("crash", true) => "Das hat nicht geklappt. Nochmal sprechen?",
        ("crash", false) => "That did not work. Say it again?",
        ("clipboard", true) => "Konnte hier nicht tippen. Der Text liegt in der Zwischenablage: Strg+V.",
        ("clipboard", false) => "Could not type here. The text is on the clipboard: Ctrl+V.",
        ("blocked", true) => "Konnte hier nicht tippen (Fenster mit Adminrechten?).",
        ("blocked", false) => "Could not type here (a window running as administrator?).",
        ("cut", true) => "Nach zwei Minuten ist Schluss: Der Rest kam nicht mit.",
        ("cut", false) => "Two minutes is the limit: the rest was not written.",
        (other, _) => other,
    }
    .to_string()
}

struct Worker {
    root: PathBuf,
    language: Language,
    enabled: bool,
    model: Option<Dictation>,
    recording: Option<Recording>,
    microphone: Option<String>,
    download_cancel: Option<Arc<AtomicBool>>,
    /// Number of the current download, so a cancelled one finishing late
    /// does not end a newer one.
    download_id: u64,
    /// A download thread that was cancelled but has not finished yet; a new
    /// download waits for it, both would write the same files.
    download_running: bool,
    download_again: bool,
    last_used: Instant,
    tx: mpsc::Sender<Command>,
    notify: Notify,
    typer: Typer,
    clipboard: Clipboard,
    state: State,
}

pub fn spawn(
    root: PathBuf,
    language: Language,
    enabled: bool,
    notify: Notify,
    typer: Typer,
    clipboard: Clipboard,
) -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let mut worker = Worker {
        root,
        language,
        enabled,
        model: None,
        recording: None,
        microphone: None,
        download_cancel: None,
        download_id: 0,
        download_running: false,
        download_again: false,
        last_used: Instant::now(),
        tx: tx.clone(),
        notify,
        typer,
        clipboard,
        state: State::Off,
    };
    std::thread::Builder::new()
        .name("kyrr-dictation".into())
        .spawn(move || {
            worker.refresh();
            loop {
                match rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(cmd) => worker.handle(cmd),
                    Err(mpsc::RecvTimeoutError::Timeout) => worker.unload_when_idle(),
                }
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

    fn tell(&self, what: &str) {
        (self.notify)(Event::Message(Some(say(self.language, what))));
    }

    /// A lasting problem: the message stays until a retry.
    fn error(&mut self, what: &str) {
        self.tell(what);
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
        match self.load() {
            Ok(()) => {
                (self.notify)(Event::Message(None));
                self.set_state(State::Ready);
            }
            Err(()) => self.error("load"),
        }
    }

    fn load(&mut self) -> Result<(), ()> {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
        let model = Dictation::load(&self.root, self.language, threads).map_err(|_| ())?;
        self.model = Some(model);
        self.last_used = Instant::now();
        Ok(())
    }

    /// Lets the model go after a long pause; the state stays Ready and the
    /// next key press loads it again.
    fn unload_when_idle(&mut self) {
        if self.state == State::Ready && self.model.is_some() && self.last_used.elapsed() >= IDLE_UNLOAD {
            self.model = None;
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
            Command::Microphone(guid) => self.microphone = guid,
            Command::Download => self.start_download(),
            Command::CancelDownload => {
                self.cancel_download();
                self.refresh();
            }
            Command::Retry => {
                (self.notify)(Event::Message(None));
                if self.language.model().is_installed(&self.root) {
                    self.refresh();
                } else {
                    self.start_download();
                }
            }
            Command::Downloaded(id, result) => {
                if id != self.download_id {
                    // A cancelled download is done with its files now.
                    self.download_running = false;
                    if std::mem::take(&mut self.download_again) {
                        self.start_download();
                    }
                    return;
                }
                self.download_running = false;
                self.download_cancel = None;
                match result {
                    Ok(()) | Err(DownloadError::Cancelled) => self.refresh(),
                    Err(DownloadError::Offline(_)) => self.error("offline"),
                    Err(DownloadError::Server(_)) => self.error("server"),
                    Err(DownloadError::Corrupt(_)) => self.error("corrupt"),
                    Err(DownloadError::Disk(_)) => self.error("disk"),
                }
            }
            Command::KeyDown => self.key_down(),
            Command::KeyUp => self.key_up(),
            Command::Quit => {}
        }
    }

    fn key_down(&mut self) {
        if self.state != State::Ready || self.recording.is_some() {
            return;
        }
        match Recording::start(self.microphone.as_deref()) {
            Ok(rec) => {
                self.recording = Some(rec);
                (self.notify)(Event::Message(None));
                self.set_state(State::Recording);
                // Asleep after a long pause: load while the recording runs.
                if self.model.is_none() && self.load().is_err() {
                    self.recording = None;
                    self.error("load");
                }
            }
            Err(_) => self.tell("mic"),
        }
    }

    fn key_up(&mut self) {
        let Some(rec) = self.recording.take() else {
            return;
        };
        std::thread::sleep(TAIL);
        let (rate, samples) = rec.finish();
        self.set_state(State::Transcribing);
        let text = match &self.model {
            Some(model) => match catch_unwind(AssertUnwindSafe(|| model.transcribe(rate, &samples))) {
                Ok(text) => text,
                Err(_) => {
                    self.tell("crash");
                    String::new()
                }
            },
            None => String::new(),
        };
        self.last_used = Instant::now();
        if !text.is_empty() {
            // A space to go on with, unless the text ends with a line break.
            let typed = if text.ends_with('\n') {
                text.clone()
            } else {
                format!("{text} ")
            };
            if (self.typer)(&typed) {
                (self.notify)(Event::Typed(text));
            } else if (self.clipboard)(&text) {
                self.tell("clipboard");
            } else {
                self.tell("blocked");
            }
        }
        if rate > 0 && samples.len() >= rate as usize * MAX_SECONDS {
            self.tell("cut");
        }
        // The language may have changed while recording: back to whatever
        // the settings say, not blindly to Ready.
        self.refresh();
    }

    fn start_download(&mut self) {
        if self.download_cancel.is_some() {
            return;
        }
        if self.download_running {
            self.download_again = true;
            self.set_state(State::Downloading);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.download_cancel = Some(Arc::clone(&cancel));
        self.download_running = true;
        self.download_id += 1;
        let id = self.download_id;
        (self.notify)(Event::Message(None));
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
            let _ = tx.send(Command::Downloaded(id, result));
        });
    }

    /// Stops the download; its thread reports back when it let go of the
    /// files (see `download_running`).
    fn cancel_download(&mut self) {
        if let Some(cancel) = self.download_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
            self.download_id += 1;
            self.download_again = false;
        }
    }
}
