//! The dictation worker: owns the speech model and the recording, reacts to
//! the push-to-talk key and reports its state to the window.
//!
//! Problems with one dictation (microphone busy, nothing recognised, typing
//! blocked) are reported and the worker stays ready for the next; only a
//! missing or broken model is a lasting error, with a retry.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use kyrr_dictate::download::{download, DownloadError};
use kyrr_dictate::{european, Dictation, Language, Live, ModelSpec, Transcript};
use kyrr_server::rework::{self, Outcome, Preset, Why};
use kyrr_server::Server;

use crate::memo::text;
use crate::recorder::{Recording, MAX_SECONDS};

/// Keeps recording this long after the key is released, so the last
/// syllable is not cut off.
const TAIL: Duration = Duration::from_millis(250);

/// The model (about 1 GB) is let go after this long without dictation and
/// loaded again on the next key press, while the recording already runs.
const IDLE_UNLOAD: Duration = Duration::from_secs(15 * 60);

/// While the key is held, the newest stretch of the recording is
/// recognised again and again for the words on the screen (see [`Live`]).
/// First look after the key goes down, and the shortest rest between two
/// looks. A slow computer rests half as long as a look took, so the words
/// keep up and it still has a third of its time for everything else.
const LOOK_FIRST: Duration = Duration::from_millis(700);
const LOOK_REST: Duration = Duration::from_millis(400);

/// A look that takes longer than this marks a slow computer: the next one
/// looks at a shorter stretch (see [`Live::set_slow`]).
const LOOK_SLOW: Duration = Duration::from_millis(500);

/// While the key is held, the recording is taken in this often, and every
/// piece of it that is complete is recognised then, so a long dictation is
/// typed soon after the key goes up (see [`Transcript`]).
const AHEAD_EVERY: Duration = Duration::from_secs(3);

/// The longest dictation with the lean setting: on the slow computers it is
/// meant for, recognising takes about a quarter of the time spoken, which
/// keeps such a computer busy for half a minute. Otherwise `MAX_SECONDS`. At the limit
/// kyrr types what it heard, as if the key had been let go.
const SHORT_LIMIT: Duration = Duration::from_secs(2 * 60);

/// Text heard while the key is still held (at the limit) waits for it to
/// be let go, as a held Ctrl would turn the typing into shortcuts; at most
/// this long.
const WAIT_FOR_KEY: Duration = Duration::from_secs(3);

/// A computer that needs longer than this for two looks in a row would
/// show the words far too late: the wave goes on without them for this
/// dictation.
const LOOK_TOO_SLOW: Duration = Duration::from_millis(2500);
/// Seconds of recording (the tail after the key's release included) from
/// which an empty result is worth a word in the pill.
const NOTHING_MIN: f32 = 0.8;

/// A tap the worker gets to only this late (it was busy typing a long
/// dictation) is from the past: it would open the microphone unasked until
/// the next tap. A held key still starts late, and its release ends it.
const STALE_PRESS: Duration = Duration::from_secs(1);

/// A rework that has not come back this long after its own time limit (a
/// name that never resolves, say) is given up: the dictation is typed.
const REWORK_GRACE: Duration = Duration::from_secs(1);

/// The user's language model, which reworks every dictation.
#[derive(Clone, Debug)]
pub struct Reworker {
    pub server: Server,
    pub model: String,
    pub preset: Preset,
}

/// The user's speech server, which recognises dictations instead of the
/// built-in model.
#[derive(Clone, Debug)]
pub struct Recognizer {
    pub server: Server,
    pub model: String,
}

/// A dictation out at the language model.
struct Reworking {
    id: u64,
    original: String,
    until: Instant,
    stop: Arc<AtomicBool>,
    started: Instant,
}

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
    /// A dictation started that ends at this moment at the latest.
    Ends(Instant),
    /// The dictation reached its limit: this, in red in the pill.
    Limit(String),
    /// A real dictation that came out empty: this, for a moment in the pill.
    Nothing(String),
    /// Something to tell the user; `None` clears an earlier message.
    Message(Option<String>),
    /// The title heard at the start of the voice memo at this path; None
    /// when nothing fit was said or there is no speech model.
    Titled(PathBuf, Option<String>),
    /// How the text of the voice memo at this path comes along.
    Text(PathBuf, Text),
}

/// How a voice memo's text comes along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    /// It waits for the one before it.
    Waiting,
    /// This far (0 to 1), and about this long still.
    Writing(f32, Option<Duration>),
    /// It lies beside the memo now.
    Written,
    /// Stopped by the user, or the memo went.
    Cancelled,
    /// The speech model is not on the computer.
    NoModel,
    Failed,
}

/// Work on voice memos, done with the speech model beside dictation.
pub enum Job {
    /// The title from the first seconds of a memo, at their rate.
    Title(PathBuf, u32, Vec<f32>),
    /// The whole text of a memo.
    Text(PathBuf),
}

