//! kyrr: noise removal, voice polish and push-to-talk dictation in the tray.
//!
//! `kyrr.exe --setup <command>` runs an elevated setup step (see
//! `kyrr_windows::run_setup_command`); `--background` starts without
//! showing the window.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod dictation;
mod listen;
mod platform;
mod player;
mod probe;
mod recorder;
mod update;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use kyrr_common::{Amount, FilterSettings, Look, VoiceSettings};
use kyrr_dictate::Language;
use slint::{ComponentHandle, LogicalSize, ModelRc, PhysicalPosition, SharedString, VecModel, Weak};

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
        "probe failed" => "Der Test hat nicht geklappt:",
        "listen failed" => "Mithören hat nicht geklappt:",
        "right ctrl" => "Strg rechts",
        other => other,
    };
    let english = match english {
        "permission" => "Windows did not grant permission. Please try again.",
        "setup failed" => "Setup failed:",
        "disconnected" => "disconnected",
        "probe failed" => "The test did not work:",
        "listen failed" => "Listening did not work:",
        "right ctrl" => "Right Ctrl",
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

/// The voice shaping with its defaults. Whether it is on and the automatic
/// level (a main switch of its own) stay as they are.
fn voice_defaults(v: &VoiceSettings) -> VoiceSettings {
    VoiceSettings {
        enabled: v.enabled,
        auto_level: v.auto_level,
        ..VoiceSettings::default()
    }
}

fn amount(i: i32) -> Amount {
    Amount::from_index(i.max(0) as u32).unwrap_or(Amount::Off)
}

/// Shows the voice settings in the window.
fn show_voice(state: &AppState, v: &VoiceSettings) {
    state.set_voice_enabled(v.enabled);
    state.set_look_index(v.look as i32);
    state.set_body_db(v.body as f32 / 10.0);
    state.set_presence_db(v.presence as f32 / 10.0);
    state.set_air_db(v.air as f32 / 10.0);
    state.set_low_cut(v.low_cut);
    state.set_pops(v.pops);
    state.set_auto_level(v.auto_level);
    state.set_deesser_index(v.deesser as i32);
    state.set_compressor_index(v.compressor as i32);
    state.set_voice_is_default(*v == voice_defaults(v));
}

/// Tallest the window gets, screen permitting; more content scrolls. Room
/// for the whole two-column settings page.
const MAX_HEIGHT: f32 = 1100.0;
/// Tallest when the screen is unknown: fits a 768-pixel-high laptop screen
/// with the taskbar and title bar.
const FALLBACK_HEIGHT: f32 = 660.0;
/// Never smaller than this, however small the screen.
const MIN_HEIGHT: f32 = 400.0;
/// Title bar and borders, about, plus a little air.
const FRAME: f32 = 40.0;
/// Borders left and right, about.
const FRAME_SIDES: f32 = 16.0;

/// Left, top, right and bottom of the free screen the window is on, in
/// physical pixels.
fn screen(w: &MainWindow) -> Option<(i32, i32, i32, i32)> {
    let pos = w.window().position();
    platform::work_area(pos.x, pos.y)
}

/// The height the content wants, up to what the screen has room for.
fn wanted_height(w: &MainWindow) -> f32 {
    let cap = screen(w).map_or(FALLBACK_HEIGHT, |(_, top, _, bottom)| {
        ((bottom - top) as f32 / w.window().scale_factor() - FRAME).clamp(MIN_HEIGHT, MAX_HEIGHT)
    });
    w.get_content_height().min(cap)
}

/// Sets the window's height (its width is the page's) and moves it up or
/// left when it would end up behind the taskbar or off the screen.
fn resize(w: &MainWindow, height: f32) {
    let win = w.window();
    let width = w.get_page_width();
    win.set_size(LogicalSize::new(width, height));
    let Some((left, top, right, bottom)) = screen(w) else {
        return;
    };
    let pos = win.position();
    let scale = win.scale_factor();
    let outer_width = ((width + FRAME_SIDES) * scale).round() as i32;
    let outer_height = ((height + FRAME) * scale).round() as i32;
    let x = if pos.x + outer_width > right {
        (right - outer_width).max(left)
    } else {
        pos.x
    };
    let y = if pos.y + outer_height > bottom {
        (bottom - outer_height).max(top)
    } else {
        pos.y
    };
    if (x, y) != (pos.x, pos.y) {
        win.set_position(PhysicalPosition::new(x, y));
    }
}

/// Resizes the window to its content, once the layout has caught up (after
/// a page change, or the window just came up). Not while minimized: the
/// size would land on the taskbar button, not the window.
fn fit(window: &Weak<MainWindow>) {
    let window = window.clone();
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        if let Some(w) = window.upgrade() {
            if !w.window().is_minimized() {
                resize(&w, wanted_height(&w));
            }
        }
    });
}

