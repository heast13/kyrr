//! The voice memos in the window: recording, the list, playing, the texts.
//! Everything here runs on the window's thread; the recording and playing
//! themselves have their own threads (see [`crate::memo`]).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime};

use chrono::{Local, NaiveDateTime};
use kyrr_common::wave::{Mode, Overlay};
use kyrr_dictate::Language;
use kyrr_server::rework::{self, Outcome, Preset, Why, TARGETS};
use kyrr_server::{Error, Server};
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};

use crate::dictation::{Command, Job, Text};
use crate::memo::compare;
use crate::memo::library::{self, Memo, Stamps};
use crate::memo::player::Player;
use crate::memo::recorder::{Ending, Recording};
use crate::memo::title;
use crate::{AppState, ComparePair, MainWindow, MemoItem};

/// Shorter memos are a key pressed twice by accident: not kept.
const SHORTEST: f64 = 3.0;
/// How often the clock and the playing position move on.
const TICK: Duration = Duration::from_millis(200);
/// How long "Copied" stays on the button.
const COPIED: Duration = Duration::from_secs(2);
/// How long a short note under the card ("Too short") stays.
const BRIEF: Duration = Duration::from_secs(5);
/// How long after the last key typed in a text it is saved.
const SAVE_AFTER: Duration = Duration::from_millis(700);
/// How often the open text looks whether its file changed outside kyrr.
const WATCH: Duration = Duration::from_secs(1);

/// Told when the list was looked at (seconds since 1970).
type SeenHook = Box<dyn Fn(u64)>;

/// The user's language model, for rewording or translating a memo's text.
pub struct MemoRework {
    pub server: Server,
    pub model: String,
    /// The dictation language's code; None with more languages.
    pub language: Option<&'static str>,
}

/// How reworking a memo's text went: the paragraphs and how many of them
/// changed and how many of those were only fixed, or what kept the server
/// from helping.
type Reworked = Result<(Vec<library::Paragraph>, usize, usize), Error>;

pub struct Memos {
    window: Weak<MainWindow>,
    worker: mpsc::Sender<Command>,
    wave: Overlay,
    dir: PathBuf,
    language: Cell<Language>,
    /// A memo is recording: dictation waits (read by the key thread), and
    /// whether dictation is recording (then a memo waits).
    active: Arc<AtomicBool>,
    dictating: Arc<AtomicBool>,
    microphone: RefCell<Option<String>>,
    /// Voice memos are switched on; off, a key press or the tray starts none.
    enabled: Cell<bool>,
    auto_text: Cell<bool>,
    /// The folder as last read, and the part of it the list shows.
    all: RefCell<Vec<Memo>>,
    shown: RefCell<Vec<PathBuf>>,
    /// The rows of the list in the window, one for each shown memo.
    rows: Rc<VecModel<MemoItem>>,
    recording: RefCell<Option<Recording>>,
    player: RefCell<Option<Player>>,
    /// Memos whose title is still being found (they keep their first name
    /// until then), and the texts on their way.
    titling: RefCell<Vec<PathBuf>>,
    /// Memos still named as the start of the recording gave: once their
    /// text is written, a title from all of it replaces that, unless the
    /// user named them meanwhile.
    auto_named: RefCell<HashSet<PathBuf>>,
    texts: RefCell<HashMap<PathBuf, Text>>,
    /// The memo whose text is open big.
    sheet: RefCell<Option<PathBuf>>,
    /// The text field holds changes that are not on disk yet.
    dirty: Cell<bool>,
    /// When the open text's file was last written, as the field shows it: a
    /// different one means it was changed outside kyrr.
    sheet_stamp: Cell<Option<SystemTime>>,
    /// What the text field shows before each paragraph.
    stamps: Cell<Stamps>,
    /// When the list was last looked at (seconds since 1970); memos made
    /// after that are new.
    seen: Cell<u64>,
    on_seen: RefCell<Option<SeenHook>>,
    timer: slint::Timer,
    copied: slint::Timer,
    brief: slint::Timer,
    saving: slint::Timer,
    watching: slint::Timer,
    /// The language model, when one is set up and switched on.
    rework: RefCell<Option<MemoRework>>,
    /// The memo whose text the language model works on, and what stops it.
    reworking: RefCell<Option<(PathBuf, Arc<AtomicBool>)>>,
}

thread_local! {
    static MEMOS: RefCell<Option<Rc<Memos>>> = const { RefCell::new(None) };
}

/// The memos of this window, once [`Memos::install`] ran.
pub fn memos() -> Option<Rc<Memos>> {
    MEMOS.with_borrow(Clone::clone)
}

impl Memos {
    #[allow(clippy::too_many_arguments)]
    pub fn install(
        window: &MainWindow,
        worker: mpsc::Sender<Command>,
        wave: Overlay,
        language: Language,
        active: Arc<AtomicBool>,
        dictating: Arc<AtomicBool>,
        auto_text: bool,
    ) -> Rc<Memos> {
        let dir = library::folder();
        library::make_private(&dir);
        let memos = Rc::new(Memos {
            window: window.as_weak(),
            worker,
            wave,
            dir,
            language: Cell::new(language),
            active,
            dictating,
            microphone: RefCell::new(None),
            enabled: Cell::new(true),
            auto_text: Cell::new(auto_text),
            all: RefCell::new(Vec::new()),
            shown: RefCell::new(Vec::new()),
            rows: Rc::new(VecModel::default()),
            recording: RefCell::new(None),
            player: RefCell::new(None),
            titling: RefCell::new(Vec::new()),
            auto_named: RefCell::new(HashSet::new()),
            texts: RefCell::new(HashMap::new()),
            sheet: RefCell::new(None),
            dirty: Cell::new(false),
            sheet_stamp: Cell::new(None),
            stamps: Cell::new(Stamps::None),
            seen: Cell::new(0),
            on_seen: RefCell::new(None),
            timer: slint::Timer::default(),
            copied: slint::Timer::default(),
            brief: slint::Timer::default(),
            saving: slint::Timer::default(),
            watching: slint::Timer::default(),
            rework: RefCell::new(None),
            reworking: RefCell::new(None),
        });
        MEMOS.set(Some(Rc::clone(&memos)));
        window
            .global::<AppState>()
            .set_memo_items(ModelRc::from(Rc::clone(&memos.rows) as Rc<dyn Model<Data = MemoItem>>));
        window.set_memo_folder(memos.folder_name().into());
        memos.wire(window);
        memos.refresh();
        memos
    }

