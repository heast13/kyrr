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

/// While the key is held, the newest stretch of the recording, this long,
/// is recognised again and again for the wave on the screen (the pill shows
/// the last few words only).
const PREVIEW_SECONDS: f32 = 8.0;

/// First look at the recording after the key goes down, and the shortest
/// rest between two looks. A look that took longer rests as long as it
/// took, so a slow computer works on it at most half the time.
const PREVIEW_FIRST: Duration = Duration::from_millis(700);
const PREVIEW_REST: Duration = Duration::from_millis(400);

/// The words of the stretch just heard after the words heard before. A
/// stretch that is the whole recording replaces them. Otherwise it starts
/// somewhere in the middle of them (perhaps in the middle of a word): the
/// longest run of words both have ties them together, and from that run on
/// the newer words count, since the model hears the last words better once
/// it knows what follows. Without such a run (a long pause before the
/// stretch) the newer words follow the older ones.
fn merge(before: &[String], heard: Vec<String>, whole: bool) -> Vec<String> {
    if whole || before.is_empty() {
        return heard;
    }
    let key = |w: &String| {
        w.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let old: Vec<String> = before.iter().map(key).collect();
    let new: Vec<String> = heard.iter().map(key).collect();
    // (length, start in before, start in heard) of the longest run.
    let mut best = (0, 0, 0);
    // The stretch is a few seconds: its run is among the last words.
    let from = old.len().saturating_sub(4 * new.len() + 8);
    for i in from..old.len() {
        for j in 0..new.len() {
            let n = old[i..]
                .iter()
                .zip(&new[j..])
                .take_while(|(a, b)| a == b && !a.is_empty())
                .count();
            if n > best.0 {
                best = (n, i, j);
            }
        }
    }
    let (n, i, j) = best;
    if n < 2 {
        return before.iter().cloned().chain(heard).collect();
    }
    // Words at the very start of the stretch are heard without what came
    // before them (a capital, a lost comma): there the older words stay.
    let mut out = before[..i].to_vec();
    for (k, word) in heard.into_iter().enumerate().skip(j) {
        let older = before.get(i + k - j).filter(|_| k < j + n && k < CONTEXT);
        out.push(older.cloned().unwrap_or(word));
    }
    out
}

/// The first words of a stretch, this many, are heard without enough
/// before them.
const CONTEXT: usize = 3;

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

/// Numbers about a recording for the log, to tell a microphone that opened
/// late or sounds too quiet from a model that misheard. Never the audio.
fn describe(rate: u32, samples: &[f32]) -> String {
    if rate == 0 || samples.is_empty() {
        return "empty recording".into();
    }
    let db = |x: f32| 20.0 * x.max(1e-6).log10();
    let empty = samples.iter().take_while(|s| **s == 0.0).count();
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    format!(
        "{:.1} s at {rate} Hz, first {:.2} s empty, peak {:.0} dBFS, level {:.0} dBFS",
        samples.len() as f32 / rate as f32,
        empty as f32 / rate as f32,
        db(peak),
        db(rms)
    )
}

pub enum Event {
    State(State),
    Progress(f32),
    /// Text that was typed.
    Typed(String),
    /// The words heard so far while the key is held, oldest first.
    Heard(Vec<String>),
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
    /// Windows has the microphone muted (or not).
    Muted(bool),
    /// Show the words heard so far while the key is held (or not).
    Live(bool),
    KeyDown,
    KeyUp,
    Quit,
    /// Internal: download number `.0` finished.
    Downloaded(u64, Result<(), DownloadError>),
}

pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;

/// Puts the text into the focused window. Returns false if that failed.
pub type Typer = fn(&str) -> bool;

/// Puts text on the clipboard. Returns false if that failed.
pub type Clipboard = fn(&str) -> bool;

/// What the worker says, in the dictation language (which is also the
/// language of the window).
fn say(language: Language, what: &str) -> String {
    let german = language == Language::German;
    if what == "cut" {
        let minutes = MAX_SECONDS / 60;
        return if german {
            format!("Nach {minutes} Minuten ist Schluss: Der Rest kam nicht mit.")
        } else {
            format!("{minutes} minutes is the limit: the rest was not written.")
        };
    }
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
        ("clipboard", true) => "Konnte hier nicht einfügen. Der Text liegt in der Zwischenablage: Strg+V.",
        ("clipboard", false) => "Could not paste here. The text is on the clipboard: Ctrl+V.",
        ("blocked", true) if cfg!(windows) => "Konnte hier nicht einfügen (Fenster mit Adminrechten?).",
        ("blocked", false) if cfg!(windows) => "Could not paste here (a window running as administrator?).",
        ("blocked", true) => "Konnte hier nicht tippen, und auch die Zwischenablage war nicht erreichbar.",
        ("blocked", false) => "Could not type here, and the clipboard was out of reach too.",
        ("muted", true) if cfg!(windows) => {
            "Dein Mikrofon ist in Windows stummgeschaltet. Auf der Hauptseite kannst du das aufheben."
        }
        ("muted", false) if cfg!(windows) => {
            "Your microphone is muted in Windows. You can switch that off on the main page."
        }
        ("muted", true) => "Dein Mikrofon ist stummgeschaltet. Auf der Hauptseite kannst du das aufheben.",
        ("muted", false) => "Your microphone is muted. You can switch that off on the main page.",
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
    muted: bool,
    live: bool,
    download_cancel: Option<Arc<AtomicBool>>,
    /// Number of the current download, so a cancelled one finishing late
    /// does not end a newer one.
    download_id: u64,
    /// A download thread that was cancelled but has not finished yet; a new
    /// download waits for it, both would write the same files.
    download_running: bool,
    download_again: bool,
    /// The new version of an installed model was fetched without asking;
    /// once is enough, a cancel or error leaves it to the user.
    upgrade_started: bool,
    last_used: Instant,
    /// The words heard so far in the running recording, and when to look
    /// again.
    heard: Vec<String>,
    next_look: Instant,
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
    live: bool,
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
        muted: false,
        live,
        download_cancel: None,
        download_id: 0,
        download_running: false,
        download_again: false,
        upgrade_started: false,
        last_used: Instant::now(),
        heard: Vec::new(),
        next_look: Instant::now(),
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
                let wait = worker.until_look().unwrap_or(Duration::from_secs(60));
                match rx.recv_timeout(wait) {
                    Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(cmd) => worker.handle(cmd),
                    Err(mpsc::RecvTimeoutError::Timeout) if worker.until_look().is_some() => worker.look(),
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
        let spec = self.language.model();
        if !spec.is_installed(&self.root) {
            if !self.upgrade_started && spec.replaces_installed(&self.root) {
                self.upgrade_started = true;
                self.start_download();
            } else {
                self.set_state(State::NoModel);
            }
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

    /// Time until the next look at the running recording; None while
    /// nothing is recorded, the model is not there, or the words are not
    /// wanted.
    fn until_look(&self) -> Option<Duration> {
        (self.live && self.recording.is_some() && self.model.is_some())
            .then(|| self.next_look.saturating_duration_since(Instant::now()))
    }

    /// Recognises the newest stretch of the running recording and shows
    /// the words on the screen.
    fn look(&mut self) {
        let (Some(rec), Some(model)) = (&self.recording, &self.model) else {
            return;
        };
        let started = Instant::now();
        let (rate, samples, whole) = rec.latest(PREVIEW_SECONDS);
        let text = catch_unwind(AssertUnwindSafe(|| model.transcribe(rate, &samples))).unwrap_or_default();
        let heard = text.split_whitespace().map(str::to_owned).collect();
        self.heard = merge(&self.heard, heard, whole);
        (self.notify)(Event::Heard(self.heard.clone()));
        self.next_look = Instant::now() + PREVIEW_REST.max(started.elapsed());
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
            Command::Muted(on) => self.muted = on,
            Command::Live(on) => {
                self.live = on;
                if !on && self.recording.is_some() {
                    self.heard.clear();
                    (self.notify)(Event::Heard(Vec::new()));
                }
            }
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
                    Ok(()) => {
                        self.language.model().remove_replaced(&self.root);
                        self.refresh();
                    }
                    Err(DownloadError::Cancelled) => self.refresh(),
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
                self.heard.clear();
                self.next_look = Instant::now() + PREVIEW_FIRST;
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
        let started = Instant::now();
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
        eprintln!(
            "kyrr: dictation: {}, recognized in {:.1} s",
            describe(rate, &samples),
            started.elapsed().as_secs_f32()
        );
        let heard = !text.is_empty();
        if heard {
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
        if !heard && self.muted {
            self.tell("muted");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn merged(before: &str, heard: &str, whole: bool) -> String {
        merge(&words(before), words(heard), whole).join(" ")
    }

    #[test]
    fn the_whole_recording_replaces_what_was_heard() {
        assert_eq!(
            merged("Hallo wie geht", "Hallo, wie geht es", true),
            "Hallo, wie geht es"
        );
        assert_eq!(merged("", "eins zwei", false), "eins zwei");
    }

    #[test]
    fn a_stretch_joins_at_the_words_both_heard() {
        // The stretch starts in the middle of "wollte" and hears "das" better.
        assert_eq!(
            merged(
                "Hallo Anna, ich wollte dir nur kurz sagen das",
                "te dir nur kurz sagen, dass wir morgen",
                false
            ),
            "Hallo Anna, ich wollte dir nur kurz sagen, dass wir morgen"
        );
    }

    #[test]
    fn words_at_the_start_of_a_stretch_keep_their_older_form() {
        assert_eq!(
            merged(
                "der Zug ist leider ausgebucht, deshalb nehmen",
                "Deshalb nehmen wir das Auto",
                false
            ),
            "der Zug ist leider ausgebucht, deshalb nehmen wir das Auto"
        );
    }

    #[test]
    fn the_latest_matching_run_is_not_fooled_by_a_single_word() {
        // "und" alone is no tie; "und dann" is.
        assert_eq!(
            merged("eins und zwei und dann drei", "dann und dann vier", false),
            "eins und zwei und dann vier"
        );
    }

    #[test]
    fn after_a_long_pause_the_new_words_follow() {
        assert_eq!(merged("eins zwei drei", "vier fünf", false), "eins zwei drei vier fünf");
    }
}