pub enum Command {
    Enable(bool),
    /// The window's language, and whether dictation uses the model for many
    /// European languages instead of the one for that language.
    Language(Language, bool),
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
    /// Dictations end after `SHORT_LIMIT` (the lean setting), or not.
    Short(bool),
    /// The key went down at this moment; true in tap mode (one tap starts,
    /// the next ends).
    KeyDown(Instant, bool),
    /// The dictation key went down (true) or up, whatever it starts or ends.
    Held(bool),
    /// The key went up at this moment.
    KeyUp(Instant),
    Quit,
    /// Internal: download number `.0` finished.
    Downloaded(u64, Result<(), DownloadError>),
    /// Internal: load number `.0` is done, loaded or not.
    Loaded(u64),
    /// Work on a voice memo; titles go first.
    Job(Job),
    /// Stops writing the text of the memo at this path, or takes it out of
    /// the queue.
    CancelText(PathBuf),
    /// Internal: the job beside the worker is done.
    JobDone,
    /// Every dictation goes through the user's language model (or not).
    Rework(Option<Reworker>),
    /// Dictations are recognised by the user's speech server (or not).
    Recognizer(Option<Recognizer>),
    /// Internal: rework number `.0` came back.
    Reworked(u64, Outcome),
}

pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;

/// Puts the text into the focused window. Returns false if that failed.
pub type Typer = fn(&str) -> bool;

/// Puts text on the clipboard. Returns false if that failed.
pub type Clipboard = fn(&str) -> bool;

/// A short grumble for an empty dictation, never the same one twice in a
/// row: kyrr was woken for nothing.
fn nothing(german: bool) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    const DE: [&str; 14] = [
        "Dafür geweckt?",
        "Nix. Danke auch.",
        "Ich hatte Pause.",
        "Ernsthaft? Stille.",
        "Luft diktiert. Bravo.",
        "Rede oder schweig.",
        "Hirn aus?",
        "Ich schlief.",
        "Wecken kostet extra.",
        "Hallo? Jemand da?",
        "Grillenzirpen",
        "Nada",
        "Hä?",
        "Räusper",
    ];
    const EN: [&str; 14] = [
        "Woke for this?",
        "Nothing. Thanks.",
        "I was off.",
        "Seriously? Silence.",
        "Dictated air. Bravo.",
        "Talk or leave.",
        "Brain off?",
        "I slept.",
        "Waking costs extra.",
        "Hello? Anyone?",
        "Crickets",
        "Nada",
        "Huh?",
        "Ahem",
    ];
    static LAST: AtomicUsize = AtomicUsize::new(usize::MAX);
    let words = if german { &DE } else { &EN };
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_micros() as usize);
    let mut pick = clock % words.len();
    // Not the same word twice in a row.
    if pick == LAST.load(Ordering::Relaxed) {
        pick = (pick + 1) % words.len();
    }
    LAST.store(pick, Ordering::Relaxed);
    words[pick].to_string()
}

/// What the worker says, in the language of the window.
fn say(language: Language, what: &str) -> String {
    let german = language == Language::German;
    if let Some(minutes) = what.strip_prefix("limit ") {
        return if german {
            format!("Nach {minutes} Minuten fügt kyrr ein, was bis dahin kam. Für mehr die Taste nochmal drücken.")
        } else {
            format!("After {minutes} minutes kyrr types what it has so far. Press the key again to go on.")
        };
    }
    match (what, german) {
        ("nothing", german) => return nothing(german),
        ("limit", true) => "Maximale Länge erreicht",
        ("limit", false) => "Longest dictation reached",
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
        ("mic gone", true) => "Das Mikrofon war mittendrin weg. Steckt es noch?",
        ("mic gone", false) => "The microphone went away halfway through. Is it still plugged in?",
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
        ("fixing", true) => "Korrigiere…",
        ("fixing", false) => "Polishing…",
        ("polishing", true) => "Formuliere…",
        ("polishing", false) => "Rewording…",
        ("translating", true) => "Übersetze…",
        ("translating", false) => "Translating…",
        ("rework asleep", true) => "Server schläft, Original getippt",
        ("rework asleep", false) => "Server asleep, typed as said",
        ("rework refused", true) => "Server sagt nein, Original getippt",
        ("rework refused", false) => "Server says no, typed as said",
        ("rework free", true) => "Zu frei, Original getippt",
        ("rework free", false) => "Too free, typed as said",
        ("asr local", true) => "Server nicht erreichbar, hier erkannt.",
        ("asr local", false) => "Server out of reach, recognised here.",
        ("asr gone", true) => "Der Diktier-Server antwortet nicht. Läuft er noch?",
        ("asr gone", false) => "The dictation server does not answer. Is it still running?",
        ("muted", false) => "Your microphone is muted. You can switch that off on the main page.",
        (other, _) => other,
    }
    .to_string()
}

