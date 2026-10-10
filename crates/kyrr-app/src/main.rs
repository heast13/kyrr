//! kyrr: clean microphone and push-to-talk dictation in the tray.
//!
//! `kyrr.exe --setup <command>` runs an elevated setup step (see
//! `kyrr_windows::run_setup_command`); `--background` starts without
//! showing the window.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod dictation;
mod platform;
mod recorder;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use kyrr_common::FilterSettings;
use kyrr_dictate::Language;
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedString, VecModel, Weak};

use config::Config;
use dictation::{Command, Event, State};
use platform::{FilterStatus, Microphone, SetupError};

slint::include_modules!();

/// Maximum attenuation for the strength choices gentle, medium, strong.
const STRENGTHS: [u32; 3] = [24, 48, 100];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.get(1).map(String::as_str) == Some("--setup") {
        std::process::exit(kyrr_windows::run_setup_command(&args[2..]));
    }
    #[cfg(windows)]
    let show_requests = match kyrr_windows::instance::claim() {
        kyrr_windows::instance::Instance::First { show_requests } => Some(show_requests),
        kyrr_windows::instance::Instance::Other => return,
    };
    #[cfg(not(windows))]
    let show_requests: Option<mpsc::Receiver<()>> = None;

    let background = args.iter().any(|a| a == "--background");
    if let Err(e) = run(background, show_requests) {
        eprintln!("kyrr: {e}");
    }
}

/// Texts set from Rust; everything else is translated in the .slint file.
fn text(language: Language, english: &str) -> String {
    let german = match english {
        "permission" => "Windows hat die Erlaubnis nicht erteilt. Bitte versuche es noch einmal.",
        "setup failed" => "Die Einrichtung ist fehlgeschlagen:",
        "disconnected" => "nicht angeschlossen",
        other => other,
    };
    let english = match english {
        "permission" => "Windows did not grant permission. Please try again.",
        "setup failed" => "Setup failed:",
        "disconnected" => "disconnected",
        other => other,
    };
    match language {
        Language::German => german.into(),
        Language::English => english.into(),
    }
}

fn megabytes(language: Language) -> String {
    format!("{} MB", language.model().total_size() / 1_000_000)
}

fn strength_index(limit_db: u32) -> i32 {
    STRENGTHS.iter().position(|&s| s >= limit_db).unwrap_or(2) as i32
}

/// Resizes the window to its content after the page changed.
fn fit(window: &Weak<MainWindow>) {
    let window = window.clone();
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        if let Some(w) = window.upgrade() {
            w.window().set_size(LogicalSize::new(380.0, w.get_content_height()));
        }
    });
}

fn show(window: &MainWindow) {
    let _ = window.show();
    window.window().set_minimized(false);
}

struct App {
    config: RefCell<Config>,
    filter: RefCell<FilterSettings>,
    microphones: RefCell<Vec<Microphone>>,
    worker: mpsc::Sender<Command>,
    window: Weak<MainWindow>,
    tray: Weak<Tray>,
}

impl App {
    fn language(&self) -> Language {
        self.config.borrow().language()
    }

    fn refresh_microphones(&self) {
        let mics = platform::microphones();
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let names: Vec<SharedString> = mics.iter().map(|m| m.name.as_str().into()).collect();
        w.global::<AppState>()
            .set_microphones(ModelRc::new(VecModel::from(names)));
        *self.microphones.borrow_mut() = mics;
    }