/// Makes the open window taller when its content grew (a hint appeared, the
/// status went to two lines). Never shrinks it on its own, so it does not
/// jump with every phrase; the next fit tidies up.
fn grow(window: &Weak<MainWindow>) {
    let window = window.clone();
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(w) = window.upgrade() else { return };
        let win = w.window();
        if !win.is_visible() || win.is_minimized() {
            return;
        }
        let height = win.size().to_logical(win.scale_factor()).height;
        let wanted = wanted_height(&w);
        if wanted > height + 0.5 {
            resize(&w, wanted);
        }
    });
}

/// How often the status line moves on to its next phrase, in 100 ms ticks.
const PHRASE_TICKS: u32 = 150;
/// The phrase fades out this many ticks before the next one fades in
/// (matches the fade in the .slint file).
const PHRASE_FADE_TICKS: u32 = 5;

/// Moves the status line on to its next phrase (the .slint side takes it
/// modulo the number of phrases, so it only has to stay non-negative).
fn next_phrase(state: &AppState) {
    state.set_phrase(state.get_phrase().wrapping_add(1) & i32::MAX);
}

/// A linear peak on the meter's scale: −60 dBFS to 0 dBFS.
fn meter(peak: f32) -> f32 {
    if peak <= 0.0 || !peak.is_finite() {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Below this peak (−120 dBFS) the microphone delivers nothing at all; even
/// a quiet room's noise floor is far above it.
const SILENCE: f32 = 1e-6;
/// How long the microphone has to be that silent before kyrr says so.
const SILENCE_TICKS: u32 = 20;

/// After a model switch, the message shows until the filter reports the
/// new model or this long has passed.
const MODEL_SWITCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Brings the window up at the size its content wants, in front of
/// everything. A window kyrr started with in the background never had a
/// size, and content changes while it was hidden; both used to show it too
/// small. Showing alone left it behind the current window at times.
fn show(window: &MainWindow) {
    if !window.window().is_visible() {
        resize(window, wanted_height(window));
    }
    let _ = window.show();
    window.window().set_minimized(false);
    platform::bring_to_front(window.window());
    // Once more when the window has really come up.
    let weak = window.as_weak();
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        if let Some(w) = weak.upgrade() {
            platform::bring_to_front(w.window());
        }
    });
    fit(&window.as_weak());
    window.invoke_shown();
}

