//! The voice memos in the window: recording, the list, playing, the texts.
//! Everything here runs on the window's thread; the recording and playing
//! themselves have their own threads (see [`crate::memo`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use kyrr_common::wave::{Mode, Overlay};
use kyrr_dictate::Language;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::dictation::{Command, Job, Text};
use crate::memo::library::{self, Memo, Paragraph};
use crate::memo::player::Player;
use crate::memo::recorder::{Ending, Recording};
use crate::{AppState, MainWindow, MemoItem, MemoParagraph};

/// Shorter memos are a key pressed twice by accident: not kept.
const SHORTEST: f64 = 1.0;
/// How often the clock and the playing position move on.
const TICK: Duration = Duration::from_millis(200);
/// How long "Copied" stays on the button.
const COPIED: Duration = Duration::from_secs(2);

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
    recording: RefCell<Option<Recording>>,
    player: RefCell<Option<Player>>,
    /// Memos whose title is still being found (they keep their first name
    /// until then), and the texts on their way.
    titling: RefCell<Vec<PathBuf>>,
    texts: RefCell<HashMap<PathBuf, Text>>,
    /// The memo whose text is open big.
    sheet: RefCell<Option<PathBuf>>,
    timer: slint::Timer,
    copied: slint::Timer,
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
        let memos = Rc::new(Memos {
            window: window.as_weak(),
            worker,
            wave,
            dir: library::folder(),
            language: Cell::new(language),
            active,
            dictating,
            microphone: RefCell::new(None),
            enabled: Cell::new(true),
            auto_text: Cell::new(auto_text),
            all: RefCell::new(Vec::new()),
            shown: RefCell::new(Vec::new()),
            recording: RefCell::new(None),
            player: RefCell::new(None),
            titling: RefCell::new(Vec::new()),
            texts: RefCell::new(HashMap::new()),
            sheet: RefCell::new(None),
            timer: slint::Timer::default(),
            copied: slint::Timer::default(),
        });
        MEMOS.set(Some(Rc::clone(&memos)));
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
        state.on_memo_copy_text(with0(|m| m.copy_text()));
        state.on_memo_open_text(with0(|m| m.open_text()));
        state.on_memo_open_folder(with0(|m| m.open_folder()));
        let weak = Rc::downgrade(self);
        state.on_memo_play_from(move |i, seconds| {
            if let Some(m) = weak.upgrade() {
                m.play(i, Some(seconds.max(0) as f64));
            }
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

    /// Reads the folder again and shows it.
    pub fn refresh(&self) {
        let recording = self.recording.borrow().as_ref().map(|r| r.path().to_path_buf());
        let mut all = library::list(&self.dir);
        all.retain(|m| Some(&m.path) != recording.as_ref());
        *self.all.borrow_mut() = all;
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
        let fits = |m: &Memo| {
            query.is_empty()
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
        *self.shown.borrow_mut() = shown.iter().map(|m| m.path.clone()).collect();
        state.set_memo_count(all.len() as i32);
        state.set_memo_items(ModelRc::new(VecModel::from(items)));
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
                self.wave
                    .set_words(vec![self.say("Aufnahme", "Recording"), self.say("läuft", "on")]);
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
                        "Konnte die Aufnahme nicht anlegen. Ist die Festplatte voll?",
                        "Could not create the recording. Is the disk full?",
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
        if finished.seconds < SHORTEST && finished.ending == Ending::Stopped {
            let _ = std::fs::remove_file(&path);
        } else if path.exists() {
            self.titling.borrow_mut().push(path.clone());
            let _ = self
                .worker
                .send(Command::Job(Job::Title(path, finished.rate, finished.start)));
        }
        let message = match finished.ending {
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
        self.tell(&message);
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
        let Some(path) = self.path(index) else {
            return;
        };
        let same = self.player.borrow().as_ref().is_some_and(|p| p.path() == path);
        self.player.borrow_mut().take();
        if !same || from.is_some() {
            *self.player.borrow_mut() = Some(Player::start(&path, from.unwrap_or(0.0)));
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
        let _ = std::fs::create_dir_all(&self.dir);
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
        self.texts.borrow_mut().remove(&path);
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
        let path = match title {
            Some(title) if path.exists() => match library::rename(&path, &title) {
                Ok(to) => {
                    self.moved(&path, &to);
                    to
                }
                Err(_) => path,
            },
            _ => path,
        };
        if self.auto_text.get() && path.exists() {
            let _ = self.worker.send(Command::Job(Job::Text(path)));
        }
        self.refresh();
    }

    /// How the text of a memo comes along.
    pub fn text(&self, path: PathBuf, text: Text) {
        match text {
            Text::Written | Text::Cancelled => {
                self.texts.borrow_mut().remove(&path);
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

    fn show_text(&self, index: i32) {
        let Some(path) = self.path(index) else {
            return;
        };
        *self.sheet.borrow_mut() = Some(path);
        self.show_sheet();
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.global::<AppState>().set_memo_sheet_open(true);
        crate::fit_sheet(&w, true);
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
        let paragraphs: Vec<MemoParagraph> = memo
            .text
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|p| MemoParagraph {
                time: if p.seconds == u32::MAX {
                    SharedString::new()
                } else {
                    library::clock(p.seconds).into()
                },
                seconds: if p.seconds == u32::MAX { -1 } else { p.seconds as i32 },
                text: p.text.as_str().into(),
            })
            .collect();
        state.set_memo_sheet_title(memo.title.as_str().into());
        state.set_memo_sheet_when(self.when(memo).into());
        state.set_memo_sheet_paragraphs(ModelRc::new(VecModel::from(paragraphs)));
        state.set_memo_copied(false);
        drop(all);
        drop(sheet);
        self.show_list();
    }

    fn close_text(&self) {
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memo_sheet_open(false);
            crate::fit_sheet(&w, false);
        }
    }

    fn sheet_text(&self) -> Option<(PathBuf, Vec<Paragraph>)> {
        let sheet = self.sheet.borrow().clone()?;
        let all = self.all.borrow();
        let memo = all.iter().find(|m| m.path == sheet)?;
        Some((sheet, memo.text.clone()?))
    }

    fn copy_text(&self) {
        let Some((_, paragraphs)) = self.sheet_text() else {
            return;
        };
        if !crate::platform::copy_to_clipboard(&library::plain(&paragraphs)) {
            return;
        }
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.global::<AppState>().set_memo_copied(true);
        let weak = self.window.clone();
        self.copied.start(slint::TimerMode::SingleShot, COPIED, move || {
            if let Some(w) = weak.upgrade() {
                w.global::<AppState>().set_memo_copied(false);
            }
        });
    }

    fn open_text(&self) {
        if let Some((path, _)) = self.sheet_text() {
            let text = library::text_path(&path);
            std::thread::spawn(move || library::open(&text));
        }
    }

    /// Before quitting: the memo being recorded is closed properly.
    pub fn finish(&self) {
        if let Some(recording) = self.recording.borrow_mut().take() {
            recording.finish();
        }
        self.player.borrow_mut().take();
    }
}