struct Worker {
    root: PathBuf,
    language: Language,
    more_languages: bool,
    enabled: bool,
    /// Shared with the job beside the worker (see [`Job`]).
    model: Option<Arc<Dictation>>,
    /// The model loading beside the worker, and the number of that load.
    loader: Option<std::thread::JoinHandle<Result<Dictation, String>>>,
    load_id: u64,
    recording: Option<Recording>,
    microphone: Option<String>,
    muted: bool,
    live: bool,
    /// Dictations end after `SHORT_LIMIT`, not `MAX_SECONDS`.
    short: bool,
    /// When the running dictation reaches its limit, and that limit.
    ends: Instant,
    limit: Duration,
    /// The dictation key is down.
    held: bool,
    /// Text to type once the key is up, and since when.
    waiting: Option<(String, Instant)>,
    download_cancel: Option<Arc<AtomicBool>>,
    /// Number of the current download, so a cancelled one finishing late
    /// does not end a newer one.
    download_id: u64,
    /// A download thread that was cancelled but has not finished yet; a new
    /// download waits for it, both would write the same files.
    download_running: bool,
    download_again: bool,
    /// The installed model would not load: a retry checks its files and
    /// fetches the damaged ones again.
    load_failed: bool,
    /// The new version of an installed model was fetched without asking;
    /// once is enough, a cancel or error leaves it to the user.
    upgrade_started: bool,
    last_used: Instant,
    /// The words shown so far in the running recording, when to look
    /// again, and how long the looks took (for the log).
    words: Live,
    next_look: Option<Instant>,
    looks: Vec<Duration>,
    /// The running recording's text so far, and when to take in more.
    transcript: Option<Transcript>,
    next_ahead: Instant,
    /// Work on voice memos waiting, the one running (with the path of the
    /// memo whose text it writes, and its cancel), and whether it waits for
    /// a dictation.
    jobs: VecDeque<Job>,
    job: Option<(Option<PathBuf>, Arc<AtomicBool>)>,
    pause: Arc<AtomicBool>,
    /// The user's language model and speech server, if set up.
    reworker: Option<Reworker>,
    recognizer: Option<Recognizer>,
    reworking: Option<Reworking>,
    rework_id: u64,
    /// The speech server could not be reached once: said only once.
    told_local: bool,
    tx: mpsc::Sender<Command>,
    notify: Notify,
    typer: Typer,
    clipboard: Clipboard,
    state: State,
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    root: PathBuf,
    language: Language,
    more_languages: bool,
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
        more_languages,
        enabled,
        model: None,
        loader: None,
        load_id: 0,
        recording: None,
        microphone: None,
        muted: false,
        live,
        short: false,
        ends: Instant::now(),
        limit: SHORT_LIMIT,
        held: false,
        waiting: None,
        download_cancel: None,
        download_id: 0,
        download_running: false,
        download_again: false,
        load_failed: false,
        upgrade_started: false,
        last_used: Instant::now(),
        words: Live::default(),
        next_look: None,
        looks: Vec::new(),
        transcript: None,
        next_ahead: Instant::now(),
        jobs: VecDeque::new(),
        job: None,
        pause: Arc::new(AtomicBool::new(false)),
        reworker: None,
        recognizer: None,
        reworking: None,
        rework_id: 0,
        told_local: false,
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
                let wait = [
                    worker.until_end(),
                    worker.until_look(),
                    worker.until_typed(),
                    worker.until_ahead(),
                    worker.until_reworked(),
                ]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(Duration::from_secs(60));
                let due = |d: Option<Duration>| d.is_some_and(|d| d.is_zero());
                let received = rx.recv_timeout(wait);
                if matches!(received, Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected)) {
                    break;
                }
                // A crash in here costs the dictation at hand, not the key
                // for the rest of the day.
                let step = catch_unwind(AssertUnwindSafe(|| {
                    match received {
                        Ok(cmd) => worker.handle(cmd),
                        Err(_) if due(worker.until_end()) => worker.end_at_limit(),
                        Err(_) if due(worker.until_typed()) => worker.type_waiting(),
                        Err(_) if due(worker.until_look()) => worker.look(),
                        Err(_) if due(worker.until_ahead()) => worker.work_ahead(),
                        Err(_) if due(worker.until_reworked()) => worker.rework_late(),
                        Err(_) => worker.unload_when_idle(),
                    }
                    // A job waits while a dictation runs or is being typed.
                    let busy = worker.recording.is_some()
                        || worker.state == State::Transcribing
                        || worker.waiting.is_some()
                        || worker.reworking.is_some();
                    worker.pause.store(busy, Ordering::Relaxed);
                    worker.next_job();
                }));
                if step.is_err() {
                    worker.start_over();
                }
            }
            if let Some((_, cancel)) = &worker.job {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some(cancel) = &worker.download_cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some(r) = &worker.reworking {
                r.stop.store(true, Ordering::Relaxed);
            }
        })
        .expect("dictation thread starts");
    tx
}