struct App {
    config: RefCell<Config>,
    filter: RefCell<FilterSettings>,
    /// Endpoint GUID of the microphone the filter is on.
    attached: RefCell<Option<String>>,
    monitor: RefCell<Option<platform::Monitor>>,
    /// Runs `tick` while the window is open; nobody sees the meter otherwise.
    meter: slint::Timer,
    heartbeat: Cell<u32>,
    ticks: Cell<u32>,
    /// Ticks in a row the microphone was in use but silent.
    silent_ticks: Cell<u32>,
    model_changed: Cell<Option<Instant>>,
    /// Events from the probe thread, picked up by `tick`.
    probe_events: (mpsc::Sender<probe::Event>, mpsc::Receiver<probe::Event>),
    /// The last before/after take, for playing it again.
    take: RefCell<Option<Arc<platform::Take>>>,
    /// The mic on the headphones, while on.
    listening: RefCell<Option<listen::Listening>>,
    /// Tap mode for the dictation key, read by the key thread.
    key_toggle: Arc<AtomicBool>,
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
        let (fs, guid, detail) = match platform::filter_status() {
            FilterStatus::Unsupported => (FilterState::Unsupported, None, String::new()),
            FilterStatus::NotInstalled => (FilterState::NotInstalled, None, String::new()),
            FilterStatus::NotAttached => (FilterState::NotAttached, None, String::new()),
            FilterStatus::Waiting(g) => (FilterState::Checking, Some(g), String::new()),
            FilterStatus::Running(g, d) => (FilterState::Running, Some(g), d),
            FilterStatus::Passthrough(g, d) => (FilterState::Passthrough, Some(g), d),
            FilterStatus::Detached(g) => (FilterState::Detached, Some(g), String::new()),
        };
        let mic = guid.as_deref().map(name).unwrap_or_default();
        // The filtered microphone is unplugged: offer the default one.
        let move_to = match &guid {
            Some(g) if !self.microphones.borrow().iter().any(|m| &m.guid == g) => self
                .microphones
                .borrow()
                .first()
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        state.set_move_to(move_to.into());
        // On CPUs without AVX2 the filter runs ONNX Runtime, at several times
        // the CPU: switch to the small model once, then leave it to the user.
        let old_cpu = fs == FilterState::Running && detail.starts_with("onnx");
        state.set_old_cpu(old_cpu);
        if old_cpu && !self.config.borrow().lean_suggested {
            {
                let mut config = self.config.borrow_mut();
                config.lean_suggested = true;
                config.save();
            }
            if !self.filter.borrow().light_model {
                state.set_model_index(1);
                self.write_filter(|f| f.light_model = true);
                self.model_changed.set(Some(Instant::now()));
            }
        }
        let _ = self.worker.send(Command::Microphone(guid.clone()));
        *self.attached.borrow_mut() = guid;
        if state.get_filter_state() != FilterState::Error || fs != FilterState::NotAttached {
            state.set_filter_state(fs);
        }
        state.set_attached_mic(mic.into());
        state.set_filter_detail(detail.into());
        let filter = *self.filter.borrow();
        // Without the filter both switches show off, whatever is stored.
        let supported = fs != FilterState::Unsupported;
        state.set_filter_enabled(filter.enabled && supported);
        if !supported {
            state.set_voice_enabled(false);
            state.set_auto_level(false);
        }
        tray.set_filter_enabled(filter.enabled && supported);
        tray.set_voice_enabled(filter.voice.enabled && supported);
        tray.set_active(filter.enabled && matches!(fs, FilterState::Running | FilterState::Checking));
    }

    /// Changes the filter settings and stores only the values that changed:
    /// every write wakes the filter, and a slider sends many.
    fn write_filter(&self, change: impl FnOnce(&mut FilterSettings)) {
        let mut filter = self.filter.borrow_mut();
        let before = filter.to_dwords();
        change(&mut filter);
        let changed: Vec<(&str, u32)> = filter
            .to_dwords()
            .into_iter()
            .zip(before)
            .filter(|(now, was)| now != was)
            .map(|(now, _)| now)
            .collect();
        drop(filter);
        if !changed.is_empty() {
            let _ = platform::set_filter_values(&changed);
        }
    }

    fn set_filter(&self, change: impl FnOnce(&mut FilterSettings)) {
        self.write_filter(change);
        self.refresh_filter();
    }

    /// Changes the voice settings and shows the result (the reset button
    /// appears or goes, the window fits its content).
    fn set_voice(&self, change: impl FnOnce(&mut VoiceSettings)) {
        self.write_filter(|f| change(&mut f.voice));
        if let Some(t) = self.tray.upgrade() {
            t.set_voice_enabled(self.filter.borrow().voice.enabled);
        }
        if let Some(w) = self.window.upgrade() {
            show_voice(&w.global::<AppState>(), &self.filter.borrow().voice);
            fit(&self.window);
        }
    }

