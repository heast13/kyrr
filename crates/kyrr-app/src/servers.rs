//! The user's own servers in the settings: the language model that
//! reworks dictations and the speech server that can recognise them (see
//! `kyrr_server`). Asking a server happens beside the window; what comes
//! back is shown, saved and handed to the dictation worker.

use std::cell::RefCell;
use std::rc::{Rc, Weak as RcWeak};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use kyrr_dictate::Language;
use kyrr_server::rework::{self, Outcome, Preset, Why, TARGETS};
use kyrr_server::{Error, Server};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::config::{Config, Keys};
use crate::dictation::{Command, Recognizer, Reworker};
use crate::{tray, App, AppState, MainWindow, Tray};

thread_local! {
    /// For what comes back from a server, on the window's thread.
    static APP: RefCell<RcWeak<App>> = const { RefCell::new(RcWeak::new()) };
}

/// A sentence with a mishearing in it, for the test: fixed, it says
/// "Bremsbeläge".
const TEST: &str = "Ich habe heute die Brainsbelege am Auto gewechselt";

/// What the settings ask the language model to do: fix or reword, then
/// translate, or one of them.
fn steps(config: &Config) -> Vec<Preset> {
    let mut steps = Vec::new();
    match config.rework {
        1 => steps.push(Preset::Fix),
        2 => steps.push(Preset::Polish),
        _ => {}
    }
    if let Some((code, _)) = TARGETS.iter().find(|(c, _)| *c == config.translate_into) {
        steps.push(Preset::Translate(code));
    }
    steps
}

/// The language translations go into, as its place in the choice: 0 for
/// none, then in the order of `TARGETS`.
fn translate_index(config: &Config) -> i32 {
    TARGETS
        .iter()
        .position(|(c, _)| *c == config.translate_into)
        .map_or(0, |i| i as i32 + 1)
}

fn translate_code(index: i32) -> String {
    let i = index.max(0) as usize;
    if i == 0 {
        return String::new();
    }
    TARGETS.get(i - 1).map_or(String::new(), |(c, _)| (*c).to_string())
}

/// The language model, when AI features are switched on and one is set up.
fn rework_server(config: &Config, keys: &Keys) -> Option<(Server, String)> {
    if !config.ai_features {
        return None;
    }
    let url = kyrr_server::address(&config.rework_url)?;
    let model = config.rework_model.trim();
    (!model.is_empty()).then(|| {
        (
            Server {
                url,
                key: keys.rework.clone(),
            },
            model.to_string(),
        )
    })
}

fn reworker(config: &Config, keys: &Keys) -> Option<Reworker> {
    let (server, model) = rework_server(config, keys)?;
    let steps = steps(config);
    (!steps.is_empty()).then_some(Reworker { server, model, steps })
}

fn recognizer(config: &Config, keys: &Keys) -> Option<Recognizer> {
    if !config.asr_server {
        return None;
    }
    Some(Recognizer {
        server: Server {
            url: kyrr_server::address(&config.asr_url)?,
            key: keys.asr.clone(),
        },
        model: config.asr_model.trim().to_string(),
    })
}

/// What went wrong with a server, in the window's language.
fn problem(german: bool, e: &Error) -> String {
    match (e, german) {
        (Error::NotLocal, true) => "Ohne https geht nur ein Server auf diesem PC oder im Heimnetz.".into(),
        (Error::NotLocal, false) => "Without https, only a server on this computer or in your home network.".into(),
        (Error::Offline, true) => "Keine Antwort. Stimmt die Adresse, läuft der Server?".into(),
        (Error::Offline, false) => "No answer. Is the address right, is the server running?".into(),
        (Error::Timeout, true) => "Der Server antwortet zu langsam.".into(),
        (Error::Timeout, false) => "The server answers too slowly.".into(),
        (Error::Refused(401 | 403), true) => "Der Server will einen (anderen) Schlüssel.".into(),
        (Error::Refused(401 | 403), false) => "The server wants a (different) key.".into(),
        (Error::Refused(404), true) => "Dort ist kein passender Server, oder er kennt das Modell nicht.".into(),
        (Error::Refused(404), false) => "No fitting server there, or it does not know the model.".into(),
        (Error::Refused(code), true) => format!("Der Server sagt nein (Fehler {code})."),
        (Error::Refused(code), false) => format!("The server says no (error {code})."),
        (Error::Answer, true) => "Der Server antwortet, aber nicht verständlich.".into(),
        (Error::Answer, false) => "The server answers, but not in a way kyrr understands.".into(),
    }
}

