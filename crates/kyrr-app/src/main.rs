//! kyrr: noise removal, voice polish and push-to-talk dictation in the tray.
//!
//! `kyrr.exe --setup <command>` runs an elevated setup step (see
//! `kyrr_windows::run_setup_command`); `--background` starts without
//! showing the window. On Linux `kyrr --dictate` starts or ends dictation
//! in the running kyrr, for desktops where kyrr cannot catch a key itself.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod dictation;
#[cfg(target_os = "linux")]
mod engine;
mod follow;
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
use platform::{Desktop, FilterStatus, KeyBinding, KeyNote, Microphone, PipewireInstall, SetupError};

slint::include_modules!();

/// Maximum attenuation for the strength choices gentle, medium, strong.
const STRENGTHS: [u32; 3] = [24, 48, 100];

fn main() {
    #[cfg(not(windows))]
    kyrr_linux::pipewire_setup::forget_stand_in();
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
    let show_requests = match kyrr_linux::instance::claim(&args) {
        kyrr_linux::instance::Instance::First { show_requests } => Some(show_requests),
        kyrr_linux::instance::Instance::Other => return,
    };

    // A dictation key pressed while kyrr is not running starts it quietly.
    let background = args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--background" | "--dictate" | "--dictate-start" | "--dictate-stop"
        )
    });
    if let Err(e) = run(background, show_requests) {
        eprintln!("kyrr: {e}");
    }
    platform::shutdown();
    #[cfg(target_os = "linux")]
    kyrr_linux::signals::exit_now(0);
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

/// What to do so the dictation key reaches kyrr, in the user's language,
/// and what the desktop will ask on the first dictation.
fn key_note(language: Language) -> String {
    let german = matches!(language, Language::German);
    let note = key_note_only(platform::key_note(), german);
    if !platform::typing_needs_permission() {
        return note;
    }
    let typing = if german {
        "Beim ersten Diktat fragt dein Desktop nach „Remote Desktop“ (oder „Entfernter Bildschirm“): „Fernzugriff zulassen“ einschalten, dann „Freigeben“. kyrr tippt damit nur deinen Text."
    } else {
        "On the first dictation your desktop asks about “Remote Desktop”: switch on “Allow Remote Interaction”, then “Share”. kyrr only uses it to type your text."
    };
    if note.is_empty() {
        typing.into()
    } else {
        format!("{note}\n\n{typing}")
    }
}

fn key_note_only(note: KeyNote, german: bool) -> String {
    match note {
        KeyNote::None => String::new(),
        KeyNote::Desktop(Desktop::Gnome) if german => {
            "Die Taste verwaltet GNOME, die Auswahl hier gilt dort nicht. Ändern: Einstellungen › Apps › kyrr.".into()
        }
        KeyNote::Desktop(Desktop::Gnome) => {
            "GNOME manages the key, so there is no choice here. To change it: Settings › Apps › kyrr.".into()
        }
        KeyNote::Desktop(Desktop::Kde) if german => {
            "Die Taste verwaltet KDE, die Auswahl hier gilt dort nicht. Ändern: Systemeinstellungen › Tastatur › Kurzbefehle › kyrr."
                .into()
        }
        KeyNote::Desktop(Desktop::Kde) => {
            "KDE manages the key, so there is no choice here. To change it: System Settings › Keyboard › Shortcuts › kyrr."
                .into()
        }
        KeyNote::Desktop(_) if german => {
            "Die Taste legt dein Desktop fest: Er fragt einmal nach, ändern geht in seinen Tastatur-Einstellungen."
                .into()
        }
        KeyNote::Desktop(_) => {
            "Your desktop decides the key: it asks once, and its keyboard settings can change it.".into()
        }
        KeyNote::Unbound if german => {
            "Dein Desktop vergibt die Taste und fragt dafür einmal nach. Diese Frage wurde abgebrochen, also kommt die Taste bei kyrr nicht an. „Taste einrichten“ fragt nochmal."
                .into()
        }
        KeyNote::Unbound => {
            "Your desktop hands out the key and asks once for it. That question was cancelled, so the key does not reach kyrr. “Set up key” asks again."
                .into()
        }
        KeyNote::Lines { desktop, file, lines } if german => format!(
            "{desktop} lässt kyrr nicht an seine Tastenbelegung. Mit diesen Zeilen in der Datei {file} klappt es:\n{lines}"
        ),
        KeyNote::Lines { desktop, file, lines } => {
            format!("{desktop} will not let kyrr near its key bindings. These lines in {file} do the trick:\n{lines}")
        }
        KeyNote::Taken { file, key, line, bind } if german => format!(
            "{key} hat schon einen anderen Job, in {file}, Zeile {line}:\n{bind}\nSuch dir hier eine andere Taste aus, oder nimm die Zeile dort raus."
        ),
        KeyNote::Taken { file, key, line, bind } => format!(
            "{key} already has another job, in {file}, line {line}:\n{bind}\nPick another key here, or take that line out there."
        ),
        KeyNote::PressOnly { desktop, key } if german => format!(
            "{desktop} kann die Taste nur drücken, nicht halten: {key} einmal drücken startet das Diktat, nochmal drücken beendet es."
        ),
        KeyNote::PressOnly { desktop, key } => format!(
            "{desktop} only knows pressing, not holding: press {key} once to start dictating, once more to finish."
        ),
        KeyNote::GnomeTaken { key, name } if name.is_empty() && german => format!(
            "{key} hat in GNOMEs eigenen Tastenkürzeln schon einen Job. Such dir hier eine andere Taste aus, oder gib {key} unter Einstellungen › Tastatur › Tastenkürzel frei."
        ),
        KeyNote::GnomeTaken { key, name } if name.is_empty() => format!(
            "{key} already has a job in GNOME's own shortcuts. Pick another key here, or free {key} under Settings › Keyboard › Keyboard Shortcuts."
        ),
        KeyNote::GnomeTaken { key, name } if german => format!(
            "{key} hat schon einen Job, im GNOME-Tastenkürzel „{name}“. Such dir hier eine andere Taste aus, oder gib {key} unter Einstellungen › Tastatur › Tastenkürzel frei."
        ),
        KeyNote::GnomeTaken { key, name } => format!(
            "{key} already has a job, in GNOME's shortcut “{name}”. Pick another key here, or free {key} under Settings › Keyboard › Keyboard Shortcuts."
        ),
        KeyNote::HyprlandCommands { file, lines } if german => format!(
            "Hyprlands Tastenkürzel-Portal antwortet nicht. Mit diesen Zeilen in der Datei {file} diktierst du wie gewohnt mit gehaltener Taste:\n{lines}"
        ),
        KeyNote::HyprlandCommands { file, lines } => format!(
            "Hyprland's shortcut portal does not answer. Add these lines to {file} and holding the key dictates as usual:\n{lines}"
        ),
        KeyNote::Command(command) if german => format!(
            "Dieser Desktop reicht keine Taste an kyrr weiter. Leg in seinen Tastatur-Einstellungen diesen Befehl auf eine Taste, jeder Druck startet oder beendet das Diktat:\n{command}"
        ),
        KeyNote::Command(command) => format!(
            "This desktop passes no key to kyrr. Put this command on a key in its keyboard settings, each press starts or ends dictation:\n{command}"
        ),
    }
}