    /// Starts the meter when the window comes up. The heartbeat is taken as
    /// it is now, so time spent closed does not read as an active mic.
    fn start_meter(self: &Rc<Self>) {
        if self.meter.running() {
            return;
        }
        if let Some(m) = self.monitor.borrow().as_ref() {
            self.heartbeat.set(m.levels().heartbeat);
        }
        let app = Rc::downgrade(self);
        self.meter
            .start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
                if let Some(app) = app.upgrade() {
                    app.tick();
                }
            });
    }

    /// The window went away: no meter, no listening.
    fn window_closed(&self) {
        self.meter.stop();
        self.stop_listening();
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_mic_active(false);
            state.set_level_before(0.0);
            state.set_level_after(0.0);
        }
    }

    /// Every 100 ms: the meter, whether an app uses the microphone, the
    /// probe's progress and whether a model switch is still under way.
    fn tick(&self) {
        while let Ok(event) = self.probe_events.1.try_recv() {
            self.probe_event(event);
        }
        let Some(w) = self.window.upgrade() else {
            return;
        };
        // Minimized, nobody looks either.
        if w.window().is_minimized() {
            return;
        }
        let ticks = self.ticks.get().wrapping_add(1);
        self.ticks.set(ticks);
        if self.monitor.borrow().is_none() && ticks.is_multiple_of(10) {
            *self.monitor.borrow_mut() = platform::Monitor::open();
        }
        let levels = self.monitor.borrow().as_ref().map(|m| m.levels()).unwrap_or_default();
        let active = levels.heartbeat != self.heartbeat.get();
        self.heartbeat.set(levels.heartbeat);
        let state = w.global::<AppState>();
        match ticks % PHRASE_TICKS {
            0 => {
                next_phrase(&state);
                state.set_phrase_visible(true);
            }
            t if t == PHRASE_TICKS - PHRASE_FADE_TICKS => state.set_phrase_visible(false),
            _ => {}
        }
        state.set_mic_active(active);
        let silent = if active && levels.before < SILENCE {
            self.silent_ticks.get().saturating_add(1)
        } else {
            0
        };
        self.silent_ticks.set(silent);
        state.set_mic_silent(silent >= SILENCE_TICKS);
        state.set_level_before(if active { meter(levels.before) } else { 0.0 });
        state.set_level_after(if active { meter(levels.after) } else { 0.0 });
        state.set_level_gain(if levels.gain_db.is_finite() {
            levels.gain_db
        } else {
            0.0
        });

        let switching = match self.model_changed.get() {
            Some(at) if at.elapsed() < MODEL_SWITCH_TIMEOUT => {
                if ticks.is_multiple_of(5) {
                    self.refresh_filter();
                }
                let wanted = if self.filter.borrow().light_model {
                    "light"
                } else {
                    "quality"
                };
                active
                    && state.get_filter_state() == FilterState::Running
                    && !state.get_filter_detail().contains(wanted)
            }
            _ => false,
        };
        state.set_model_switching(switching);
        if !switching {
            self.model_changed.set(None);
        }
    }

    fn set_model(&self, light: bool) {
        self.set_filter(|f| f.light_model = light);
        self.model_changed.set(Some(Instant::now()));
        if let Some(w) = self.window.upgrade() {
            next_phrase(&w.global::<AppState>());
        }
    }

    fn probe_event(&self, event: probe::Event) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        match event {
            probe::Event::Recording => state.set_probe_state(ProbeState::Recording),
            probe::Event::Before => state.set_probe_state(ProbeState::Before),
            probe::Event::After => state.set_probe_state(ProbeState::After),
            probe::Event::Done(take) => {
                state.set_probe_state(ProbeState::Idle);
                state.set_has_probe(true);
                *self.take.borrow_mut() = Some(take);
            }
            probe::Event::Failed(e) => {
                state.set_probe_state(ProbeState::Idle);
                state.set_probe_error(format!("{} {e}", text(self.language(), "probe failed")).into());
            }
        }
        fit(&self.window);
    }

    fn probe_notify(&self) -> impl Fn(probe::Event) + Send + 'static {
        let events = self.probe_events.0.clone();
        move |event| {
            let _ = events.send(event);
        }
    }

    fn probe_record(&self) {
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_probe_error("".into());
            state.set_probe_state(ProbeState::Recording);
            next_phrase(&state);
        }
        probe::record(self.attached.borrow().clone(), self.probe_notify());
    }

    /// Forgets the take: Before and After go away until the next test.
    fn probe_reset(&self) {
        self.take.borrow_mut().take();
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_has_probe(false);
            state.set_probe_error("".into());
        }
        fit(&self.window);
    }

    fn probe_play(&self, after: bool) {
        let Some(take) = self.take.borrow().clone() else {
            return;
        };
        let half = if after { probe::Half::After } else { probe::Half::Before };
        probe::replay(take, half, self.probe_notify());
    }

    /// Asks GitHub for a newer kyrr on a thread of its own. The button
    /// (`manual`) shows every outcome; the daily check only speaks up when
    /// there is a new version, and never interrupts an update under way.
    fn check_updates(&self, manual: bool) {
        let dismissed = {
            let mut config = self.config.borrow_mut();
            config.update_checked = now();
            config.save();
            config.update_dismissed.clone()
        };
        if manual {
            if let Some(w) = self.window.upgrade() {
                w.global::<AppState>().set_update_state(UpdateState::Checking);
            }
        }
        let (window, tray) = (self.window.clone(), self.tray.clone());
        std::thread::spawn(move || {
            let outcome = update::check();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(w) = window.upgrade() else { return };
                let state = w.global::<AppState>();
                if matches!(
                    state.get_update_state(),
                    UpdateState::Downloading | UpdateState::Installing
                ) {
                    return;
                }
                match outcome {
                    update::Outcome::UpToDate if manual => state.set_update_state(UpdateState::Current),
                    update::Outcome::Failed if manual => state.set_update_state(UpdateState::Failed),
                    update::Outcome::UpToDate | update::Outcome::Failed => {}
                    update::Outcome::Available {
                        version,
                        url,
                        installer,
                    } => {
                        state.set_update_dismissed(version == dismissed);
                        state.set_update_version(version.as_str().into());
                        state.set_update_url(url.into());
                        state.set_update_installable(installer.is_some());
                        *pending_installer().lock().unwrap_or_else(|e| e.into_inner()) = installer;
                        state.set_update_state(UpdateState::Available);
                        if let Some(t) = tray.upgrade() {
                            t.set_update_version(version.into());
                        }
                    }
                }
            });
        });
    }

    /// The daily check: when it is on and a day has passed since kyrr last
    /// asked.
    fn check_updates_daily(&self) {
        let due = {
            let config = self.config.borrow();
            config.update_check && now().saturating_sub(config.update_checked) >= UPDATE_INTERVAL.as_secs()
        };
        if due {
            self.check_updates(false);
        }
    }

    /// Listening on or off. The window's state decides, not the handle: a
    /// start that failed leaves a dead one behind.
    fn toggle_listening(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let was_on = state.get_listening();
        self.stop_listening();
        if was_on {
            return;
        }
        state.set_probe_error("".into());
        state.set_listening(true);
        let weak = self.window.clone();
        let prefix = text(self.language(), "listen failed");
        let listening = listen::start(self.attached.borrow().clone(), move |e| {
            let _ = weak.upgrade_in_event_loop(move |w| {
                let state = w.global::<AppState>();
                state.set_listening(false);
                state.set_probe_error(format!("{prefix} {e}").into());
            });
        });
        *self.listening.borrow_mut() = Some(listening);
    }

    fn stop_listening(&self) {
        self.listening.borrow_mut().take();
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_listening(false);
        }
    }

    /// The dictation key and mode, from the settings: to the hook, the
    /// window and the tray.
    fn apply_key(&self) {
        let (index, toggle) = {
            let config = self.config.borrow();
            let index = config::DICTATION_KEYS
                .iter()
                .position(|&k| k == config.dictation_key)
                .unwrap_or(1);
            (index, config.dictation_toggle)
        };
        platform::set_hotkey_key(config::DICTATION_KEYS[index]);
        self.key_toggle.store(toggle, Ordering::Relaxed);
        let name = match index {
            0 => "F8".to_string(),
            1 => "F9".to_string(),
            2 => "Pause".to_string(),
            _ => text(self.language(), "right ctrl"),
        };
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_key_index(index as i32);
            state.set_key_toggle(toggle);
            state.set_key_name(name.as_str().into());
        }
        if let Some(t) = self.tray.upgrade() {
            t.set_key_name(name.into());
        }
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
        self.apply_key();
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
        self.move_filter(mic.guid);
    }

    /// Puts the filter on microphone `guid` (or back on it, after Windows
    /// took it off), with one permission prompt.
    fn move_filter(self: &Rc<Self>, guid: String) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.global::<AppState>().set_filter_state(FilterState::SettingUp);
        let weak = self.window.clone();
        let language = self.language();
        std::thread::spawn(move || {
            let result = platform::attach(&guid);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(w) = weak.upgrade() else {
                    return;
                };
                let state = w.global::<AppState>();
                match result {
                    Ok(()) => {
                        state.set_filter_state(FilterState::Checking);
                        state.set_move_to("".into());
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

/// How often kyrr asks GitHub for a newer version, when that is on.
const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Seconds since 1970.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The installer the last update check found, for "Update now".
fn pending_installer() -> &'static std::sync::Mutex<Option<update::Installer>> {
    static PENDING: std::sync::Mutex<Option<update::Installer>> = std::sync::Mutex::new(None);
    &PENDING
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
    // An installer downloaded for the last update is done with.
    let _ = std::fs::remove_dir_all(update::folder());
    let window = MainWindow::new()?;
    let tray = Tray::new()?;
    let _ = slint::select_bundled_translation(language.code());

    // Dictation worker, reporting back on the UI thread. Whether it records
    // is also kept here, for the tap-to-toggle key.
    let recording = Arc::new(AtomicBool::new(false));
    let key_toggle = Arc::new(AtomicBool::new(false));
    let weak = window.as_weak();
    let weak_tray = tray.as_weak();
    let recording_flag = Arc::clone(&recording);
    let notify: dictation::Notify = Arc::new(move |event| {
        if let Event::State(s) = &event {
            recording_flag.store(*s == State::Recording, Ordering::Relaxed);
        }
        let (weak, weak_tray) = (weak.clone(), weak_tray.clone());
        let _ = slint::invoke_from_event_loop(move || {
            let (Some(w), Some(t)) = (weak.upgrade(), weak_tray.upgrade()) else {
                return;
            };
            let state = w.global::<AppState>();
            match event {
                Event::State(s) => {
                    state.set_dictation_state(dictation_state(s));
                    // A fresh phrase for every F9 press and for the typing after it.
                    if matches!(s, State::Recording | State::Transcribing) {
                        next_phrase(&state);
                    }
                    t.set_recording(s == State::Recording);
                    t.set_dictation_ready(matches!(s, State::Ready | State::Recording | State::Transcribing));
                    platform::set_hotkey_active(matches!(s, State::Ready | State::Recording | State::Transcribing));
                }
                Event::Progress(p) => state.set_download_progress(p),
                Event::Typed(text) => state.set_last_text(text.into()),
                Event::Message(m) => state.set_dictation_message(m.unwrap_or_default().into()),
            }
        });
    });
    let worker = dictation::spawn(
        config::models_dir(),
        language,
        config.dictation,
        notify,
        platform::paste_text,
        platform::copy_to_clipboard,
    );

    // Hold mode: press starts, release stops. Tap mode: each press toggles.
    if let Some(keys) = platform::start_hotkey() {
        let worker = worker.clone();
        let (recording, key_toggle) = (Arc::clone(&recording), Arc::clone(&key_toggle));
        std::thread::spawn(move || {
            while let Ok(key) = keys.recv() {
                let toggle = key_toggle.load(Ordering::Relaxed);
                let cmd = match (key, toggle) {
                    (platform::KeyEvent::Pressed, false) => Command::KeyDown,
                    (platform::KeyEvent::Released, false) => Command::KeyUp,
                    (platform::KeyEvent::Pressed, true) if recording.load(Ordering::Relaxed) => Command::KeyUp,
                    (platform::KeyEvent::Pressed, true) => Command::KeyDown,
                    (platform::KeyEvent::Released, true) => continue,
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
        attached: RefCell::new(None),
        monitor: RefCell::new(None),
        meter: slint::Timer::default(),
        heartbeat: Cell::new(0),
        ticks: Cell::new(0),
        silent_ticks: Cell::new(0),
        model_changed: Cell::new(None),
        probe_events: mpsc::channel(),
        take: RefCell::new(None),
        listening: RefCell::new(None),
        key_toggle,
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
    state.set_model_index(filter.light_model as i32);
    state.set_autostart(platform::autostart_enabled());
    state.set_update_auto(app.config.borrow().update_check);
    state.set_advanced(app.config.borrow().advanced_settings);
    app.apply_key();
    show_voice(&state, &filter.voice);
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
    state.on_autostart_toggled(platform::set_autostart);
    state.on_open_licenses(platform::open_licenses);
    state.on_check_updates({
        let app = Rc::clone(&app);
        move || app.check_updates(true)
    });
    state.on_update_auto_toggled({
        let app = Rc::clone(&app);
        move |on| {
            let mut config = app.config.borrow_mut();
            config.update_check = on;
            config.save();
        }
    });
    state.on_advanced_toggled({
        let app = Rc::clone(&app);
        move |on| {
            {
                let mut config = app.config.borrow_mut();
                config.advanced_settings = on;
                config.save();
            }
            fit(&app.window);
        }
    });
    state.on_dismiss_update({
        let app = Rc::clone(&app);
        move || {
            let Some(w) = app.window.upgrade() else { return };
            let state = w.global::<AppState>();
            {
                let mut config = app.config.borrow_mut();
                config.update_dismissed = state.get_update_version().into();
                config.save();
            }
            state.set_update_dismissed(true);
            fit(&app.window);
        }
    });
    state.on_install_update({
        let weak = window.as_weak();
        move || {
            let Some(installer) = pending_installer().lock().unwrap_or_else(|e| e.into_inner()).clone() else {
                return;
            };
            let Some(w) = weak.upgrade() else { return };
            let state = w.global::<AppState>();
            state.set_update_progress(0.0);
            state.set_update_dismissed(false);
            state.set_update_state(UpdateState::Downloading);
            let weak = weak.clone();
            std::thread::spawn(move || {
                let progress_weak = weak.clone();
                let result = update::download(&installer, &mut |p| {
                    let _ = progress_weak.upgrade_in_event_loop(move |w| w.global::<AppState>().set_update_progress(p));
                });
                let set = |s: UpdateState| {
                    let _ = weak.upgrade_in_event_loop(move |w| w.global::<AppState>().set_update_state(s));
                };
                let path = match result {
                    Ok(path) => path,
                    Err(e) => {
                        eprintln!("kyrr: update: {e}");
                        return set(UpdateState::DownloadFailed);
                    }
                };
                set(UpdateState::Installing);
                // The installer ends this kyrr and starts the new one. Back
                // here means nothing was installed: Windows' permission was
                // declined, so the offer stands again.
                match platform::run_installer(&path) {
                    Ok(()) | Err(SetupError::Declined) => set(UpdateState::Available),
                    Err(SetupError::Failed(e)) => {
                        eprintln!("kyrr: update: {e}");
                        set(UpdateState::DownloadFailed);
                    }
                }
            });
        }
    });
    state.on_open_update({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                platform::open_url(&w.global::<AppState>().get_update_url());
            }
        }
    });
    state.on_dictation_retry({
        let worker = worker.clone();
        move || {
            let _ = worker.send(Command::Retry);
        }
    });
    state.on_repair({
        let app = Rc::clone(&app);
        move || {
            if let Some(guid) = app.attached.borrow().clone() {
                app.move_filter(guid);
            }
        }
    });
    state.on_move_to_default({
        let app = Rc::clone(&app);
        move || {
            let first = app.microphones.borrow().first().map(|m| m.guid.clone());
            if let Some(guid) = first {
                app.move_filter(guid);
            }
        }
    });
    state.on_key_changed({
        let app = Rc::clone(&app);
        move |i| {
            {
                let mut config = app.config.borrow_mut();
                config.dictation_key = config::DICTATION_KEYS[(i.max(0) as usize).min(3)];
                config.save();
            }
            app.apply_key();
        }
    });
    state.on_key_toggle_changed({
        let app = Rc::clone(&app);
        move |on| {
            {
                let mut config = app.config.borrow_mut();
                config.dictation_toggle = on;
                config.save();
            }
            app.apply_key();
        }
    });
    state.on_skip_setup({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.set_setup_step(1);
                fit(&weak);
            }
        }
    });
    state.on_page_changed({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.invoke_reset_scroll();
            }
            fit(&weak);
        }
    });
    state.on_model_changed({
        let app = Rc::clone(&app);
        move |i| app.set_model(i == 1)
    });
    state.on_probe_record({
        let app = Rc::clone(&app);
        move || app.probe_record()
    });
    state.on_probe_play({
        let app = Rc::clone(&app);
        move |after| app.probe_play(after)
    });
    state.on_probe_reset({
        let app = Rc::clone(&app);
        move || app.probe_reset()
    });
    state.on_listen_toggled({
        let app = Rc::clone(&app);
        move || app.toggle_listening()
    });
    window.on_content_height_changed({
        let weak = window.as_weak();
        move || grow(&weak)
    });
    // Closing only hides the window; listening and the meter end with it.
    // The button in the window does the same; quitting is in the tray menu.
    window.window().on_close_requested({
        let app = Rc::clone(&app);
        move || {
            app.window_closed();
            slint::CloseRequestResponse::HideWindow
        }
    });
    state.on_hide({
        let app = Rc::clone(&app);
        move || {
            app.window_closed();
            if let Some(w) = app.window.upgrade() {
                let _ = w.hide();
            }
        }
    });
    window.on_shown({
        let app = Rc::clone(&app);
        move || app.start_meter()
    });
    state.on_voice_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_voice(|v| v.enabled = on)
    });
    state.on_look_changed({
        let app = Rc::clone(&app);
        move |i| app.set_voice(|v| v.look = Look::from_index(i.max(0) as u32).unwrap_or(Look::Clear))
    });
    state.on_band_changed({
        let app = Rc::clone(&app);
        move |band, db| {
            let tenths = (db * 10.0).round() as i32;
            app.set_voice(|v| match band {
                0 => v.body = tenths,
                1 => v.presence = tenths,
                _ => v.air = tenths,
            })
        }
    });
    state.on_low_cut_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_voice(|v| v.low_cut = on)
    });
    state.on_pops_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_voice(|v| v.pops = on)
    });
    state.on_auto_level_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_voice(|v| v.auto_level = on)
    });
    state.on_deesser_changed({
        let app = Rc::clone(&app);
        move |i| app.set_voice(|v| v.deesser = amount(i))
    });
    state.on_compressor_changed({
        let app = Rc::clone(&app);
        move |i| app.set_voice(|v| v.compressor = amount(i))
    });
    state.on_voice_reset({
        let app = Rc::clone(&app);
        move || app.set_voice(|v| *v = voice_defaults(v))
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
                w.global::<AppState>().set_autostart(platform::autostart_enabled());
                w.set_setup_mode(false);
                w.global::<AppState>().set_page(0);
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
                w.global::<AppState>().set_page(0);
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
    tray.on_quit(quit);
    tray.on_install_update({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                show(&w);
                w.global::<AppState>().invoke_install_update();
            }
        }
    });
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
    tray.on_toggle_voice({
        let app = Rc::clone(&app);
        move || {
            let on = !app.filter.borrow().voice.enabled;
            app.set_voice(|v| v.enabled = on);
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
    // The daily look for a new version: a minute after the start (not in
    // the busy moments of signing in), then whenever a day has passed.
    let update_timer = slint::Timer::default();
    update_timer.start(slint::TimerMode::Repeated, Duration::from_secs(30 * 60), {
        let app = Rc::clone(&app);
        move || app.check_updates_daily()
    });
    slint::Timer::single_shot(Duration::from_secs(60), {
        let app = Rc::downgrade(&app);
        move || {
            if let Some(app) = app.upgrade() {
                app.check_updates_daily();
            }
        }
    });
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
    // No filter on this PC: the setup starts with dictation.
    if state.get_filter_state() == FilterState::Unsupported {
        window.set_setup_step(1);
    }
    if !setup_done || !background {
        show(&window);
    }
    tray.show()?;
    slint::run_event_loop_until_quit()
}