fn seconds(german: bool, d: Duration) -> String {
    let s = format!("{:.1}", d.as_secs_f32());
    if german {
        s.replace('.', ",")
    } else {
        s
    }
}

fn list(names: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(names.iter().map(SharedString::from).collect::<Vec<_>>()))
}

/// Shows the saved servers and wires the settings page and the choice on
/// the main page.
pub fn setup(app: &Rc<App>, window: &MainWindow) {
    APP.set(Rc::downgrade(app));
    let state = window.global::<AppState>();
    {
        let config = app.config.borrow();
        let keys = Keys::load();
        state.set_rework_url(config.rework_url.as_str().into());
        state.set_rework_model(config.rework_model.as_str().into());
        state.set_rework_key(keys.rework.as_str().into());
        state.set_ai_features(config.ai_features);
        state.set_rework_index(config.rework.min(2) as i32);
        state.set_translate_index(translate_index(&config));
        state.set_rework_ready(rework_server(&config, &keys).is_some());
        state.set_asr_on(config.asr_server);
        state.set_asr_url(config.asr_url.as_str().into());
        state.set_asr_model(config.asr_model.as_str().into());
        state.set_asr_key(keys.asr.as_str().into());
        let _ = app.worker.send(Command::Rework(reworker(&config, &keys)));
        let _ = app.worker.send(Command::Recognizer(recognizer(&config, &keys)));
    }
    state.on_ai_features_toggled({
        let app = Rc::clone(app);
        move |on| {
            app.config.borrow_mut().ai_features = on;
            app.config.borrow().save();
            send(&app);
            crate::fit(&app.window);
        }
    });
    state.on_rework_changed({
        let app = Rc::clone(app);
        move |i| {
            app.config.borrow_mut().rework = i.clamp(0, 2) as u8;
            app.config.borrow().save();
            send(&app);
        }
    });
    state.on_translate_changed({
        let app = Rc::clone(app);
        move |i| {
            app.config.borrow_mut().translate_into = translate_code(i);
            app.config.borrow().save();
            send(&app);
        }
    });
    state.on_rework_connect({
        let app = Rc::clone(app);
        move || connect_rework(&app)
    });
    state.on_rework_model_picked({
        let app = Rc::clone(app);
        move |i| {
            let Some(w) = app.window.upgrade() else { return };
            let state = w.global::<AppState>();
            if let Some(name) = state.get_rework_models().row_data(i as usize) {
                state.set_rework_model(name.clone());
                app.config.borrow_mut().rework_model = name.into();
                app.config.borrow().save();
                send(&app);
            }
        }
    });
    state.on_asr_toggled({
        let app = Rc::clone(app);
        move |on| {
            app.config.borrow_mut().asr_server = on;
            app.config.borrow().save();
            if let Some(w) = app.window.upgrade() {
                w.global::<AppState>().set_asr_on(on);
            }
            send(&app);
            crate::fit(&app.window);
        }
    });
    state.on_asr_connect({
        let app = Rc::clone(app);
        move || connect_asr(&app)
    });
    state.on_asr_model_picked({
        let app = Rc::clone(app);
        move |i| {
            let Some(w) = app.window.upgrade() else { return };
            let state = w.global::<AppState>();
            if let Some(name) = state.get_asr_models().row_data(i as usize) {
                state.set_asr_model(name.clone());
                app.config.borrow_mut().asr_model = name.into();
                app.config.borrow().save();
                send(&app);
            }
        }
    });
}

/// The tray's quick choice of what the language model does.
pub fn wire_tray(app: &Rc<App>, tray: &Tray) {
    show_tray(app, tray);
    tray.on_rework_picked({
        let app = Rc::clone(app);
        move |i| {
            {
                let mut config = app.config.borrow_mut();
                config.rework = i.clamp(0, 2) as u8;
                config.save();
            }
            send(&app);
        }
    });
    tray.on_translate_picked({
        let app = Rc::clone(app);
        move |i| {
            {
                let mut config = app.config.borrow_mut();
                config.translate_into = translate_code(i);
                config.save();
            }
            send(&app);
        }
    });
}