    fn refresh_filter(&self) {
        let (Some(w), Some(tray)) = (self.window.upgrade(), self.tray.upgrade()) else {
            return;
        };
        let state = w.global::<AppState>();
        if state.get_filter_state() == FilterState::SettingUp {
            return;
        }
        let name = |guid: &str| {
            self.microphones
                .borrow()
                .iter()
                .find(|m| m.guid == guid)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| text(self.language(), "disconnected"))
        };
        let (fs, mic, detail) = match platform::filter_status() {
            FilterStatus::NotInstalled => (FilterState::NotInstalled, String::new(), String::new()),
            FilterStatus::NotAttached => (FilterState::NotAttached, String::new(), String::new()),
            FilterStatus::Waiting(g) => (FilterState::Checking, name(&g), String::new()),
            FilterStatus::Running(g, d) => (FilterState::Running, name(&g), d),
            FilterStatus::Passthrough(g, d) => (FilterState::Passthrough, name(&g), d),
        };
        if state.get_filter_state() != FilterState::Error || fs != FilterState::NotAttached {
            state.set_filter_state(fs);
        }
        state.set_attached_mic(mic.into());
        state.set_filter_detail(detail.into());
        let filter = *self.filter.borrow();
        state.set_filter_enabled(filter.enabled);
        tray.set_filter_enabled(filter.enabled);
        tray.set_active(filter.enabled && matches!(fs, FilterState::Running | FilterState::Checking));
    }

    fn set_filter(&self, change: impl FnOnce(&mut FilterSettings)) {
        let mut filter = self.filter.borrow_mut();
        change(&mut filter);
        let _ = platform::set_filter_settings(&filter);
        drop(filter);
        self.refresh_filter();
    }

    fn set_language(&self, language: Language) {
        {
            let mut config = self.config.borrow_mut();
            config.language = language.code().into();
            config.save();
        }
        let _ = slint::select_bundled_translation(language.code());
        let _ = self.worker.send(Command::Language(language));
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_language_index(language as i32);
            state.set_download_size(megabytes(language).into());
        }
        self.refresh_filter();
    }

    fn set_dictation(&self, on: bool) {
        {
            let mut config = self.config.borrow_mut();
            config.dictation = on;
            config.save();
        }
        let _ = self.worker.send(Command::Enable(on));
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_dictation_enabled(on);
        }
        if let Some(t) = self.tray.upgrade() {
            t.set_dictation_enabled(on);
        }
    }

    fn attach(self: &Rc<Self>, index: usize) {
        let Some(mic) = self.microphones.borrow().get(index).cloned() else {
            return;
        };
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.global::<AppState>().set_filter_state(FilterState::SettingUp);
        let weak = self.window.clone();
        let language = self.language();
        std::thread::spawn(move || {
            let result = platform::attach(&mic.guid);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(w) = weak.upgrade() else {
                    return;
                };
                let state = w.global::<AppState>();
                match result {
                    Ok(()) => {
                        state.set_filter_state(FilterState::Checking);
                        if w.get_setup_mode() {
                            w.set_setup_step(1);
                            fit(&weak);
                        }
                    }
                    Err(e) => {
                        let msg = match e {
                            SetupError::Declined => text(language, "permission"),
                            SetupError::Failed(e) => format!("{} {e}", text(language, "setup failed")),
                        };
                        state.set_error_text(msg.into());
                        state.set_filter_state(FilterState::Error);
                    }
                }
            });
        });
    }
}

fn dictation_state(s: State) -> DictationState {
    match s {
        State::Off => DictationState::Off,
        State::NoModel => DictationState::NoModel,
        State::Downloading => DictationState::Downloading,
        State::Loading => DictationState::Loading,
        State::Ready => DictationState::Ready,
        State::Recording => DictationState::Recording,
        State::Transcribing => DictationState::Transcribing,
        State::Error => DictationState::Error,
    }
}