impl Worker {
    /// The speech model the settings ask for.
    fn spec(&self) -> &'static ModelSpec {
        if self.more_languages {
            european()
        } else {
            self.language.model()
        }
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
        (self.notify)(Event::State(state));
    }

    fn tell(&self, what: &str) {
        (self.notify)(Event::Message(Some(say(self.language, what))));
    }

    /// After a crash in the worker: the dictation at hand is given up, the
    /// rest goes on as the settings say.
    fn start_over(&mut self) {
        eprintln!("kyrr: dictation: crashed, starting over");
        self.recording = None;
        self.transcript = None;
        self.waiting = None;
        // A dictation out at the language model is typed as it was.
        if let Some(r) = self.reworking.take() {
            r.stop.store(true, Ordering::Relaxed);
            let _ = catch_unwind(AssertUnwindSafe(|| self.type_text(r.original)));
        }
        self.pause.store(false, Ordering::Relaxed);
        let _ = catch_unwind(AssertUnwindSafe(|| self.refresh()));
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
        // A model change (the language) waits until the running dictation
        // is typed: `key_up` comes back here.
        if self.enabled && (self.recording.is_some() || self.reworking.is_some()) {
            return;
        }
        if !self.enabled {
            // Work on memos keeps it until that is done.
            if !self.has_jobs() {
                self.model = None;
            }
            self.transcript = None;
            self.recording = None;
            self.set_state(State::Off);
            return;
        }
        // The speech server recognises: no model needed here (one loaded
        // for a memo's text stays until that is done).
        if self.recognizer.is_some() {
            self.transcript = None;
            self.set_state(State::Ready);
            return;
        }
        let spec = self.spec();
        if self.model.as_ref().is_some_and(|m| m.model() == spec.id) {
            self.set_state(State::Ready);
            return;
        }
        self.model = None;
        // Its pieces were heard by the model that goes.
        self.transcript = None;
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
        self.start_load();
    }

    /// After a model that would not load: fetches the files that are
    /// damaged on the disk again, else tries loading once more.
    fn repair(&mut self) {
        self.set_state(State::Loading);
        let damaged = self.spec().remove_damaged(&self.root);
        self.load_failed = false;
        if damaged {
            eprintln!("kyrr: dictation: the speech model was damaged, fetching it again");
            self.start_download();
        } else {
            self.refresh();
        }
    }

    /// Loads the model beside the worker (a few seconds, more on a slow
    /// disk), which keeps listening to the key: a dictation started
    /// meanwhile records from the first word on and is recognised once the
    /// model is there. `Loaded` says when the load is done.
    fn start_load(&mut self) {
        if self.loader.is_some() {
            return;
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
        let (root, spec, tx) = (self.root.clone(), self.spec(), self.tx.clone());
        self.load_id += 1;
        let id = self.load_id;
        self.loader = Some(std::thread::spawn(move || {
            // Whatever happens in there, the worker hears back.
            let model = catch_unwind(AssertUnwindSafe(|| Dictation::load(&root, spec, threads)))
                .unwrap_or_else(|_| Err("loading crashed".into()));
            let _ = tx.send(Command::Loaded(id));
            model
        }));
    }

    /// Takes the model from the loader, waiting for it if it is still busy
    /// (a dictation ended before the model was there). False only when it
    /// would not load; true also when nothing was loading, or the model was
    /// let go because the settings changed meanwhile.
    fn finish_load(&mut self) -> bool {
        let Some(loader) = self.loader.take() else {
            return true;
        };
        let model = loader
            .join()
            .unwrap_or_else(|_| Err("the loader crashed".into()))
            .map_err(|e| eprintln!("kyrr: dictation: the speech model would not load: {e}"));
        let Ok(model) = model else {
            self.load_failed = true;
            self.recording = None;
            self.transcript = None;
            // Loaded for a memo's text with dictation off: the memo says so.
            if self.enabled {
                self.error("load");
            }
            return false;
        };
        self.load_failed = false;
        self.last_used = Instant::now();
        // The settings changed while it loaded. A dictation in progress
        // still uses it; the switch follows once it is typed.
        let busy = self.recording.is_some() || self.state == State::Transcribing;
        if (!self.enabled && !self.has_jobs()) || (model.model() != self.spec().id && !busy) {
            drop(model);
            self.refresh();
            return true;
        }
        self.model = Some(Arc::new(model));
        // A dictation that runs or waits for it keeps its state.
        if self.state == State::Loading {
            (self.notify)(Event::Message(None));
            self.set_state(State::Ready);
        }
        true
    }

    /// Time until the next look at the running recording; None while
    /// nothing is recorded, the model is not there, or the words are not
    /// wanted.
    fn until_look(&self) -> Option<Duration> {
        let next = self
            .next_look
            .filter(|_| self.live && self.recording.is_some() && self.model.is_some() && self.recognizer.is_none())?;
        Some(next.saturating_duration_since(Instant::now()))
    }

    /// Time until the recording is taken in again; None while nothing is
    /// recorded or the model is not there.
    fn until_ahead(&self) -> Option<Duration> {
        if self.transcript.is_none() || self.recording.is_none() || self.model.is_none() {
            return None;
        }
        Some(self.next_ahead.saturating_duration_since(Instant::now()))
    }

    /// Takes in what was recorded since the last time and recognises a
    /// piece that is complete. One piece at a time: a look at the words or
    /// the key going up may be waiting.
    fn work_ahead(&mut self) {
        let (Some(rec), Some(model), Some(transcript)) = (&self.recording, &self.model, self.transcript.as_mut())
        else {
            return;
        };
        let new = rec.samples_from(transcript.taken());
        let done = catch_unwind(AssertUnwindSafe(|| {
            transcript.push(&new);
            model.ahead(transcript)
        }));
        self.next_ahead = match done {
            // Maybe one more is complete (the model was still loading).
            Ok(true) => Instant::now(),
            Ok(false) => Instant::now() + AHEAD_EVERY,
            // The whole recording is recognised at the end instead.
            Err(_) => {
                self.transcript = None;
                Instant::now()
            }
        };
    }

    /// Time until the running dictation reaches its limit; None while
    /// nothing is recorded.
    fn until_end(&self) -> Option<Duration> {
        self.recording
            .as_ref()
            .map(|_| self.ends.saturating_duration_since(Instant::now()))
    }

    /// The longest dictation with the current settings.
    fn limit(&self) -> Duration {
        if self.short {
            SHORT_LIMIT
        } else {
            Duration::from_secs(MAX_SECONDS as u64)
        }
    }

    /// Time until text that waits for the key is typed anyway.
    fn until_typed(&self) -> Option<Duration> {
        let (_, since) = self.waiting.as_ref()?;
        Some((*since + WAIT_FOR_KEY).saturating_duration_since(Instant::now()))
    }

    /// At the limit: types what was heard, as if the key had been let go.
    fn end_at_limit(&mut self) {
        self.tell(&format!("limit {}", self.limit.as_secs() / 60));
        (self.notify)(Event::Limit(say(self.language, "limit")));
        self.key_up(Instant::now());
    }

    /// Recognises the newest stretch of the running recording and shows
    /// the words that are sure by now.
    fn look(&mut self) {
        let (Some(rec), Some(model)) = (&self.recording, &self.model) else {
            return;
        };
        let started = Instant::now();
        let mut from = 0.0;
        let (rate, samples, to) = rec.since(|length| {
            from = self.words.from(length);
            from
        });
        let heard = catch_unwind(AssertUnwindSafe(|| model.words(rate, &samples))).unwrap_or_default();
        if self.words.heard(from, to, heard) {
            (self.notify)(Event::Heard(self.words.shown().to_vec()));
        }
        let took = started.elapsed();
        self.words.set_slow(took > LOOK_SLOW);
        self.looks.push(took);
        // The first look after loading the model may be slow once.
        let slow = self.looks.iter().rev().take(2).filter(|t| **t > LOOK_TOO_SLOW).count();
        self.next_look = if slow == 2 {
            eprintln!(
                "kyrr: dictation: looks at the words take {:.1} s, too slow to show them",
                took.as_secs_f32()
            );
            None
        } else {
            Some(Instant::now() + LOOK_REST.max(took / 2))
        };
    }

    /// Lets the model go after a long pause; the state stays Ready and the
    /// next key press loads it again.
    fn unload_when_idle(&mut self) {
        if self.state == State::Ready
            && self.model.is_some()
            && !self.has_jobs()
            && self.last_used.elapsed() >= IDLE_UNLOAD
        {
            self.model = None;
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Enable(on) => {
                self.enabled = on;
                self.refresh();
            }
            Command::Language(language, more) => {
                let before = self.spec().id;
                self.language = language;
                self.more_languages = more;
                // The window changes language, the model only if it follows it.
                if self.spec().id != before {
                    self.cancel_download();
                    self.refresh();
                }
            }
            Command::Microphone(guid) => self.microphone = guid,
            Command::Short(on) => self.short = on,
            Command::Held(down) => {
                self.held = down;
                if !down {
                    self.type_waiting();
                }
            }
            Command::Muted(on) => self.muted = on,
            Command::Live(on) => {
                self.live = on;
                if !on && self.recording.is_some() {
                    self.words = Live::default();
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
                if self.spec().is_installed(&self.root) && self.load_failed {
                    self.repair();
                } else if self.spec().is_installed(&self.root) {
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
                        self.spec().remove_replaced(&self.root);
                        self.refresh();
                    }
                    Err(DownloadError::Cancelled) => self.refresh(),
                    Err(DownloadError::Offline(_)) => self.error("offline"),
                    Err(DownloadError::Server(_)) => self.error("server"),
                    Err(DownloadError::Corrupt(_)) => self.error("corrupt"),
                    Err(DownloadError::Disk(_)) => self.error("disk"),
                }
            }
            Command::KeyDown(at, tap) => self.key_down(at, tap),
            Command::KeyUp(at) => self.key_up(at),
            // An older load may have been taken already, by a dictation
            // that ended before it was done.
            Command::Loaded(id) if id == self.load_id => {
                self.finish_load();
            }
            Command::Loaded(_) => {}
            Command::Job(Job::Title(path, rate, samples)) => {
                // Before the texts, which take long.
                let at = self.jobs.iter().take_while(|j| matches!(j, Job::Title(..))).count();
                self.jobs.insert(at, Job::Title(path, rate, samples));
            }
            Command::Job(Job::Text(path)) => {
                let queued = self.jobs.iter().any(|j| matches!(j, Job::Text(p) if *p == path));
                let running = self.job.as_ref().is_some_and(|(p, _)| p.as_ref() == Some(&path));
                if !queued && !running {
                    (self.notify)(Event::Text(path.clone(), Text::Waiting));
                    self.jobs.push_back(Job::Text(path));
                }
            }
            Command::CancelText(path) => {
                let before = self.jobs.len();
                self.jobs.retain(|j| !matches!(j, Job::Text(p) if *p == path));
                if self.jobs.len() != before {
                    (self.notify)(Event::Text(path.clone(), Text::Cancelled));
                }
                if let Some((Some(running), cancel)) = &self.job {
                    if *running == path {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
            Command::JobDone => {
                self.job = None;
                self.last_used = Instant::now();
                if !self.enabled && !self.has_jobs() {
                    self.model = None;
                }
            }
            Command::Rework(reworker) => self.reworker = reworker,
            Command::Recognizer(recognizer) => {
                // Back to the built-in model in the middle of a dictation:
                // it loads while the recording runs.
                if recognizer.is_none() && self.recording.is_some() && self.model.is_none() {
                    self.start_load();
                }
                self.recognizer = recognizer;
                self.told_local = false;
                self.refresh();
            }
            Command::Reworked(id, outcome) => {
                if self.reworking.as_ref().is_some_and(|r| r.id == id) {
                    self.reworked(outcome);
                }
            }
            Command::Quit => {}
        }
    }

    fn has_jobs(&self) -> bool {
        self.job.is_some() || !self.jobs.is_empty()
    }

    /// Starts the next job beside the worker once the model is there; with
    /// no model on the computer the jobs end at once.
    fn next_job(&mut self) {
        if self.job.is_some() || self.jobs.is_empty() || self.download_cancel.is_some() {
            return;
        }
        if !self.spec().is_installed(&self.root) || self.load_failed {
            let failed = if self.load_failed { Text::Failed } else { Text::NoModel };
            for job in self.jobs.drain(..) {
                match job {
                    Job::Title(path, ..) => (self.notify)(Event::Titled(path, None)),
                    Job::Text(path) => (self.notify)(Event::Text(path, failed)),
                }
            }
            return;
        }
        let Some(model) = self.model.clone() else {
            self.start_load();
            return;
        };
        let Some(job) = self.jobs.pop_front() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let path = match &job {
            Job::Text(path) => Some(path.clone()),
            Job::Title(..) => None,
        };
        self.job = Some((path, Arc::clone(&cancel)));
        let (pause, notify, tx) = (Arc::clone(&self.pause), Arc::clone(&self.notify), self.tx.clone());
        let spawned = std::thread::Builder::new()
            .name("kyrr-memo-text".into())
            .spawn(move || {
                match job {
                    Job::Title(path, rate, samples) => {
                        // A dictation that started meanwhile goes first.
                        while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        let title =
                            catch_unwind(AssertUnwindSafe(|| text::title(&model, rate, &samples))).unwrap_or(None);
                        notify(Event::Titled(path, title));
                    }
                    Job::Text(path) => {
                        let started = Instant::now();
                        let mut progress = |done, left| notify(Event::Text(path.clone(), Text::Writing(done, left)));
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            text::write(&model, &path, &cancel, &pause, &mut progress)
                        }))
                        .unwrap_or_else(|_| Err("crashed".into()));
                        let state = match result {
                            Ok(text::Done::Written) => Text::Written,
                            Ok(text::Done::Cancelled) => Text::Cancelled,
                            Err(e) => {
                                eprintln!("kyrr: memo: writing the text of {}: {e}", path.display());
                                Text::Failed
                            }
                        };
                        eprintln!(
                            "kyrr: memo: text {state:?} after {:.1} s",
                            started.elapsed().as_secs_f32()
                        );
                        notify(Event::Text(path, state));
                    }
                }
                drop(model);
                let _ = tx.send(Command::JobDone);
            });
        if spawned.is_err() {
            self.job = None;
        }
    }

    fn key_down(&mut self, at: Instant, tap: bool) {
        // Still waiting for the key (its release got lost): first that.
        self.type_waiting();
        // The key while the language model works: the dictation as it was,
        // without waiting (once the key is up again, in hold mode).
        if self.reworking.is_some() {
            self.reworked(Outcome::Original(Why::Skipped));
            return;
        }
        // While the model loads, the recording starts all the same.
        let ready = matches!(self.state, State::Ready | State::Loading);
        if !ready || self.recording.is_some() || (tap && at.elapsed() > STALE_PRESS) {
            return;
        }
        match Recording::start(self.microphone.as_deref()) {
            Ok(rec) => {
                self.transcript = self.recognizer.is_none().then(|| Transcript::new(rec.rate()));
                self.recording = Some(rec);
                self.words = Live::default();
                self.next_look = Some(Instant::now() + LOOK_FIRST);
                self.next_ahead = Instant::now() + AHEAD_EVERY;
                self.limit = self.limit();
                self.ends = Instant::now() + self.limit;
                self.looks.clear();
                (self.notify)(Event::Message(None));
                self.set_state(State::Recording);
                (self.notify)(Event::Ends(self.ends));
                // Asleep after a long pause: load while the recording runs.
                if self.model.is_none() && self.recognizer.is_none() {
                    self.start_load();
                }
            }
            Err(_) => self.tell("mic"),
        }
    }

    fn key_up(&mut self, at: Instant) {
        let Some(rec) = self.recording.take() else {
            return;
        };
        // A look may have kept the worker busy after the key went up; the
        // recording ran on meanwhile.
        std::thread::sleep(TAIL.saturating_sub(at.elapsed()));
        let lost = rec.lost();
        let (rate, samples) = rec.finish();
        self.set_state(State::Transcribing);
        let started = Instant::now();
        // Ended before the model was there: wait for it.
        if !self.finish_load() {
            return;
        }
        let transcript = self.transcript.take();
        let ahead = transcript.as_ref().map_or(0, Transcript::pieces);
        let mut ran = false;
        let served = self.recognizer.clone().and_then(|r| self.recognize(&r, rate, &samples));
        let text = match &self.model {
            _ if served.is_some() => {
                ran = true;
                served.unwrap_or_default()
            }
            None if self.recognizer.is_some() => String::new(),
            Some(model) => match catch_unwind(AssertUnwindSafe(|| match transcript {
                Some(mut transcript) => {
                    transcript.push(samples.get(transcript.taken()..).unwrap_or_default());
                    model.finish(transcript)
                }
                None => model.transcribe(rate, &samples),
            })) {
                Ok(text) => {
                    ran = true;
                    text
                }
                // Once more from the recording itself, without the pieces
                // heard ahead.
                Err(_) if ahead > 0 => match catch_unwind(AssertUnwindSafe(|| model.transcribe(rate, &samples))) {
                    Ok(text) => {
                        ran = true;
                        text
                    }
                    Err(_) => {
                        self.tell("crash");
                        String::new()
                    }
                },
                Err(_) => {
                    self.tell("crash");
                    String::new()
                }
            },
            None => String::new(),
        };
        self.last_used = Instant::now();
        eprintln!(
            "kyrr: dictation: {}, recognized in {:.1} s after the key, {ahead} pieces while recording",
            describe(rate, &samples),
            started.elapsed().as_secs_f32()
        );
        if let Some(longest) = self.looks.iter().max() {
            let total: Duration = self.looks.iter().sum();
            eprintln!(
                "kyrr: dictation: {} looks at the words, {:.2} s each, longest {:.2} s",
                self.looks.len(),
                total.as_secs_f32() / self.looks.len() as f32,
                longest.as_secs_f32()
            );
        }
        let heard = !text.is_empty();
        if heard && self.reworker.is_some() {
            self.start_rework(text);
        } else if heard && self.held {
            self.waiting = Some((text, Instant::now()));
        } else if heard {
            self.type_text(text);
        }
        if lost {
            self.tell("mic gone");
        } else if !heard && self.muted {
            self.tell("muted");
        } else if !heard && ran && rate > 0 && samples.len() as f32 / rate as f32 >= NOTHING_MIN {
            // A tap by accident is no dictation; a spoken one that came out
            // empty gets a word, or the pill would vanish without a trace.
            (self.notify)(Event::Nothing(say(self.language, "nothing")));
        }
        // The language may have changed while recording: back to whatever
        // the settings say, not blindly to Ready. A rework comes back there.
        if self.reworking.is_none() {
            self.refresh();
        }
    }

    /// The text of a recording from the user's speech server; None when it
    /// could not be reached, and the built-in model, if it is on the
    /// computer, recognised it instead (or nothing could).
    fn recognize(&mut self, recognizer: &Recognizer, rate: u32, samples: &[f32]) -> Option<String> {
        if rate == 0 || (samples.len() as f32) < kyrr_dictate::MIN_SECONDS * rate as f32 {
            return Some(String::new());
        }
        let started = Instant::now();
        let audio = kyrr_dictate::at_model_rate(rate, samples);
        let seconds = audio.len() as f32 / 16_000.0;
        let timeout = Duration::from_secs_f32(3.0 + 0.5 * seconds);
        let language = (!self.more_languages).then(|| self.language.code());
        let wav = kyrr_server::wav(&audio);
        match kyrr_server::transcribe(&recognizer.server, &recognizer.model, &wav, language, timeout) {
            Ok(text) => {
                eprintln!(
                    "kyrr: dictation: the speech server took {:.1} s for {seconds:.1} s",
                    started.elapsed().as_secs_f32()
                );
                return Some(kyrr_dictate::tidy(&text));
            }
            Err(e) => eprintln!("kyrr: dictation: the speech server did not help: {e:?}"),
        }
        // Here then, with the built-in model if it is on the computer.
        if self.model.is_none() && self.spec().is_installed(&self.root) {
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
            if self.loader.is_none() {
                match Dictation::load(&self.root, self.spec(), threads) {
                    Ok(model) => self.model = Some(Arc::new(model)),
                    Err(e) => eprintln!("kyrr: dictation: the speech model would not load: {e}"),
                }
            } else {
                self.finish_load();
            }
        }
        match &self.model {
            Some(model) => {
                if !std::mem::replace(&mut self.told_local, true) {
                    self.tell("asr local");
                }
                let model = Arc::clone(model);
                catch_unwind(AssertUnwindSafe(|| model.transcribe(rate, samples))).ok()
            }
            None => {
                self.tell("asr gone");
                None
            }
        }
    }

    /// Sends the dictation to the user's language model, beside the worker,
    /// which meanwhile listens to the key (one press: the dictation as it
    /// was). The pill says what it waits for.
    fn start_rework(&mut self, text: String) {
        let Some(reworker) = self.reworker.clone() else {
            return;
        };
        self.rework_id += 1;
        let id = self.rework_id;
        let limit = rework::time_limit(reworker.preset, &text);
        let stop = Arc::new(AtomicBool::new(false));
        let language = (!self.more_languages).then(|| self.language.code());
        let busy = match reworker.preset {
            Preset::Fix => "fixing",
            Preset::Polish => "polishing",
            Preset::Translate(_) => "translating",
        };
        (self.notify)(Event::Heard(vec![say(self.language, busy)]));
        let (tx, original, cancel) = (self.tx.clone(), text.clone(), Arc::clone(&stop));
        let deadline = Instant::now() + limit;
        let spawned = std::thread::Builder::new().name("kyrr-rework".into()).spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                rework::rework(
                    &reworker.server,
                    &reworker.model,
                    reworker.preset,
                    &original,
                    language,
                    deadline,
                    &cancel,
                )
            }))
            .unwrap_or(Outcome::Original(Why::Nothing));
            let _ = tx.send(Command::Reworked(id, outcome));
        });
        if spawned.is_err() {
            self.type_text(text);
            return;
        }
        self.reworking = Some(Reworking {
            id,
            original: text,
            until: deadline + REWORK_GRACE,
            stop,
            started: Instant::now(),
        });
    }

    /// Time until a rework is given up; None while none runs.
    fn until_reworked(&self) -> Option<Duration> {
        let r = self.reworking.as_ref()?;
        Some(r.until.saturating_duration_since(Instant::now()))
    }

    fn rework_late(&mut self) {
        self.reworked(Outcome::Original(Why::Server(kyrr_server::Error::Timeout)));
    }

    /// Types what came back from the language model, or the dictation as
    /// it was, with a word in the pill why.
    fn reworked(&mut self, outcome: Outcome) {
        let Some(r) = self.reworking.take() else {
            return;
        };
        r.stop.store(true, Ordering::Relaxed);
        let took = r.started.elapsed().as_secs_f32();
        let (text, note) = match outcome {
            Outcome::Reworked(text) => {
                eprintln!("kyrr: dictation: reworked in {took:.1} s");
                (text, None)
            }
            Outcome::Original(why) => {
                // Numbers only, never the text.
                eprintln!("kyrr: dictation: typed as dictated after {took:.1} s: {why:?}");
                let note = match why {
                    Why::Server(kyrr_server::Error::Refused(_) | kyrr_server::Error::NotLocal) => {
                        Some("rework refused")
                    }
                    Why::Server(_) => Some("rework asleep"),
                    Why::Checked(_) => Some("rework free"),
                    Why::Short | Why::Same | Why::Nothing | Why::Skipped => None,
                };
                (r.original, note)
            }
        };
        if self.held {
            self.waiting = Some((text, Instant::now()));
        } else {
            self.type_text(text);
        }
        if let Some(note) = note {
            (self.notify)(Event::Nothing(say(self.language, note)));
        }
        self.refresh();
    }

    /// Types the text that waited for the key, if any.
    fn type_waiting(&mut self) {
        if let Some((text, _)) = self.waiting.take() {
            self.type_text(text);
        }
    }

    fn type_text(&mut self, text: String) {
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
        let (root, spec) = (self.root.clone(), self.spec());
        let (tx, notify) = (self.tx.clone(), Arc::clone(&self.notify));
        std::thread::spawn(move || {
            let mut last = -1.0f32;
            // Whatever happens in there, the worker hears back, or it
            // would wait for this download forever.
            let result = catch_unwind(AssertUnwindSafe(|| {
                download(
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
                )
            }))
            .unwrap_or_else(|_| Err(DownloadError::Offline("the download crashed".into())));
            let _ = tx.send(Command::Downloaded(id, result));
        });
    }

    /// Stops the download; its thread reports back when it let go of the
    /// files (see `download_running`).
    fn cancel_download(&mut self) {
        // Also a download still waiting for a cancelled one to let go.
        self.download_again = false;
        if let Some(cancel) = self.download_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
            self.download_id += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_dictation_never_gets_the_same_grumble_twice_in_a_row() {
        for german in [true, false] {
            let mut last = String::new();
            for _ in 0..200 {
                let word = nothing(german);
                assert!(!word.is_empty() && word.split(' ').count() <= 3 && word != last);
                last = word;
            }
        }
    }
}