fn show_tray(app: &App, tray: &Tray) {
    let config = app.config.borrow();
    tray.set_rework_ready(rework_server(&config, &Keys::load()).is_some());
    tray.set_rework_index(config.rework.min(2) as i32);
    tray.set_translate_index(translate_index(&config));
    if let Some(w) = app.window.upgrade() {
        tray.set_translate_options(w.global::<AppState>().get_translate_options());
    }
}

/// The settings as they are now, to the worker, the window and the tray.
fn send(app: &App) {
    let config = app.config.borrow().clone();
    let keys = Keys::load();
    let _ = app.worker.send(Command::Rework(reworker(&config, &keys)));
    let _ = app.worker.send(Command::Recognizer(recognizer(&config, &keys)));
    if let Some(w) = app.window.upgrade() {
        let state = w.global::<AppState>();
        state.set_ai_features(config.ai_features);
        state.set_rework_index(config.rework.min(2) as i32);
        state.set_translate_index(translate_index(&config));
        state.set_rework_ready(rework_server(&config, &keys).is_some());
    }
    if let Some(t) = tray() {
        show_tray(app, &t);
    }
}

/// Runs `work` beside the window and hands its result to `done` on the
/// window's thread.
fn beside<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&Rc<App>, T) + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = work();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = APP.with_borrow(RcWeak::upgrade) {
                done(&app, result);
            }
        });
    });
}

/// Saves what was typed for the language model, asks the server for its
/// models and fixes one sentence with it.
fn connect_rework(app: &Rc<App>) {
    let Some(w) = app.window.upgrade() else { return };
    let state = w.global::<AppState>();
    let german = app.language() == Language::German;
    let Some(url) = kyrr_server::address(&state.get_rework_url()) else {
        state.set_rework_note(
            if german {
                "Erst eine Adresse eintragen."
            } else {
                "First an address."
            }
            .into(),
        );
        return;
    };
    state.set_rework_url(url.as_str().into());
    let key = state.get_rework_key().trim().to_string();
    let typed = state.get_rework_model().trim().to_string();
    {
        let mut keys = Keys::load();
        keys.rework = key.clone();
        keys.save();
        app.config.borrow_mut().rework_url = url.clone();
        app.config.borrow().save();
    }
    state.set_rework_busy(true);
    state.set_rework_note("".into());
    let server = Server { url, key };
    beside(
        move || {
            let models = kyrr_server::models(&server, Duration::from_secs(5));
            let names = models.clone().unwrap_or_default();
            let model = if !typed.is_empty() && (names.is_empty() || names.contains(&typed)) {
                typed
            } else {
                names.first().cloned().unwrap_or(typed)
            };
            let tested = match &models {
                Err(e @ (Error::NotLocal | Error::Offline | Error::Refused(401 | 403))) => Err(e.clone()),
                _ if model.is_empty() => Err(Error::Refused(404)),
                _ => {
                    let started = Instant::now();
                    let deadline = started + Duration::from_secs(30);
                    let outcome = rework::rework(
                        &server,
                        &model,
                        Preset::Fix,
                        TEST,
                        Some("de"),
                        deadline,
                        &AtomicBool::new(false),
                    );
                    Ok((outcome, started.elapsed()))
                }
            };
            (names, model, tested)
        },
        move |app, (names, model, tested)| {
            let Some(w) = app.window.upgrade() else { return };
            let state = w.global::<AppState>();
            state.set_rework_busy(false);
            state.set_rework_models(list(&names));
            state.set_rework_model_index(names.iter().position(|n| *n == model).map_or(-1, |i| i as i32));
            state.set_rework_model(model.as_str().into());
            let note = match tested {
                Err(e) => problem(german, &e),
                Ok((Outcome::Reworked(text), took)) if german => format!("Geht, {} s: „{text}“", seconds(german, took)),
                Ok((Outcome::Reworked(text), took)) => format!("Works, {} s: “{text}”", seconds(german, took)),
                Ok((Outcome::Original(Why::Server(e)), _)) => problem(german, &e),
                Ok((Outcome::Original(_), took)) if german => format!(
                    "Verbunden ({} s), aber der Testsatz kam unverändert zurück. Ein anderes Modell probieren?",
                    seconds(german, took)
                ),
                Ok((Outcome::Original(_), took)) => format!(
                    "Connected ({} s), but the test sentence came back unchanged. Try another model?",
                    seconds(german, took)
                ),
            };
            state.set_rework_note(note.into());
            if !model.is_empty() {
                app.config.borrow_mut().rework_model = model;
                app.config.borrow().save();
            }
            send(app);
            crate::fit(&app.window);
        },
    );
}