fn run(background: bool, show_requests: Option<mpsc::Receiver<()>>) -> Result<(), slint::PlatformError> {
    let config = Config::load();
    let language = config.language();
    let window = MainWindow::new()?;
    let tray = Tray::new()?;
    let _ = slint::select_bundled_translation(language.code());

    // Dictation worker, reporting back on the UI thread.
    let weak = window.as_weak();
    let weak_tray = tray.as_weak();
    let notify: dictation::Notify = Arc::new(move |event| {
        let (weak, weak_tray) = (weak.clone(), weak_tray.clone());
        let _ = slint::invoke_from_event_loop(move || {
            let (Some(w), Some(t)) = (weak.upgrade(), weak_tray.upgrade()) else {
                return;
            };
            let state = w.global::<AppState>();
            match event {
                Event::State(s) => {
                    state.set_dictation_state(dictation_state(s));
                    t.set_recording(s == State::Recording);
                    platform::set_hotkey_active(matches!(s, State::Ready | State::Recording | State::Transcribing));
                }
                Event::Progress(p) => state.set_download_progress(p),
                Event::Typed(text) => state.set_last_text(text.into()),
                Event::Error(e) => state.set_error_text(e.into()),
            }
        });
    });
    let worker = dictation::spawn(
        config::models_dir(),
        language,
        config.dictation,
        notify,
        platform::type_text,
    );

    if let Some(keys) = platform::start_hotkey() {
        let worker = worker.clone();
        std::thread::spawn(move || {
            while let Ok(key) = keys.recv() {
                let cmd = match key {
                    platform::KeyEvent::Pressed => Command::KeyDown,
                    platform::KeyEvent::Released => Command::KeyUp,
                };
                if worker.send(cmd).is_err() {
                    break;
                }
            }
        });
    }

    let filter = platform::filter_settings();
    let app = Rc::new(App {
        config: RefCell::new(config),
        filter: RefCell::new(filter),
        microphones: RefCell::new(Vec::new()),
        worker: worker.clone(),
        window: window.as_weak(),
        tray: tray.as_weak(),
    });

    let state = window.global::<AppState>();
    state.set_version(env!("CARGO_PKG_VERSION").into());
    state.set_language_index(language as i32);
    state.set_download_size(megabytes(language).into());
    state.set_dictation_enabled(app.config.borrow().dictation);
    state.set_strength_index(strength_index(filter.limit_db));
    tray.set_dictation_enabled(app.config.borrow().dictation);
    app.refresh_microphones();
    app.refresh_filter();

    state.on_filter_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_filter(|f| f.enabled = on)
    });
    state.on_strength_changed({
        let app = Rc::clone(&app);
        move |i| app.set_filter(|f| f.limit_db = STRENGTHS[(i as usize).min(2)])
    });
    state.on_attach_microphone({
        let app = Rc::clone(&app);
        move |i| app.attach(i.max(0) as usize)
    });
    state.on_dictation_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_dictation(on)
    });
    state.on_language_changed({
        let app = Rc::clone(&app);
        move |i| app.set_language(if i == 1 { Language::English } else { Language::German })
    });
    state.on_download_model({
        let worker = worker.clone();
        move || {
            let _ = worker.send(Command::Download);
        }
    });
    state.on_cancel_download({
        let worker = worker.clone();
        move || {
            let _ = worker.send(Command::CancelDownload);
        }
    });
    state.on_finish_setup({
        let app = Rc::clone(&app);
        move || {
            {
                let mut config = app.config.borrow_mut();
                config.setup_done = true;
                config.save();
            }
            platform::enable_autostart();
            if let Some(w) = app.window.upgrade() {
                w.set_setup_mode(false);
                fit(&app.window);
            }
        }
    });
    state.on_change_microphone({
        let app = Rc::clone(&app);
        move || {
            app.refresh_microphones();
            if let Some(w) = app.window.upgrade() {
                w.set_setup_step(0);
                w.set_setup_mode(true);
                fit(&app.window);
            }
        }
    });
    let quit = {
        let worker = worker.clone();
        move || {
            let _ = worker.send(Command::Quit);
            let _ = slint::quit_event_loop();
        }
    };
    state.on_quit(quit.clone());
    tray.on_quit(quit);
    tray.on_open({
        let app = Rc::clone(&app);
        move || {
            if let Some(w) = app.window.upgrade() {
                app.refresh_microphones();
                app.refresh_filter();
                show(&w);
            }
        }
    });
    tray.on_toggle_filter({
        let app = Rc::clone(&app);
        move || {
            let on = !app.filter.borrow().enabled;
            app.set_filter(|f| f.enabled = on);
        }
    });
    tray.on_toggle_dictation({
        let app = Rc::clone(&app);
        move || {
            let on = !app.config.borrow().dictation;
            app.set_dictation(on);
        }
    });

    // The filter's state can change outside the app (stream starts, errors).
    let poll = slint::Timer::default();
    poll.start(slint::TimerMode::Repeated, Duration::from_secs(2), {
        let app = Rc::clone(&app);
        move || app.refresh_filter()
    });

    if let Some(requests) = show_requests {
        let weak = window.as_weak();
        std::thread::spawn(move || {
            while requests.recv().is_ok() {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = weak.upgrade() {
                        show(&w);
                    }
                });
            }
        });
    }

    let setup_done = app.config.borrow().setup_done;
    window.set_setup_mode(!setup_done);
    if !setup_done || !background {
        show(&window);
        fit(&window.as_weak());
    }
    tray.show()?;
    slint::run_event_loop_until_quit()
}