/// Where setting up PipeWire stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipewireStep {
    Offer,
    Installing,
    AfterSignIn,
    Declined,
    Failed,
}

/// What the window says under "PipeWire is not running": what the button
/// does, how it went, or what to do by hand.
fn pipewire_note(step: PipewireStep, plan: Option<(bool, String)>, german: bool) -> String {
    let Some((can_ask, command)) = plan else {
        return if german {
            "Hol dir PipeWire aus der Softwareverwaltung deines Systems (Paket „pipewire-pulse“), dann einmal ab- und wieder anmelden.".into()
        } else {
            "Get PipeWire from your system's software manager (package “pipewire-pulse”), then sign out and back in once.".into()
        };
    };
    match step {
        _ if !can_ask && german => format!("Im Terminal geht's so, danach einmal ab- und wieder anmelden:\n{command}"),
        _ if !can_ask => format!("In a terminal it goes like this, then sign out and back in once:\n{command}"),
        PipewireStep::Offer if german => {
            "Ein Klick, dein System fragt nach deinem Passwort, kyrr holt PipeWire und startet neu.".into()
        }
        PipewireStep::Offer => {
            "One click, your system asks for your password, kyrr fetches PipeWire and restarts.".into()
        }
        PipewireStep::Installing if german => "Hole PipeWire… dauert so lang wie der Download.".into(),
        PipewireStep::Installing => "Fetching PipeWire… takes as long as the download.".into(),
        PipewireStep::AfterSignIn if german => {
            "PipeWire ist drauf. Einmal ab- und wieder anmelden, dann läuft's.".into()
        }
        PipewireStep::AfterSignIn => "PipeWire is in. Sign out and back in once, then it runs.".into(),
        PipewireStep::Declined if german => "Ohne Passwort geht's nicht. Der Knopf wartet geduldig.".into(),
        PipewireStep::Declined => "No password, no PipeWire. The button waits patiently.".into(),
        PipewireStep::Failed if german => format!("Das hat nicht geklappt. Im Terminal geht's so:\n{command}"),
        PipewireStep::Failed => format!("That did not work. In a terminal it goes like this:\n{command}"),
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
/// for the whole two-column settings page in its advanced view (about 1200).
const MAX_HEIGHT: f32 = 1250.0;
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
    platform::work_area(w.window())
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
    #[cfg(target_os = "linux")]
    {
        w.set_fit_height(height);
        // The new minimum reaches the compositor with the next frame; until
        // then a smaller size is held at the old minimum (GNOME), so ask
        // once more after it.
        let weak = w.as_weak();
        slint::Timer::single_shot(Duration::from_millis(50), move || {
            if let Some(w) = weak.upgrade() {
                w.window().set_size(LogicalSize::new(width, height));
            }
        });
    }
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
/// status went to two lines). Never shrinks it on its own (except the main
/// page, whose height is fixed to its content), so it does not jump with
/// every phrase; the next fit tidies up.
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
        // The main page has a fixed height that follows its content both
        // ways (Slint applies it by itself); resizing still moves the window
        // clear of the taskbar.
        let fixed = !w.get_setup_mode() && w.global::<AppState>().get_page() == 0;
        if wanted > height + 0.5 || (fixed && (wanted - height).abs() > 0.5) {
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

/// The small red notes at the bottom of the main page, numbered as the
/// .slint side reads them (`AppState.alert`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Alert {
    MicQuiet = 1,
    MicLoud = 2,
    Listening = 3,
    /// Windows has the microphone muted; says so alone, it outweighs the rest.
    Muted = 4,
    /// Windows lists no microphone at all; kyrr keeps looking.
    NoMic = 6,
    /// A microphone just showed up (the name is `AppState.connected-name`).
    Connected = 7,
}

/// How long the "connected" note stays up.
const CONNECTED_NOTE: Duration = Duration::from_secs(10);

/// The automatic volume's limits (see kyrr-voice), less a little slack for
/// the gain stopping just short of them.
const GAIN_AT_MAX_DB: f32 = 23.5;
const GAIN_AT_MIN_DB: f32 = -11.5;

/// The notes that apply right now. The window shows them one after the other.
#[allow(clippy::too_many_arguments)]
fn alerts(
    auto_level: bool,
    mic_active: bool,
    gain_db: f32,
    listening: bool,
    muted: bool,
    no_mic: bool,
    connected: bool,
) -> Vec<Alert> {
    if muted {
        return vec![Alert::Muted];
    }
    if no_mic {
        return vec![Alert::NoMic];
    }
    let mut list = Vec::new();
    if connected {
        list.push(Alert::Connected);
    }
    // The gain only means something while an app uses the microphone.
    if auto_level && mic_active {
        if gain_db >= GAIN_AT_MAX_DB {
            list.push(Alert::MicQuiet);
        } else if gain_db <= GAIN_AT_MIN_DB {
            list.push(Alert::MicLoud);
        }
    }
    if listening {
        list.push(Alert::Listening);
    }
    list
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
        // The width first: until the window has one, text is measured as if
        // it wrapped after every word, and the content seems far taller.
        // Wayland compositors may keep the first size they saw.
        let width = window.get_page_width();
        window.window().set_size(LogicalSize::new(width, FALLBACK_HEIGHT));
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
    /// Which of the red notes is up; moves on with the phrases.
    alert_turn: Cell<u32>,
    /// Ticks in a row the microphone was in use but silent.
    silent_ticks: Cell<u32>,
    model_changed: Cell<Option<Instant>>,
    /// Events from the probe thread, picked up by `tick`.
    probe_events: (mpsc::Sender<probe::Event>, mpsc::Receiver<probe::Event>),
    /// The last before/after take, for playing it again.
    take: RefCell<Option<Arc<platform::Take>>>,
    /// The mic on the headphones, while on.
    listening: RefCell<Option<listen::Listening>>,
    /// The "What's new" window, while it is open.
    news_window: RefCell<Option<NewsWindow>>,
    /// Tap mode for the dictation key, read by the key thread.
    key_toggle: Arc<AtomicBool>,
    /// Key presses counted when the key was set up, to see it pressed since.
    presses_at_bound: Cell<Option<u32>>,
    microphones: RefCell<Vec<Microphone>>,
    /// Where setting up PipeWire stands (Linux without it).
    pipewire: Cell<PipewireStep>,
    /// The microphone list has been read once (before that, "none" means nothing).
    mics_seen: Cell<bool>,
    /// A microphone that just showed up, and when: shown for a few seconds.
    connected: RefCell<Option<(String, Instant)>>,
    /// Moves the filter along when the default microphone changes.
    follower: RefCell<follow::Follower>,
    worker: mpsc::Sender<Command>,
    window: Weak<MainWindow>,
}

thread_local! {
    /// The tray icon. It is replaced when a panel to host it turns up after
    /// kyrr started (see [`retry_tray`]), so it is looked up, not kept.
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn tray() -> Option<Tray> {
    TRAY.with_borrow(|t| t.as_ref().map(Tray::clone_strong))
}

impl App {
    fn language(&self) -> Language {
        self.config.borrow().language()
    }

    /// Reads the microphones Windows lists now. The window's list only changes
    /// when the microphones did, so a pick in it is not lost every few seconds.
    fn refresh_microphones(&self) {
        let mics = platform::microphones();
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let first = !self.mics_seen.replace(true);
        if *self.microphones.borrow() == mics {
            return;
        }
        // A microphone that was not there before: name it for a few seconds
        // (not at the start, when everything is new).
        if !first {
            let known = self.microphones.borrow();
            if let Some(m) = mics.iter().find(|m| !known.iter().any(|k| k.guid == m.guid)) {
                *self.connected.borrow_mut() = Some((m.name.clone(), Instant::now()));
            }
        }
        let names: Vec<SharedString> = mics.iter().map(|m| m.name.as_str().into()).collect();
        let state = w.global::<AppState>();
        state.set_microphones(ModelRc::new(VecModel::from(names)));
        // The pick pointed into the old list.
        state.set_mic_index(0);
        *self.microphones.borrow_mut() = mics;
    }

    /// Writes the user's pick to the config when it changed, so it holds
    /// after a restart.
    fn keep_pin(&self) {
        let pinned = self.follower.borrow().pinned().unwrap_or_default().to_string();
        let mut config = self.config.borrow_mut();
        if config.pinned_default != pinned {
            config.pinned_default = pinned;
            config.save();
        }
    }

    /// The microphone the filter is on, else the default one.
    fn working_microphone(&self) -> Option<Microphone> {
        let attached = self.attached.borrow().clone();
        let mics = self.microphones.borrow();
        attached
            .and_then(|g| mics.iter().find(|m| m.guid.eq_ignore_ascii_case(&g)).cloned())
            .or_else(|| mics.iter().find(|m| m.is_default).cloned())
    }

    /// Whether Windows has the microphone muted: said on the main page and
    /// to the dictation, instead of nothing happening.
    fn check_mute(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let muted = self
            .working_microphone()
            .and_then(|m| platform::microphone_muted(&m.id))
            .unwrap_or(false);
        let state = w.global::<AppState>();
        if state.get_mic_muted() != muted {
            state.set_mic_muted(muted);
        }
        let _ = self.worker.send(Command::Muted(muted));
    }

    /// The microphone Windows uses by default.
    fn default_microphone(&self) -> Option<Microphone> {
        self.microphones.borrow().iter().find(|m| m.is_default).cloned()
    }

    /// Keeps the filter on the microphone Windows uses: plugging another one
    /// in or out, or a microphone on another USB port (a new id), moves it.
    /// Windows asks for permission once per change; when that is declined or
    /// fails, a button on the main page offers it instead.
    fn follow_default(self: &Rc<Self>) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let fs = state.get_filter_state();
        let attached = self.attached.borrow().clone();
        let default = self.default_microphone();
        let present = attached
            .as_deref()
            .is_some_and(|g| self.microphones.borrow().iter().any(|m| m.guid.eq_ignore_ascii_case(g)));
        let busy = fs == FilterState::SettingUp
            || !matches!(
                fs,
                FilterState::Running
                    | FilterState::Checking
                    | FilterState::Passthrough
                    | FilterState::Detached
                    | FilterState::NotAttached
                    | FilterState::Error
            )
            || w.get_setup_mode()
            || !self.config.borrow().setup_done
            // Whoever switched the noise removal off gets no prompts for it.
            || !self.filter.borrow().enabled;
        let outcome = {
            let mics = self.microphones.borrow();
            self.follower.borrow_mut().tick(&follow::Input {
                attached: attached.as_deref(),
                attached_present: present,
                detached: fs == FilterState::Detached,
                default: default.as_ref().map(|m| m.guid.as_str()),
                busy,
                listed: mics.iter().map(|m| m.guid.as_str()).collect(),
            })
        };
        self.keep_pin();
        let (offer, unplugged) = match &outcome {
            follow::Outcome::Offer { to, unplugged } => (
                self.microphones
                    .borrow()
                    .iter()
                    .find(|m| m.guid.eq_ignore_ascii_case(to))
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
                *unplugged,
            ),
            _ => (String::new(), false),
        };
        state.set_move_to(offer.into());
        state.set_move_unplugged(unplugged);
        if let follow::Outcome::Move(guid) = outcome {
            // The window says what is going on while Windows asks; Linux
            // does not ask, and the move takes a moment only.
            if cfg!(windows) {
                show(&w);
            }
            self.move_filter(guid);
        }
    }

    fn refresh_filter(&self) {
        let (Some(w), Some(tray)) = (self.window.upgrade(), tray()) else {
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
        self.refresh_pipewire(!supported && platform::pipewire_missing());
    }

    /// The button and words that set PipeWire up where it is missing.
    fn refresh_pipewire(&self, missing: bool) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let german = self.language() == Language::German;
        let plan = platform::pipewire_plan();
        let step = self.pipewire.get();
        let button = missing && matches!(plan, Some((true, _))) && step != PipewireStep::Installing;
        let note = if missing {
            pipewire_note(step, plan, german)
        } else {
            String::new()
        };
        state.set_pipewire_button(button);
        if state.get_pipewire_note() != note.as_str() {
            state.set_pipewire_note(note.into());
        }
    }

    fn install_pipewire(self: &Rc<Self>) {
        self.pipewire.set(PipewireStep::Installing);
        self.refresh_pipewire(true);
        let weak = self.window.clone();
        std::thread::spawn(move || {
            let result = platform::install_pipewire();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(w) = weak.upgrade() else {
                    return;
                };
                let step = match result {
                    // Its sound runs now: kyrr starts again on it.
                    PipewireInstall::Running => {
                        platform::restart_after_quit();
                        w.global::<AppState>().invoke_quit_app();
                        return;
                    }
                    PipewireInstall::AfterSignIn => PipewireStep::AfterSignIn,
                    PipewireInstall::Declined => PipewireStep::Declined,
                    PipewireInstall::Failed => PipewireStep::Failed,
                };
                w.global::<AppState>().invoke_pipewire_step(step as i32);
            });
        });
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
        if let Some(t) = tray() {
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
    /// Minimizes the window. The meter keeps its timer, which idles while
    /// the window is minimized: nothing starts it again when the taskbar
    /// brings the window back.
    fn minimize(&self) {
        self.stop_listening();
        if let Some(w) = self.window.upgrade() {
            w.window().set_minimized(true);
        }
    }

    fn window_closed(&self) {
        self.meter.stop();
        self.stop_listening();
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_mic_active(false);
            state.set_alert(0);
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
                self.alert_turn.set(self.alert_turn.get().wrapping_add(1));
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
        let connected_name = self
            .connected
            .borrow()
            .as_ref()
            .map(|(n, _)| n.clone())
            .unwrap_or_default();
        state.set_connected_name(connected_name.into());
        let connected = self
            .connected
            .borrow()
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < CONNECTED_NOTE);
        if !connected {
            *self.connected.borrow_mut() = None;
        }
        let notes = alerts(
            state.get_auto_level(),
            active,
            state.get_level_gain(),
            state.get_listening(),
            state.get_mic_muted(),
            self.microphones.borrow().is_empty() && self.mics_seen.get(),
            connected,
        );
        state.set_alert(if notes.is_empty() {
            0
        } else {
            notes[self.alert_turn.get() as usize % notes.len()] as i32
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
        let window = self.window.clone();
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
                        news,
                    } => {
                        state.set_update_dismissed(version == dismissed);
                        state.set_update_version(version.as_str().into());
                        state.set_update_url(url.into());
                        // The installer is Windows'; elsewhere kyrr is updated
                        // where it came from (Flathub, AUR, a new AppImage).
                        let installer = installer.filter(|_| cfg!(windows));
                        state.set_update_installable(installer.is_some());
                        state.set_update_has_news(news.is_some());
                        *pending_installer().lock().unwrap_or_else(|e| e.into_inner()) = installer;
                        *pending_news().lock().unwrap_or_else(|e| e.into_inner()) = news;
                        state.set_update_state(UpdateState::Available);
                        if let Some(t) = tray() {
                            t.set_update_version(version.into());
                        }
                    }
                }
            });
        });
    }

    /// Opens the window with what the offered version brings, in the app's
    /// language.
    fn show_news(&self) {
        let Some(w) = self.window.upgrade() else { return };
        let Some(news) = pending_news().lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };
        self.open_news(&w.global::<AppState>().get_update_version(), news, 0);
    }

    /// What the running version brought, from the NEWS.md built into the app.
    fn show_current_news(&self) {
        let version = env!("CARGO_PKG_VERSION");
        if let Some(news) = update::bundled_news(version) {
            self.open_news(version, news, 1);
        }
    }

    /// Every version's summary, the newest on top.
    fn show_all_news(&self) {
        if let Some(news) = update::all_news() {
            self.open_news(env!("CARGO_PKG_VERSION"), news, 2);
        }
    }

    /// The news window: `mode` 0 for the version on offer, 1 for the one just
    /// installed, 2 for all of them.
    fn open_news(&self, version: &str, news: update::News, mode: i32) {
        let text = match self.language() {
            Language::German => news.de,
            Language::English => news.en,
        };
        let Ok(win) = NewsWindow::new() else { return };
        win.set_version(version.into());
        win.set_mode(mode);
        win.set_news(text.into());
        let weak = win.as_weak();
        win.on_close(move || {
            if let Some(win) = weak.upgrade() {
                let _ = win.hide();
            }
        });
        if win.show().is_ok() {
            *self.news_window.borrow_mut() = Some(win);
        }
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
        platform::set_key_wanted(self.config.borrow().dictation);
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
        self.refresh_key_note();
        if let Some(t) = tray() {
            t.set_key_name(name.into());
        }
    }

    /// What the window says about the key. It changes after the start: the
    /// desktop's dialog is answered, typing is allowed.
    fn refresh_key_note(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let language = self.language();
        let note = platform::key_note();
        let fixed = matches!(note, KeyNote::Desktop(_) | KeyNote::Unbound);
        let unbound = matches!(note, KeyNote::Unbound);
        let german = language == Language::German;
        let button = match note {
            KeyNote::Unbound if german => "Taste einrichten",
            KeyNote::Unbound => "Set up key",
            KeyNote::Desktop(_) if platform::can_open_key_settings() && german => "Taste ändern",
            KeyNote::Desktop(_) if platform::can_open_key_settings() => "Change key",
            _ => "",
        };
        let text = key_note(language);
        if state.get_key_note() != text.as_str() {
            state.set_key_note(text.into());
        }
        state.set_key_fixed(fixed);
        state.set_key_unbound(unbound);
        if state.get_key_button() != button {
            state.set_key_button(button.into());
        }
        // The setup's walk through the desktop's questions.
        state.set_guide(fixed);
        let key_state = match platform::key_binding() {
            KeyBinding::NotAsked => 0,
            KeyBinding::Asking => 1,
            KeyBinding::Bound => 2,
            KeyBinding::Refused => 3,
        };
        state.set_key_state(key_state);
        let presses = platform::key_presses();
        if key_state == 2 {
            let base = self.presses_at_bound.get().unwrap_or(presses);
            self.presses_at_bound.set(Some(base));
            state.set_key_tried(presses > base);
        } else {
            self.presses_at_bound.set(None);
            state.set_key_tried(false);
        }
        if state.get_typing_state() == 0 && !platform::typing_needs_permission() {
            state.set_typing_state(2);
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
        platform::set_key_wanted(on);
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_dictation_enabled(on);
        }
        if let Some(t) = tray() {
            t.set_dictation_enabled(on);
        }
    }

    fn attach(self: &Rc<Self>, index: usize) {
        let Some(mic) = self.microphones.borrow().get(index).cloned() else {
            return;
        };
        // Chosen on purpose: kyrr leaves it while Windows' default stays as it is.
        let default = self.default_microphone().map(|m| m.guid);
        self.follower.borrow_mut().user_chose(default.as_deref());
        self.keep_pin();
        self.move_filter(mic.guid);
    }

    /// Puts the filter on microphone `guid` (or back on it, after Windows
    /// took it off), with one permission prompt.
    fn move_filter(self: &Rc<Self>, guid: String) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let before = w.global::<AppState>().get_filter_state();
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
                    // Unplugged during the prompt: nothing changed, and the
                    // next look at the microphones says what to do now.
                    Err(SetupError::Gone) => state.set_filter_state(match before {
                        // A second move was running: the next look at the
                        // filter sets the real state.
                        FilterState::SettingUp => FilterState::Checking,
                        before => before,
                    }),
                    Err(e) => {
                        let msg = match e {
                            SetupError::Declined | SetupError::Gone => text(language, "permission"),
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

/// What the offered version brings, for the "What's new" window.
fn pending_news() -> &'static std::sync::Mutex<Option<update::News>> {
    static PENDING: std::sync::Mutex<Option<update::News>> = std::sync::Mutex::new(None);
    &PENDING
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
    // So the desktop matches the window to kyrr's desktop entry and icon.
    #[cfg(target_os = "linux")]
    let _ = slint::set_xdg_app_id("io.github.heast13.kyrr");
    let _ = slint::select_bundled_translation(language.code());

    // Dictation worker, reporting back on the UI thread. Whether it records
    // is also kept here, for the tap-to-toggle key.
    let recording = Arc::new(AtomicBool::new(false));
    let key_toggle = Arc::new(AtomicBool::new(false));
    let weak = window.as_weak();
    let recording_flag = Arc::clone(&recording);
    let notify: dictation::Notify = Arc::new(move |event| {
        if let Event::State(s) = &event {
            recording_flag.store(*s == State::Recording, Ordering::Relaxed);
        }
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let (Some(w), Some(t)) = (weak.upgrade(), tray()) else {
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
    let description = match language {
        Language::German => "kyrr: Diktieren (Taste halten)",
        Language::English => "kyrr: Dictate (hold the key)",
    };
    if let Some(keys) = platform::start_hotkey(description, config.setup_done) {
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
    let pinned = Some(config.pinned_default.clone()).filter(|p| !p.is_empty());
    let app = Rc::new(App {
        config: RefCell::new(config),
        filter: RefCell::new(filter),
        attached: RefCell::new(None),
        monitor: RefCell::new(None),
        meter: slint::Timer::default(),
        heartbeat: Cell::new(0),
        ticks: Cell::new(0),
        alert_turn: Cell::new(0),
        silent_ticks: Cell::new(0),
        model_changed: Cell::new(None),
        probe_events: mpsc::channel(),
        take: RefCell::new(None),
        listening: RefCell::new(None),
        news_window: RefCell::new(None),
        key_toggle,
        presses_at_bound: Cell::new(None),
        microphones: RefCell::new(Vec::new()),
        pipewire: Cell::new(PipewireStep::Offer),
        mics_seen: Cell::new(false),
        connected: RefCell::new(None),
        follower: RefCell::new(follow::Follower::with_pinned(pinned)),
        worker: worker.clone(),
        window: window.as_weak(),
    });
    let tray = new_tray(&app)?;
    TRAY.set(Some(tray.clone_strong()));

    let state = window.global::<AppState>();
    state.set_version(env!("CARGO_PKG_VERSION").into());
    state.set_language_index(language as i32);
    state.set_download_size(megabytes(language).into());
    state.set_dictation_enabled(app.config.borrow().dictation);
    state.set_strength_index(strength_index(filter.limit_db));
    state.set_model_index(filter.light_model as i32);
    state.set_autostart(platform::autostart_enabled());
    state.set_update_auto(app.config.borrow().update_check);
    state.set_linux(cfg!(target_os = "linux"));
    state.set_tray(platform::has_tray());
    state.set_can_minimize(platform::can_minimize());
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
                    Ok(()) | Err(SetupError::Declined | SetupError::Gone) => set(UpdateState::Available),
                    Err(SetupError::Failed(e)) => {
                        eprintln!("kyrr: update: {e}");
                        set(UpdateState::DownloadFailed);
                    }
                }
            });
        }
    });
    state.on_show_current_news({
        let app = Rc::clone(&app);
        move || app.show_all_news()
    });
    state.on_show_news({
        let app = Rc::clone(&app);
        move || app.show_news()
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
    state.on_install_pipewire({
        let app = Rc::clone(&app);
        move || app.install_pipewire()
    });
    state.on_pipewire_step({
        let app = Rc::clone(&app);
        move |step| {
            let step = match step {
                2 => PipewireStep::AfterSignIn,
                3 => PipewireStep::Declined,
                _ => PipewireStep::Failed,
            };
            app.pipewire.set(step);
            app.refresh_pipewire(true);
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
            let target = app
                .default_microphone()
                .or_else(|| app.microphones.borrow().first().cloned())
                .map(|m| m.guid);
            if let Some(guid) = target {
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
    state.on_key_button_clicked({
        let app = Rc::clone(&app);
        move || {
            if matches!(platform::key_note(), KeyNote::Unbound) {
                platform::bind_key_again();
            } else {
                platform::open_key_settings();
            }
            app.refresh_key_note();
        }
    });
    state.on_setup_key({
        let app = Rc::clone(&app);
        move || {
            platform::bind_key_again();
            // The portal thread takes a moment to say it is asking.
            if let Some(w) = app.window.upgrade() {
                w.global::<AppState>().set_key_state(1);
            }
        }
    });
    state.on_allow_typing({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.global::<AppState>().set_typing_state(1);
            }
            let weak = weak.clone();
            std::thread::spawn(move || {
                let allowed = platform::allow_typing();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = weak.upgrade() {
                        w.global::<AppState>().set_typing_state(if allowed { 2 } else { 3 });
                        fit(&weak);
                    }
                });
            });
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
    // Without a tray (GNOME without its extension) both minimize, so kyrr
    // never hides where nobody can reach it, and a button in the window
    // quits. A desktop that cannot minimize (Hyprland, sway) quits on close.
    window.window().on_close_requested({
        let app = Rc::clone(&app);
        move || {
            if platform::has_tray() {
                app.window_closed();
            } else if platform::can_minimize() {
                app.minimize();
                return slint::CloseRequestResponse::KeepWindowShown;
            } else {
                app.window_closed();
                let _ = slint::quit_event_loop();
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    state.on_hide({
        let app = Rc::clone(&app);
        move || {
            if !platform::has_tray() {
                app.minimize();
                return;
            }
            app.window_closed();
            if let Some(w) = app.window.upgrade() {
                let _ = w.hide();
            }
        }
    });
    state.on_quit_app({
        let worker = app.worker.clone();
        move || {
            let _ = worker.send(Command::Quit);
            let _ = slint::quit_event_loop();
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
    state.on_unmute({
        let app = Rc::clone(&app);
        move || {
            if let Some(m) = app.working_microphone() {
                platform::unmute_microphone(&m.id);
            }
            app.check_mute();
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
        move || {
            // The microphones first: the filter's state is read against them.
            app.refresh_microphones();
            app.refresh_filter();
            app.check_mute();
            app.follow_default();
            app.refresh_key_note();
        }
    });
    // In the setup the answers to the desktop's questions and the first
    // press of the key show at once.
    let guide_poll = slint::Timer::default();
    guide_poll.start(slint::TimerMode::Repeated, Duration::from_millis(250), {
        let app = Rc::clone(&app);
        move || {
            let Some(w) = app.window.upgrade() else {
                return;
            };
            if w.get_setup_mode() && w.get_setup_step() == 1 && w.global::<AppState>().get_guide() {
                let before = (
                    w.global::<AppState>().get_key_state(),
                    w.global::<AppState>().get_key_tried(),
                );
                app.refresh_key_note();
                let after = (
                    w.global::<AppState>().get_key_state(),
                    w.global::<AppState>().get_key_tried(),
                );
                if before != after {
                    fit(&app.window);
                }
            }
        }
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

    // First visible start after an update (an existing install that last ran
    // another version): say what the new version brings, once. A start in the
    // background (Windows login) leaves it for the next visible one.
    let updated = !background && {
        let mut config = app.config.borrow_mut();
        let version = env!("CARGO_PKG_VERSION");
        let updated = config.last_run_version != version && (!config.last_run_version.is_empty() || config.setup_done);
        if config.last_run_version != version {
            config.last_run_version = version.into();
            config.save();
        }
        updated
    };
    if updated {
        slint::Timer::single_shot(Duration::from_secs(3), {
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    app.show_current_news();
                }
            }
        });
    }

    let setup_done = app.config.borrow().setup_done;
    window.set_setup_mode(!setup_done);
    // No filter on this PC: the setup starts with dictation. Without
    // PipeWire it starts with setting that up.
    if state.get_filter_state() == FilterState::Unsupported && !platform::pipewire_missing() {
        window.set_setup_step(1);
    }
    if !setup_done || !background {
        show(&window);
    } else if !platform::has_tray() && cfg!(windows) {
        // Started at login without a tray to sit in (or before the panel
        // that hosts it): give the panel a moment, then show up minimized
        // rather than run where nobody can reach kyrr. Not on Linux: not
        // every desktop can minimize (Hyprland would show the window), and
        // starting kyrr from the app menu brings up the running one.
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::from_secs(10), move || {
            if let Some(w) = weak.upgrade() {
                if !platform::has_tray() {
                    show(&w);
                    w.window().set_minimized(true);
                }
            }
        });
    }
    let tray_at_start = platform::has_tray();
    // One lookup for both, so a host that turns up in between cannot leave
    // the window saying "no tray" next to a visible icon.
    window.global::<AppState>().set_tray(tray_at_start);
    // Without a tray host Slint would only log a complaint: wait for one
    // (the retry below) instead of shouting at start.
    if tray_at_start {
        tray.show()?;
    }
    drop(tray);
    // A panel that hosts tray icons may start after kyrr (both at login).
    // An icon can only register once one is there: make a new one then.
    let tray_retry = slint::Timer::default();
    if !tray_at_start {
        let app = Rc::downgrade(&app);
        let started = Instant::now();
        let done = Cell::new(false);
        tray_retry.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || {
            if done.get() || started.elapsed() > Duration::from_secs(120) || !platform::has_tray() {
                return;
            }
            if let Some(app) = app.upgrade() {
                done.set(retry_tray(&app).is_ok());
            }
        });
    }
    #[cfg(target_os = "linux")]
    kyrr_linux::signals::on_terminate({
        let worker = worker.clone();
        move || {
            let _ = worker.send(Command::Quit);
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
        }
    });
    let result = slint::run_event_loop_until_quit();
    TRAY.take();
    result
}

/// A tray icon wired to the app.
fn new_tray(app: &Rc<App>) -> Result<Tray, slint::PlatformError> {
    let tray = Tray::new()?;
    let quit = {
        let worker = app.worker.clone();
        move || {
            let _ = worker.send(Command::Quit);
            let _ = slint::quit_event_loop();
        }
    };
    tray.on_quit(quit);
    tray.on_install_update({
        let weak = app.window.clone();
        move || {
            if let Some(w) = weak.upgrade() {
                show(&w);
                w.global::<AppState>().invoke_install_update();
            }
        }
    });
    tray.on_open({
        let app = Rc::clone(app);
        move || {
            if let Some(w) = app.window.upgrade() {
                app.refresh_microphones();
                app.refresh_filter();
                show(&w);
            }
        }
    });
    tray.on_toggle_filter({
        let app = Rc::clone(app);
        move || {
            let on = !app.filter.borrow().enabled;
            app.set_filter(|f| f.enabled = on);
        }
    });
    tray.on_toggle_voice({
        let app = Rc::clone(app);
        move || {
            let on = !app.filter.borrow().voice.enabled;
            app.set_voice(|v| v.enabled = on);
        }
    });
    tray.on_toggle_dictation({
        let app = Rc::clone(app);
        move || {
            let on = !app.config.borrow().dictation;
            app.set_dictation(on);
        }
    });
    Ok(tray)
}

/// Replaces the tray icon with a new one that shows what the old one did;
/// the old one never got a place in a panel.
fn retry_tray(app: &Rc<App>) -> Result<(), slint::PlatformError> {
    let new = new_tray(app)?;
    if let Some(old) = tray() {
        new.set_recording(old.get_recording());
        new.set_active(old.get_active());
        new.set_filter_enabled(old.get_filter_enabled());
        new.set_voice_enabled(old.get_voice_enabled());
        new.set_dictation_enabled(old.get_dictation_enabled());
        new.set_dictation_ready(old.get_dictation_ready());
        new.set_key_name(old.get_key_name());
        new.set_update_version(old.get_update_version());
        let _ = old.hide();
    }
    new.show()?;
    TRAY.set(Some(new));
    if let Some(w) = app.window.upgrade() {
        w.global::<AppState>().set_tray(true);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_notes_when_all_is_well() {
        assert!(alerts(true, true, 8.0, false, false, false, false).is_empty());
        assert!(alerts(false, false, 0.0, false, false, false, false).is_empty());
    }

    #[test]
    fn gain_at_its_limit_says_so() {
        assert_eq!(alerts(true, true, 24.0, false, false, false, false), [Alert::MicQuiet]);
        assert_eq!(alerts(true, true, -12.0, false, false, false, false), [Alert::MicLoud]);
        // Just short of the limits is still fine.
        assert!(alerts(true, true, 23.0, false, false, false, false).is_empty());
        assert!(alerts(true, true, -11.0, false, false, false, false).is_empty());
    }

    #[test]
    fn gain_only_counts_while_it_works() {
        // Automatic volume off, or no app on the microphone: the number is stale.
        assert!(alerts(false, true, 24.0, false, false, false, false).is_empty());
        assert!(alerts(true, false, -12.0, false, false, false, false).is_empty());
    }

    #[test]
    fn several_notes_come_in_turn() {
        assert_eq!(
            alerts(true, true, 24.0, true, false, false, false),
            [Alert::MicQuiet, Alert::Listening]
        );
        // Muted outweighs everything else, even with no app on the microphone.
        assert_eq!(alerts(true, true, 24.0, true, true, false, false), [Alert::Muted]);
        assert_eq!(alerts(false, false, 0.0, false, true, false, false), [Alert::Muted]);
        // No microphone at all, and one that just came back.
        assert_eq!(alerts(true, false, 0.0, true, false, true, false), [Alert::NoMic]);
        assert_eq!(alerts(false, false, 0.0, false, false, false, true), [Alert::Connected]);
        assert_eq!(alerts(false, false, 0.0, true, false, false, false), [Alert::Listening]);
    }
}