/// Saves what was typed for the speech server, asks it for its models and
/// sends it a second of silence.
fn connect_asr(app: &Rc<App>) {
    let Some(w) = app.window.upgrade() else { return };
    let state = w.global::<AppState>();
    let german = app.language() == Language::German;
    let Some(url) = kyrr_server::address(&state.get_asr_url()) else {
        state.set_asr_note(
            if german {
                "Erst eine Adresse eintragen."
            } else {
                "First an address."
            }
            .into(),
        );
        return;
    };
    state.set_asr_url(url.as_str().into());
    let key = state.get_asr_key().trim().to_string();
    let typed = state.get_asr_model().trim().to_string();
    {
        let mut keys = Keys::load();
        keys.asr = key.clone();
        keys.save();
        app.config.borrow_mut().asr_url = url.clone();
        app.config.borrow().save();
    }
    state.set_asr_busy(true);
    state.set_asr_note("".into());
    let server = Server { url, key };
    let language = (!app.config.borrow().more_languages()).then(|| app.language().code());
    beside(
        move || {
            // whisper.cpp has no list of models: the test says whether it works.
            let names = kyrr_server::models(&server, Duration::from_secs(5)).unwrap_or_default();
            let model = if !typed.is_empty() {
                typed
            } else {
                names.first().cloned().unwrap_or_default()
            };
            let started = Instant::now();
            let wav = kyrr_server::wav(&vec![0.0; 16_000]);
            let tested = kyrr_server::transcribe(&server, &model, &wav, language, Duration::from_secs(20))
                .map(|_| started.elapsed());
            (names, model, tested)
        },
        move |app, (names, model, tested)| {
            let Some(w) = app.window.upgrade() else { return };
            let state = w.global::<AppState>();
            state.set_asr_busy(false);
            state.set_asr_models(list(&names));
            state.set_asr_model_index(names.iter().position(|n| *n == model).map_or(-1, |i| i as i32));
            state.set_asr_model(model.as_str().into());
            let note = match tested {
                Ok(took) if german => format!("Geht, {} s für eine Sekunde Stille.", seconds(german, took)),
                Ok(took) => format!("Works, {} s for a second of silence.", seconds(german, took)),
                Err(e) => problem(german, &e),
            };
            state.set_asr_note(note.into());
            app.config.borrow_mut().asr_model = model;
            app.config.borrow().save();
            send(app);
            crate::fit(&app.window);
        },
    );
}

/// The own-models page opened with AI features on and no language model
/// set up: kyrr looks for servers on this computer and fills in what it
/// finds.
pub fn look_around(app: &Rc<App>) {
    let Some(w) = app.window.upgrade() else { return };
    let state = w.global::<AppState>();
    if state.get_page() != 7 || !state.get_ai_features() || !state.get_rework_url().is_empty() {
        return;
    }
    let german = app.language() == Language::German;
    beside(kyrr_server::find_local, move |app, found| {
        let Some(w) = app.window.upgrade() else { return };
        let state = w.global::<AppState>();
        if !state.get_rework_url().is_empty() {
            return;
        }
        let Some((name, url)) = found.into_iter().find(|(name, _)| *name != "speaches") else {
            return;
        };
        state.set_rework_url(url.into());
        state.set_rework_note(
            if german {
                format!("{name} läuft auf diesem PC. „Verbinden und testen“ übernimmt es.")
            } else {
                format!("{name} runs on this computer. “Connect and test” takes it.")
            }
            .into(),
        );
        crate::fit(&app.window);
    });
}