    fn german(&self) -> bool {
        self.language.get() == Language::German
    }

    fn say(&self, german: &str, english: &str) -> String {
        if self.german() { german } else { english }.to_string()
    }

    pub fn set_language(&self, language: Language) {
        self.language.set(language);
        if let Some(w) = self.window.upgrade() {
            w.set_memo_folder(self.folder_name().into());
        }
        self.show_list();
    }

    pub fn set_microphone(&self, guid: Option<String>) {
        *self.microphone.borrow_mut() = guid;
    }

    pub fn set_enabled(self: &Rc<Self>, on: bool) {
        self.enabled.set(on);
        if !on {
            self.stop();
        }
    }

    pub fn set_auto_text(&self, on: bool) {
        self.auto_text.set(on);
    }

    /// What the text field shows before each paragraph (0, 1 or 2, as
    /// saved in the settings).
    pub fn set_stamps(&self, index: i32) {
        self.stamps.set(Stamps::from_index(index));
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_sheet_stamps(index.clamp(0, 2));
        }
    }

    /// The choice in the text view: saves what was typed, then shows the
    /// text with the other stamps.
    pub fn stamps_changed(&self, index: i32) {
        self.flush();
        self.set_stamps(index);
        self.show_sheet();
    }

    /// When the list was last looked at; `on_seen` hears each new moment so
    /// it can be kept.
    pub fn set_seen(&self, seconds: u64, on_seen: impl Fn(u64) + 'static) {
        self.seen.set(seconds);
        *self.on_seen.borrow_mut() = Some(Box::new(on_seen));
        self.show_list();
    }

    /// The user looks at the list: nothing is new any more.
    pub fn mark_seen(&self) {
        let now = Local::now().timestamp().max(0) as u64;
        self.seen.set(now);
        if let Some(hook) = self.on_seen.borrow().as_ref() {
            hook(now);
        }
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_new(false);
        }
    }

    /// Which memo the commands of the widget in Omarchy's bar mean: they
    /// name it by its file, which stays put while the list changes.
    pub fn index_of(&self, name: &str) -> Option<i32> {
        self.shown
            .borrow()
            .iter()
            .position(|p| p.file_name().is_some_and(|n| n.to_string_lossy() == name))
            .map(|i| i as i32)
    }

    /// The memo whose text the widget's menu shows, by its file's name:
    /// the same text view as in the window, which the menu draws instead.
    pub fn show_in_widget(self: &Rc<Self>, name: Option<&str>) {
        match name.and_then(|n| self.index_of(n)) {
            Some(i) => self.show_text(i),
            None => {
                self.close_text();
                *self.sheet.borrow_mut() = None;
                self.show_list();
            }
        }
    }

    /// The file name of the memo whose text is open.
    pub fn sheet_name(&self) -> Option<String> {
        self.sheet
            .borrow()
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// The same, "" for none.
    #[cfg(target_os = "linux")]
    pub fn sheet_id(&self) -> String {
        self.sheet_name().unwrap_or_default()
    }

    /// Whether the memo with this file name plays now.
    #[cfg(target_os = "linux")]
    fn playing(&self, path: &Path) -> bool {
        self.player.borrow().as_ref().is_some_and(|p| p.path() == path)
    }

    /// The list for the widget in Omarchy's bar, newest first, as the search
    /// finds it.
    #[cfg(target_os = "linux")]
    pub fn widget_list(&self) -> serde_json::Value {
        let all = self.all.borrow();
        let texts = self.texts.borrow();
        let titling = self.titling.borrow();
        let shown = self.shown.borrow();
        let list: Vec<serde_json::Value> = all
            .iter()
            .filter(|m| shown.contains(&m.path))
            .map(|m| {
                let (text, progress, left) = match texts.get(&m.path) {
                    Some(Text::Waiting) => ("writing", 0.0, String::new()),
                    Some(Text::Writing(done, left)) => {
                        ("writing", *done, left.map(|l| self.left(l)).unwrap_or_default())
                    }
                    Some(Text::Failed | Text::NoModel) => ("failed", 0.0, String::new()),
                    _ if m.text.is_some() => ("ready", 1.0, String::new()),
                    _ => ("none", 0.0, String::new()),
                };
                serde_json::json!({
                    "id": m.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    "title": m.title,
                    "when": self.date(m.when),
                    "length": library::clock(m.seconds.round() as u32),
                    "pending": titling.contains(&m.path),
                    "text": text,
                    "progress": progress,
                    "left": left,
                    "isNew": m.saved > self.seen.get(),
                    "waiting": matches!(texts.get(&m.path), Some(Text::Waiting)),
                    "playing": self.playing(&m.path),
                })
            })
            .collect();
        list.into()
    }

    /// The folder the memos' folder is in, as people know it: "Dokumente".
    #[cfg(target_os = "linux")]
    pub fn widget_folder(&self) -> String {
        self.folder_name()
    }

    /// Whether texts can be written: the speech model is on the computer.
    pub fn set_can_write(&self, can: bool) {
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_can_write(can);
        }
    }

    pub fn recording(&self) -> bool {
        self.recording.borrow().is_some()
    }

    /// The documents folder as people know it: "Dokumente". Windows names
    /// it in the user's language only on the screen.
    fn folder_name(&self) -> String {
        if cfg!(windows) {
            return self.say("Dokumente", "Documents");
        }
        self.dir
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.say("Dokumente", "Documents"))
    }

    fn wire(self: &Rc<Self>, window: &MainWindow) {
        let state = window.global::<AppState>();
        let weak = Rc::downgrade(self);
        let with = move |f: fn(&Rc<Memos>, i32)| {
            let weak = weak.clone();
            move |i: i32| {
                if let Some(m) = weak.upgrade() {
                    f(&m, i);
                }
            }
        };
        state.on_memo_play(with(|m, i| m.play(i, None)));
        state.on_memo_show(with(|m, i| m.show_in_folder(i)));
        state.on_memo_delete(with(|m, i| m.delete(i)));
        state.on_memo_write_text(with(|m, i| m.write_text(i)));
        state.on_memo_cancel_text(with(|m, i| m.cancel_text(i)));
        state.on_memo_show_text(with(|m, i| m.show_text(i)));
        let weak = Rc::downgrade(self);
        let with0 = move |f: fn(&Rc<Memos>)| {
            let weak = weak.clone();
            move || {
                if let Some(m) = weak.upgrade() {
                    f(&m);
                }
            }
        };
        state.on_memo_record(with0(|m| m.toggle()));
        state.on_memo_close_text(with0(|m| m.close_text()));
        state.on_memo_restore_text(with0(|m| m.restore_text()));
        state.on_memo_show_compare(with0(|m| m.show_compare()));
        state.on_memo_stop_rework(with0(|m| m.stop_rework()));
        let weak = Rc::downgrade(self);
        state.on_memo_rework(move |action, target| {
            if let Some(m) = weak.upgrade() {
                m.rework_text(action, target);
            }
        });
        state.on_memo_open_text(with0(|m| m.open_text()));
        state.on_memo_open_folder(with0(|m| m.open_folder()));
        let weak = Rc::downgrade(self);
        state.on_memo_copy_text(move |a, b| {
            if let Some(m) = weak.upgrade() {
                m.copy_text(a, b);
            }
        });
        let weak = Rc::downgrade(self);
        state.on_memo_play_here(move |cursor| {
            if let Some(m) = weak.upgrade() {
                m.play_here(cursor);
            }
        });
        let weak = Rc::downgrade(self);
        state.on_memo_sheet_edited(move |_| {
            if let Some(m) = weak.upgrade() {
                m.sheet_edited();
            }
        });
        let weak = Rc::downgrade(self);
        state.on_memo_here_label(move |text, cursor| {
            weak.upgrade()
                .map(|m| m.here_label(&text, cursor))
                .unwrap_or_default()
                .into()
        });
        let weak = Rc::downgrade(self);
        state.on_memo_rename(move |i, title| {
            if let Some(m) = weak.upgrade() {
                m.rename(i, &title);
            }
        });
        let weak = Rc::downgrade(self);
        state.on_memo_search(move |_| {
            if let Some(m) = weak.upgrade() {
                m.show_list();
            }
        });
    }

    fn path(&self, index: i32) -> Option<PathBuf> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.shown.borrow().get(i).cloned())
    }

    fn tell(&self, message: &str) {
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_message(message.into());
        }
    }

    /// A note that goes again after a few seconds, unless another took its
    /// place.
    fn tell_briefly(&self, message: &str) {
        self.tell(message);
        let weak = self.window.clone();
        let message = slint::SharedString::from(message);
        self.brief.start(slint::TimerMode::SingleShot, BRIEF, move || {
            if let Some(w) = weak.upgrade() {
                let state = w.global::<AppState>();
                if state.get_memo_message() == message {
                    state.set_memo_message("".into());
                }
            }
        });
    }

    /// Reads the folder again and shows it.
    pub fn refresh(&self) {
        let recording = self.recording.borrow().as_ref().map(|r| r.path().to_path_buf());
        let mut all = library::list(&self.dir);
        all.retain(|m| Some(&m.path) != recording.as_ref());
        *self.all.borrow_mut() = all;
        // Looking at the list as it changes: what comes in is seen.
        if self
            .window
            .upgrade()
            .is_some_and(|w| w.global::<AppState>().get_page() == 6)
        {
            self.mark_seen();
        }
        self.show_list();
    }

    /// Shows the memos that fit the search.
    fn show_list(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let query = state.get_memo_query().to_lowercase();
        let query = query.trim();
        let all = self.all.borrow();
        let sheet = self.sheet.borrow().clone();
        // The one whose text is open stays, also when a correction or a new
        // title no longer matches the search.
        let fits = |m: &Memo| {
            query.is_empty()
                || sheet.as_ref() == Some(&m.path)
                || m.title.to_lowercase().contains(query)
                || m.text
                    .as_ref()
                    .is_some_and(|t| t.iter().any(|p| p.text.to_lowercase().contains(query)))
        };
        let shown: Vec<&Memo> = all.iter().filter(|m| fits(m)).collect();
        let titling = self.titling.borrow();
        let texts = self.texts.borrow();
        let items: Vec<MemoItem> = shown
            .iter()
            .map(|m| {
                let (text_state, progress, left) = match texts.get(&m.path) {
                    Some(Text::Waiting) => (1, 0.0, String::new()),
                    Some(Text::Writing(done, left)) => (2, *done, left.map(|l| self.left(l)).unwrap_or_default()),
                    Some(Text::Failed | Text::NoModel) => (4, 0.0, String::new()),
                    _ if m.text.is_some() => (3, 1.0, String::new()),
                    _ => (0, 0.0, String::new()),
                };
                MemoItem {
                    title: m.title.as_str().into(),
                    when: self.when(m).into(),
                    pending: titling.contains(&m.path),
                    text_state,
                    text_progress: progress,
                    text_left: left.into(),
                }
            })
            .collect();
        let playing = self.player.borrow().as_ref().map(|p| p.path().to_path_buf());
        let index_of = |path: &Option<PathBuf>| {
            path.as_ref()
                .and_then(|p| shown.iter().position(|m| &m.path == p))
                .map_or(-1, |i| i as i32)
        };
        state.set_memo_playing(index_of(&playing));
        state.set_memo_sheet_index(index_of(&self.sheet.borrow()));
        // A name being typed and a delete being asked stay with their memo
        // when rows come and go (a new memo on top, a search).
        let editing = self.path(state.get_memo_editing());
        let confirm = self.path(state.get_memo_confirm());
        let paths: Vec<PathBuf> = shown.iter().map(|m| m.path.clone()).collect();
        let same_rows = *self.shown.borrow() == paths;
        *self.shown.borrow_mut() = paths;
        state.set_memo_editing(index_of(&editing));
        state.set_memo_confirm(index_of(&confirm));
        state.set_memo_count(all.len() as i32);
        state.set_memo_new(state.get_page() != 6 && all.iter().any(|m| m.saved > self.seen.get()));
        // The same rows: changed in place, so a row being edited keeps what
        // was typed into it.
        if same_rows && self.rows.row_count() == items.len() {
            for (i, item) in items.into_iter().enumerate() {
                if self.rows.row_data(i).as_ref() != Some(&item) {
                    self.rows.set_row_data(i, item);
                }
            }
        } else {
            self.rows.set_vec(items);
        }
    }

    /// "Heute, 18:05 · 3:12": when and how long.
    fn when(&self, memo: &Memo) -> String {
        format!(
            "{} · {}",
            self.date(memo.when),
            library::clock(memo.seconds.round() as u32)
        )
    }

    fn date(&self, when: NaiveDateTime) -> String {
        let today = Local::now().date_naive();
        let day = when.date();
        let time = when.format("%H:%M");
        if day == today {
            format!("{} {time}", self.say("Heute,", "Today,"))
        } else if today.pred_opt() == Some(day) {
            format!("{} {time}", self.say("Gestern,", "Yesterday,"))
        } else if self.german() {
            format!("{}, {time}", when.format("%d.%m.%Y"))
        } else {
            format!("{}, {time}", when.format("%-d %b %Y"))
        }
    }

    /// "noch 3 Min.": about how long a text still takes.
    fn left(&self, left: Duration) -> String {
        let minutes = (left.as_secs_f64() / 60.0).ceil() as u64;
        match minutes {
            0 | 1 if left.as_secs() < 40 => self.say("gleich fertig", "almost done"),
            n => {
                if self.german() {
                    format!("noch {n} Min.")
                } else {
                    format!("{n} min left")
                }
            }
        }
    }

    /// The memo key, the button and the tray: starts a memo, or ends it.
    pub fn toggle(self: &Rc<Self>) {
        if self.recording() {
            self.stop();
        } else {
            self.start();
        }
    }

    /// The memo key held down came up: ends the memo it started.
    pub fn end(self: &Rc<Self>) {
        if self.recording() {
            self.stop();
        }
    }

    fn start(self: &Rc<Self>) {
        if !self.enabled.get() || self.dictating.load(Ordering::Relaxed) {
            return;
        }
        let placeholder = self.say("Aufnahme", "Recording");
        let path = library::new_path(&self.dir, &placeholder);
        let microphone = self.microphone.borrow().clone();
        match Recording::start(microphone.as_deref(), &path) {
            Ok(recording) => {
                *self.recording.borrow_mut() = Some(recording);
                self.active.store(true, Ordering::Relaxed);
                self.tell("");
                self.wave.set(Mode::Recording);
                self.wave.set_clock(0.0);
                let weak = Rc::downgrade(self);
                self.timer.start(slint::TimerMode::Repeated, TICK, move || {
                    if let Some(m) = weak.upgrade() {
                        m.tick();
                    }
                });
                self.show_state();
            }
            Err(e) => {
                eprintln!("kyrr: memo: recording into {}: {e}", path.display());
                let message = if e.starts_with("microphone") {
                    self.say(
                        "Konnte das Mikrofon nicht öffnen. Steckt es noch?",
                        "Could not open the microphone. Is it still plugged in?",
                    )
                } else {
                    self.say(
                        "Konnte die Aufnahme nicht anlegen. Ist die Festplatte voll, oder darf kyrr nicht in den Ordner Dokumente schreiben?",
                        "Could not create the recording. Is the disk full, or may kyrr not write to the Documents folder?",
                    )
                };
                self.tell(&message);
            }
        }
    }

    fn stop(self: &Rc<Self>) {
        let Some(recording) = self.recording.borrow_mut().take() else {
            return;
        };
        let finished = recording.finish();
        self.active.store(false, Ordering::Relaxed);
        self.wave.set(Mode::Hidden);
        let path = finished.path.clone();
        // A full disk may leave nothing to measure: that message stays.
        let short = finished.seconds < SHORTEST && finished.ending != Ending::Disk;
        if short {
            let _ = std::fs::remove_file(&path);
            self.wave
                .set_note(self.say("Zu kurz, nicht gespeichert", "Too short, not saved"));
        } else if path.exists() {
            self.titling.borrow_mut().push(path.clone());
            let _ = self
                .worker
                .send(Command::Job(Job::Title(path, finished.rate, finished.start)));
        }
        let message = match finished.ending {
            Ending::MicGone if short => self.say(
                "Das Mikrofon war weg, und unter 3 Sekunden speichert kyrr nichts.",
                "The microphone went away, and kyrr keeps nothing under 3 seconds.",
            ),
            _ if short => self.say("Zu kurz, nicht gespeichert.", "Too short, not saved."),
            Ending::Stopped => String::new(),
            Ending::Limit => self.say(
                "Nach 4 Stunden war Schluss: die Aufnahme ist gespeichert.",
                "Stopped after 4 hours: the recording is saved.",
            ),
            Ending::MicGone => self.say(
                "Das Mikrofon war weg. Was bis dahin kam, ist gespeichert.",
                "The microphone went away. What came until then is saved.",
            ),
            Ending::Disk => self.say(
                "Die Festplatte ist voll. Was bis dahin kam, ist gespeichert.",
                "The disk is full. What came until then is saved.",
            ),
        };
        // A key pressed by accident needs no lasting note.
        if short && finished.ending != Ending::MicGone {
            self.tell_briefly(&message);
        } else {
            self.tell(&message);
        }
        self.keep_ticking();
        self.show_state();
        self.refresh();
    }

    /// Recording or not, in the window and the tray.
    fn show_state(&self) {
        let recording = self.recording();
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_memo_recording(recording);
            state.set_memo_elapsed(self.elapsed().into());
        }
        if let Some(t) = crate::tray() {
            t.set_memo_recording(recording);
        }
    }

    fn elapsed(&self) -> String {
        self.recording
            .borrow()
            .as_ref()
            .map_or_else(String::new, |r| library::clock(r.elapsed().as_secs() as u32))
    }

    /// The timer runs while something records or plays.
    fn keep_ticking(&self) {
        if !self.recording() && self.player.borrow().is_none() {
            self.timer.stop();
        }
    }

    fn tick(self: &Rc<Self>) {
        if self.recording.borrow().as_ref().is_some_and(Recording::ended) {
            self.stop();
        }
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        if self.recording() {
            state.set_memo_elapsed(self.elapsed().into());
        }
        if let Some(r) = self.recording.borrow().as_ref() {
            self.wave.set_clock(r.elapsed().as_secs_f32());
        }
        let finished = self.player.borrow().as_ref().is_some_and(Player::finished);
        if finished {
            self.player.borrow_mut().take();
            state.set_memo_playing(-1);
            self.keep_ticking();
        }
        if let Some(player) = self.player.borrow().as_ref() {
            let total = self
                .all
                .borrow()
                .iter()
                .find(|m| m.path == player.path())
                .map_or(0.0, |m| m.seconds);
            let at = player.seconds().min(total);
            state.set_memo_play_fraction(if total > 0.0 { (at / total) as f32 } else { 0.0 });
            state.set_memo_play_time(
                format!(
                    "{} / {}",
                    library::clock(at as u32),
                    library::clock(total.round() as u32)
                )
                .into(),
            );
        }
    }

    fn play(self: &Rc<Self>, index: i32, from: Option<f64>) {
        if let Some(path) = self.path(index) {
            self.play_path(&path, from);
        }
    }

    fn play_path(self: &Rc<Self>, path: &Path, from: Option<f64>) {
        let same = self.player.borrow().as_ref().is_some_and(|p| p.path() == path);
        self.player.borrow_mut().take();
        if !same || from.is_some() {
            *self.player.borrow_mut() = Some(Player::start(path, from.unwrap_or(0.0)));
            let weak = Rc::downgrade(self);
            if !self.timer.running() {
                self.timer.start(slint::TimerMode::Repeated, TICK, move || {
                    if let Some(m) = weak.upgrade() {
                        m.tick();
                    }
                });
            }
        }
        self.keep_ticking();
        self.show_list();
        self.tick();
    }

    fn show_in_folder(&self, index: i32) {
        if let Some(path) = self.path(index) {
            std::thread::spawn(move || library::show(&path));
        }
    }

    fn open_folder(&self) {
        let _ = library::create_folder(&self.dir);
        let dir = self.dir.clone();
        std::thread::spawn(move || library::open(&dir));
    }

    fn delete(&self, index: i32) {
        let Some(path) = self.path(index) else {
            return;
        };
        if self.player.borrow().as_ref().is_some_and(|p| p.path() == path) {
            self.player.borrow_mut().take();
        }
        let _ = self.worker.send(Command::CancelText(path.clone()));
        if self.reworking.borrow().as_ref().is_some_and(|(p, _)| *p == path) {
            self.stop_rework();
        }
        self.texts.borrow_mut().remove(&path);
        self.auto_named.borrow_mut().remove(&path);
        // Nothing typed into its text is saved after it is gone.
        if self.sheet.borrow().as_ref() == Some(&path) {
            self.saving.stop();
            self.dirty.set(false);
        }
        if let Err(e) = library::delete(&path) {
            eprintln!("kyrr: memo: deleting {}: {e}", path.display());
            self.tell(&self.say("Konnte die Aufnahme nicht löschen.", "Could not delete the recording."));
        }
        if self.sheet.borrow().as_ref() == Some(&path) {
            self.close_text();
        }
        self.refresh();
    }

    fn rename(&self, index: i32, title: &str) {
        let Some(path) = self.path(index) else {
            return;
        };
        if self.player.borrow().as_ref().is_some_and(|p| p.path() == path) {
            self.player.borrow_mut().take();
        }
        self.auto_named.borrow_mut().remove(&path);
        match library::rename(&path, title) {
            Ok(to) => {
                self.moved(&path, &to);
                if self.sheet.borrow().as_ref() == Some(&path) {
                    *self.sheet.borrow_mut() = Some(to);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(e) => {
                eprintln!("kyrr: memo: renaming {}: {e}", path.display());
                self.tell(&self.say(
                    "Konnte die Aufnahme nicht umbenennen.",
                    "Could not rename the recording.",
                ));
            }
        }
        self.refresh();
        self.show_sheet();
    }

    fn write_text(&self, index: i32) {
        if let Some(path) = self.path(index) {
            let _ = self.worker.send(Command::Job(Job::Text(path)));
        }
    }

    /// A memo renamed while its text waits or is being written: the text
    /// starts again under the new name.
    fn moved(&self, from: &Path, to: &Path) {
        if self.texts.borrow_mut().remove(from).is_some() {
            let _ = self.worker.send(Command::CancelText(from.to_path_buf()));
            let _ = self.worker.send(Command::Job(Job::Text(to.to_path_buf())));
        }
    }

    fn cancel_text(&self, index: i32) {
        if let Some(path) = self.path(index) {
            let _ = self.worker.send(Command::CancelText(path));
        }
    }

    /// The title the dictation model heard at the start of a memo.
    pub fn titled(&self, path: PathBuf, title: Option<String>) {
        self.titling.borrow_mut().retain(|p| *p != path);
        // Never the memo being recorded: the first one's name can come free
        // again (deleted in the file manager) and be the new one's.
        let recording = self.recording.borrow().as_ref().is_some_and(|r| r.path() == path);
        let path = match title {
            Some(title) if path.exists() && !recording => match library::rename(&path, &title) {
                Ok(to) => {
                    self.moved(&path, &to);
                    if self.sheet.borrow().as_ref() == Some(&path) {
                        *self.sheet.borrow_mut() = Some(to.clone());
                    }
                    to
                }
                Err(_) => path,
            },
            _ => path,
        };
        if path.exists() && !recording {
            self.auto_named.borrow_mut().insert(path.clone());
        }
        if self.auto_text.get() && path.exists() && !recording {
            let _ = self.worker.send(Command::Job(Job::Text(path)));
        }
        self.refresh();
    }

    /// A memo whose name came from its start gets one from all it says,
    /// once. Returns where the memo is now.
    fn retitle(&self, path: PathBuf) -> PathBuf {
        if !self.auto_named.borrow_mut().remove(&path) {
            return path;
        }
        let Some(title) =
            library::read_text(&path).and_then(|paragraphs| title::from_text(&library::plain(&paragraphs)))
        else {
            return path;
        };
        if self.player.borrow().as_ref().is_some_and(|p| p.path() == path) {
            self.player.borrow_mut().take();
        }
        let Ok(to) = library::rename(&path, &title) else {
            return path;
        };
        if self.sheet.borrow().as_ref() == Some(&path) {
            *self.sheet.borrow_mut() = Some(to.clone());
        }
        to
    }

    /// How the text of a memo comes along.
    pub fn text(&self, path: PathBuf, text: Text) {
        match text {
            Text::Written | Text::Cancelled => {
                self.texts.borrow_mut().remove(&path);
                let path = if text == Text::Written {
                    self.retitle(path)
                } else {
                    path
                };
                self.refresh();
                if text == Text::Written && self.sheet.borrow().as_ref() == Some(&path) {
                    self.show_sheet();
                }
            }
            other => {
                self.texts.borrow_mut().insert(path, other);
                self.show_list();
            }
        }
    }

    fn show_text(self: &Rc<Self>, index: i32) {
        let Some(path) = self.path(index) else {
            return;
        };
        // What could not be saved for the memo before is given up here.
        self.flush();
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_memo_rework_note("".into());
            // Another memo opens as text.
            state.set_memo_sheet_view(0);
        }
        self.dirty.set(false);
        *self.sheet.borrow_mut() = Some(path);
        self.show_sheet();
        let weak = Rc::downgrade(self);
        self.watching.start(slint::TimerMode::Repeated, WATCH, move || {
            if let Some(m) = weak.upgrade() {
                m.watch_sheet();
            }
        });
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.global::<AppState>().set_memo_sheet_open(true);
        // In Omarchy's bar the menu shows it; the window stays as it is.
        if !crate::headless() {
            crate::fit_sheet(&w, true);
        }
    }

    /// Fills the big text view with the memo it is open for.
    fn show_sheet(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let sheet = self.sheet.borrow();
        let all = self.all.borrow();
        let Some(memo) = sheet.as_ref().and_then(|p| all.iter().find(|m| &m.path == p)) else {
            return;
        };
        // Typed text that is not saved yet stays as it is.
        if !self.dirty.get() {
            let field = library::field(
                memo.text.as_deref().unwrap_or_default(),
                self.stamps.get(),
                library::started(&memo.path),
            );
            if state.get_memo_sheet_text() != field.as_str() {
                state.set_memo_sheet_text(field.into());
            }
            state.set_memo_sheet_saved(true);
            self.sheet_stamp.set(text_stamp(&memo.path));
        }
        state.set_memo_sheet_title(memo.title.as_str().into());
        state.set_memo_sheet_when(self.when(memo).into());
        let original = library::read_original(&memo.path);
        state.set_memo_sheet_original(original.is_some());
        match original {
            Some(before) => state.set_memo_sheet_compare(self.comparison(&before, memo)),
            None => state.set_memo_sheet_view(0),
        }
        state.set_memo_copied(false);
        drop(all);
        drop(sheet);
        self.show_list();
    }

    fn close_text(&self) {
        self.stop_rework();
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_rework_note("".into());
        }
        self.flush();
        self.watching.stop();
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_sheet_open(false);
            if !crate::headless() {
                crate::fit_sheet(&w, false);
            }
        }
    }

    /// The open text changed outside kyrr (an editor, a sync): the field
    /// shows the file again, unless something typed waits to be saved. A
    /// memo gone from the folder closes its text.
    fn watch_sheet(&self) {
        let Some(path) = self.sheet.borrow().clone() else {
            self.watching.stop();
            return;
        };
        if !path.exists() {
            self.saving.stop();
            self.dirty.set(false);
            *self.sheet.borrow_mut() = None;
            self.close_text();
            self.refresh();
            return;
        }
        let stamp = text_stamp(&path);
        if self.dirty.get() || stamp == self.sheet_stamp.get() {
            return;
        }
        if let Some(memo) = self.all.borrow_mut().iter_mut().find(|m| m.path == path) {
            memo.text = library::read_text(&path);
        }
        self.show_sheet();
    }

    /// The text field changed: saved a moment after the last key.
    fn sheet_edited(self: &Rc<Self>) {
        self.dirty.set(true);
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_sheet_saved(false);
        }
        let weak = Rc::downgrade(self);
        self.saving.start(slint::TimerMode::SingleShot, SAVE_AFTER, move || {
            if let Some(m) = weak.upgrade() {
                m.flush();
            }
        });
    }

    /// Writes what was typed into the text field to the memo's text file.
    pub fn flush(&self) {
        self.saving.stop();
        if !self.dirty.get() {
            return;
        }
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let Some(path) = self.sheet.borrow().clone() else {
            self.dirty.set(false);
            return;
        };
        // The memo went (deleted or renamed outside kyrr): no text without it.
        if !path.exists() {
            self.dirty.set(false);
            return;
        }
        let old = self.paragraphs_of(&path);
        let paragraphs = library::edited(&old, state.get_memo_sheet_text().as_str());
        match library::write_text(&path, &paragraphs) {
            Ok(_) => {
                self.dirty.set(false);
                self.sheet_stamp.set(text_stamp(&path));
                if let Some(memo) = self.all.borrow_mut().iter_mut().find(|m| m.path == path) {
                    memo.text = Some(paragraphs);
                }
                state.set_memo_sheet_saved(true);
                // The search finds what was typed.
                self.show_list();
            }
            Err(e) => {
                eprintln!("kyrr: memo: saving the text of {}: {e}", path.display());
                self.tell_briefly(&self.say("Konnte den Text nicht speichern.", "Could not save the text."));
            }
        }
    }

    /// The paragraphs of the memo at `path` as last read or saved.
    fn paragraphs_of(&self, path: &Path) -> Vec<library::Paragraph> {
        self.all
            .borrow()
            .iter()
            .find(|m| m.path == path)
            .and_then(|m| m.text.clone())
            .unwrap_or_default()
    }

    /// The text before and after the language model, marked, paragraph
    /// beside paragraph.
    fn comparison(&self, before: &[library::Paragraph], memo: &Memo) -> ModelRc<ComparePair> {
        let after = memo.text.as_deref().unwrap_or_default();
        let styled = |markdown: &str| {
            slint::StyledText::from_markdown(markdown).unwrap_or_else(|e| {
                eprintln!("kyrr: memo: comparing the text of {}: {e}", memo.path.display());
                slint::StyledText::from_plain_text(markdown)
            })
        };
        let pairs: Vec<ComparePair> = compare::markdown(before, after)
            .iter()
            .map(|(then, now)| ComparePair {
                before: styled(then),
                after: styled(now),
            })
            .collect();
        ModelRc::new(VecModel::from(pairs))
    }

    /// The comparison opens: what was typed counts in it.
    fn show_compare(&self) {
        self.flush();
        self.show_sheet();
    }

    /// The language model as set up now (None for none or switched off).
    pub fn set_rework(&self, rework: Option<MemoRework>) {
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_rework_ready(rework.is_some());
        }
        *self.rework.borrow_mut() = rework;
    }

    /// Has the language model fix (1), reword (2) or formalize (3) the open
    /// text and then translate it (into `TARGETS[target - 1]`), or only one
    /// of them, paragraph by paragraph beside the window. The text before
    /// it is kept.
    fn rework_text(self: &Rc<Self>, action: i32, target: i32) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let (Some(path), Some(rework)) = (
            self.sheet.borrow().clone(),
            self.rework
                .borrow()
                .as_ref()
                .map(|r| (r.server.clone(), r.model.clone(), r.language)),
        ) else {
            return;
        };
        if self.reworking.borrow().is_some() {
            return;
        }
        let mut steps = Vec::new();
        match action {
            1 => steps.push(Preset::Fix),
            2 => steps.push(Preset::Polish),
            3 => steps.push(Preset::Business),
            _ => {}
        }
        if let Some((code, _)) = usize::try_from(target - 1).ok().and_then(|i| TARGETS.get(i)) {
            steps.push(Preset::Translate(code));
        }
        if steps.is_empty() {
            return;
        }
        self.flush();
        // What was typed could not be saved: it is not worked over.
        if self.dirty.get() {
            return;
        }
        let paragraphs = library::read_text(&path).unwrap_or_default();
        if paragraphs.iter().all(|p| p.text.trim().is_empty()) {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        *self.reworking.borrow_mut() = Some((path.clone(), Arc::clone(&stop)));
        state.set_memo_rework_busy(true);
        state.set_memo_rework_progress(0.0);
        state.set_memo_rework_note("".into());
        let german = self.german();
        std::thread::spawn(move || {
            let (server, model, language) = rework;
            let all = paragraphs.len();
            let (mut changed, mut fixed_only) = (0, 0);
            let mut out = Vec::with_capacity(all);
            let mut failed = None;
            for (i, p) in paragraphs.into_iter().enumerate() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                // Nobody waits at the keyboard: more time than for a dictation.
                let limit = (rework::time_limit_all(&steps, &p.text) * 4).max(Duration::from_secs(30));
                let deadline = std::time::Instant::now() + limit;
                let outcomes = rework::rework_all(&server, &model, &steps, &p.text, language, deadline, &stop);
                if let Some(e) = outcomes.iter().find_map(|o| match o {
                    Outcome::Original(Why::Server(e)) => Some(e.clone()),
                    _ => None,
                }) {
                    failed = Some(e);
                    break;
                }
                fixed_only += usize::from(outcomes.iter().any(|o| matches!(o, Outcome::FixedInstead(_))));
                // Each step went on from the one before: the last text is it.
                let text = outcomes.into_iter().rev().find_map(|o| match o {
                    Outcome::Reworked(text) | Outcome::FixedInstead(text) => Some(text),
                    Outcome::Original(_) => None,
                });
                match text {
                    Some(text) => {
                        changed += 1;
                        out.push(library::Paragraph {
                            seconds: p.seconds,
                            text,
                        });
                    }
                    None => out.push(p),
                }
                let done = (i + 1) as f32 / all as f32;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = memos().and_then(|m| m.window.upgrade()) {
                        w.global::<AppState>().set_memo_rework_progress(done);
                    }
                });
            }
            let result: Reworked = match failed {
                Some(e) => Err(e),
                None => Ok((out, changed, fixed_only)),
            };
            let stopped = stop.load(Ordering::Relaxed);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(m) = memos() {
                    m.reworked(&path, &stop, &steps, (!stopped).then_some(result), german);
                }
            });
        });
    }

    /// Ends the language model's work on a text; the text stays as it was.
    fn stop_rework(&self) {
        if let Some((_, stop)) = self.reworking.borrow_mut().take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_rework_busy(false);
        }
    }

    /// What the language model made of a memo's text: kept as its text, the
    /// one before kept beside it. None when it was stopped.
    fn reworked(&self, path: &Path, run: &Arc<AtomicBool>, steps: &[Preset], result: Option<Reworked>, german: bool) {
        // Stopped, and maybe another one started since.
        if self
            .reworking
            .borrow()
            .as_ref()
            .is_none_or(|(_, r)| !Arc::ptr_eq(r, run))
        {
            return;
        }
        *self.reworking.borrow_mut() = None;
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        state.set_memo_rework_busy(false);
        let Some(result) = result else {
            return;
        };
        if !path.exists() {
            return;
        }
        let note = match result {
            Err(e) => crate::servers::problem(german, &e),
            Ok((_, 0, _)) => self.say(
                "Nichts geändert: Der Text passt so, oder das Modell hat ihn zu stark verändert.",
                "Nothing changed: the text is fine as it is, or the model changed too much.",
            ),
            Ok((paragraphs, changed, fixed_only)) => {
                let all = paragraphs.iter().filter(|p| !p.text.trim().is_empty()).count();
                let written = library::keep_original(path).and_then(|_| library::write_text(path, &paragraphs));
                if let Err(e) = written {
                    eprintln!("kyrr: memo: saving the reworked text of {}: {e}", path.display());
                    state.set_memo_rework_note(
                        self.say("Konnte den Text nicht speichern.", "Could not save the text.")
                            .into(),
                    );
                    return;
                }
                if let Some(memo) = self.all.borrow_mut().iter_mut().find(|m| m.path == path) {
                    memo.text = Some(paragraphs);
                }
                self.dirty.set(false);
                if self.sheet.borrow().as_deref() == Some(path) {
                    self.show_sheet();
                }
                let done = steps
                    .iter()
                    .enumerate()
                    .map(|(i, step)| {
                        let said = match step {
                            Preset::Fix => self.say("Ausgebessert", "Fixed"),
                            Preset::Polish => self.say("Umformuliert", "Reworded"),
                            Preset::Business => self.say("Professionell formuliert", "Made professional"),
                            Preset::Translate(_) => self.say("Übersetzt", "Translated"),
                        };
                        if i == 0 {
                            said
                        } else {
                            said.to_lowercase()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(&self.say(" und ", " and "));
                let mut note = format!("{done}.");
                if fixed_only > 0 {
                    note += &if german {
                        format!(" {fixed_only} von {all} Absätzen nur ausgebessert.")
                    } else {
                        format!(" {fixed_only} of {all} paragraphs only fixed.")
                    };
                }
                if changed < all {
                    let left = all - changed;
                    note += &if german {
                        format!(" {left} von {all} unverändert.")
                    } else {
                        format!(" {left} of {all} unchanged.")
                    };
                }
                note + &self.say(" Das Original ist gesichert.", " The original is kept.")
            }
        };
        state.set_memo_rework_note(note.into());
    }

    /// The open text as it was before the first rework.
    fn restore_text(&self) {
        let Some(path) = self.sheet.borrow().clone() else {
            return;
        };
        self.stop_rework();
        self.saving.stop();
        self.dirty.set(false);
        if let Err(e) = library::restore_original(&path) {
            eprintln!("kyrr: memo: restoring the text of {}: {e}", path.display());
            if let Some(w) = self.window.upgrade() {
                w.global::<AppState>().set_memo_rework_note(
                    self.say(
                        "Konnte das Original nicht zurückholen.",
                        "Could not bring back the original.",
                    )
                    .into(),
                );
            }
            return;
        }
        if let Some(memo) = self.all.borrow_mut().iter_mut().find(|m| m.path == path) {
            memo.text = library::read_text(&path);
        }
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>()
                .set_memo_rework_note(self.say("Original wiederhergestellt.", "Original restored.").into());
        }
        self.show_sheet();
    }

    /// Where the paragraph at `cursor` in `text` starts, as "1:30"; "" when
    /// it has no time (or the text view is not open).
    fn here_label(&self, text: &str, cursor: i32) -> String {
        let Ok(sheet) = self.sheet.try_borrow() else {
            return String::new();
        };
        let (Some(path), Ok(all)) = (sheet.as_ref(), self.all.try_borrow()) else {
            return String::new();
        };
        let old = all.iter().find(|m| &m.path == path).and_then(|m| m.text.as_deref());
        usize::try_from(cursor)
            .ok()
            .and_then(|cursor| library::seconds_at(old.unwrap_or_default(), text, cursor))
            .map(library::clock)
            .unwrap_or_default()
    }

    /// Plays the memo from the paragraph at the cursor of the text field.
    fn play_here(self: &Rc<Self>, cursor: i32) {
        let (Some(w), Some(path)) = (self.window.upgrade(), self.sheet.borrow().clone()) else {
            return;
        };
        let text = w.global::<AppState>().get_memo_sheet_text();
        let seconds = usize::try_from(cursor)
            .ok()
            .and_then(|cursor| library::seconds_at(&self.paragraphs_of(&path), &text, cursor));
        if let Some(seconds) = seconds {
            self.play_path(&path, Some(f64::from(seconds)));
        }
    }

    /// The selection of the text field, or all of it, without the stamps,
    /// onto the clipboard.
    fn copy_text(&self, a: i32, b: i32) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let field = w.global::<AppState>().get_memo_sheet_text();
        let (from, to) = (a.min(b), a.max(b));
        let selected = usize::try_from(from)
            .ok()
            .zip(usize::try_from(to).ok())
            .filter(|(from, to)| from < to)
            .and_then(|(from, to)| field.get(from..to));
        let text = library::without_stamps(selected.unwrap_or(&field));
        if text.trim().is_empty() || !crate::platform::copy_to_clipboard(&text) {
            return;
        }
        w.global::<AppState>().set_memo_copied(true);
        let weak = self.window.clone();
        self.copied.start(slint::TimerMode::SingleShot, COPIED, move || {
            if let Some(w) = weak.upgrade() {
                w.global::<AppState>().set_memo_copied(false);
            }
        });
    }

    fn open_text(&self) {
        // The file as the field has it.
        self.flush();
        if let Some(path) = self.sheet.borrow().clone() {
            let text = library::text_path(&path);
            std::thread::spawn(move || library::open(&text));
        }
    }

    /// Before quitting: the memo being recorded is closed properly.
    pub fn finish(&self) {
        self.flush();
        if let Some(recording) = self.recording.borrow_mut().take() {
            recording.finish();
        }
        self.player.borrow_mut().take();
    }
}

/// When the text of the memo at `path` was last written.
fn text_stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(library::text_path(path))
        .and_then(|m| m.modified())
        .ok()
}
