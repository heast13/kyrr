//! kyrr: noise removal, voice polish and push-to-talk dictation in the tray.
//!
//! `kyrr.exe --setup <command>` runs an elevated setup step (see
//! `kyrr_windows::run_setup_command`); `--background` starts without
//! showing the window. On Linux `kyrr --dictate` starts or ends dictation
//! in the running kyrr, for desktops where kyrr cannot catch a key itself,
//! and `kyrr --memo` a voice memo.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod dictation;
#[cfg(target_os = "linux")]
mod engine;
mod follow;
mod keycapture;
mod listen;
mod memo;
mod memos;
mod platform;
mod player;
mod probe;
mod recorder;
mod servers;
#[cfg(target_os = "linux")]
mod skin;
mod update;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use kyrr_common::keys;
use kyrr_common::wave::{Mode, Overlay};
use kyrr_common::{Amount, FilterSettings, Look, Request, VoiceSettings};
use kyrr_dictate::Language;
use slint::{ComponentHandle, LogicalSize, Model, ModelRc, PhysicalPosition, SharedString, VecModel, Weak};

use config::Config;
use dictation::{Command, Event, State};
use platform::{Desktop, FilterStatus, KeyBinding, KeyNote, Microphone, PipewireInstall, SetupError};

slint::include_modules!();

/// Maximum attenuation for the strength choices gentle, medium, strong.
const STRENGTHS: [u32; 3] = [24, 48, 100];
/// The voice memo key kyrr takes now. Not before the setup is done on
/// Windows, where the hook would take it from every program at once (on
/// Linux the keys wait for the setup anyway).
fn memo_key_now(config: &config::Config) -> u32 {
    if cfg!(windows) && !config.setup_done {
        return 0;
    }
    config.memo_key()
}

/// What the desktop's shortcut list calls the voice memo key (German,
/// English).
const MEMO_DESCRIPTION: (&str, &str) = (
    "kyrr: Sprachaufzeichnung (Taste tippen)",
    "kyrr: Voice memo (tap the key)",
);

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
            "--background"
                | "--dictate"
                | "--dictate-start"
                | "--dictate-stop"
                | "--memo"
                | "--memo-start"
                | "--memo-stop"
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
        "setup failed" => "Die Einrichtung hat nicht geklappt. Nochmal versuchen, sonst kyrr neu installieren.",
        "disconnected" => "nicht angeschlossen",
        "probe failed" => "Der Test hat nicht geklappt. Sind Lautsprecher oder Kopfhörer angeschlossen?",
        "listen failed" => "Mithören hat nicht geklappt. Sind Mikrofon und Kopfhörer angeschlossen?",
        "typing key" => "Diese Taste braucht jedes Programm. Nimm eine F-Taste, Pause oder Strg rechts.",
        "memo key taken" => {
            "{} gehört schon einem anderen Kurzbefehl, die Aufzeichnung hat darum keine Taste. Such dir eine andere."
        }
        "taken by voice memos" => "{} ist schon für die Sprachaufzeichnung belegt.",
        "taken by dictation" => "{} ist schon fürs Diktieren belegt.",
        "right ctrl busy" => {
            "Mit Strg rechts gehen Tastenkürzel in anderen Programmen nicht mehr, und jedes Mal startet kyrr."
        }
        "busy key" => "{} nutzen viele Programme selbst. kyrr nimmt sie ihnen weg, solange es läuft.",
        other => other,
    };
    let english = match english {
        "permission" => "Windows did not grant permission. Please try again.",
        "setup failed" => "Setup did not finish. Try again, or reinstall kyrr.",
        "disconnected" => "disconnected",
        "probe failed" => "The test did not work. Are your speakers or headphones plugged in?",
        "listen failed" => "Listening did not work. Are your microphone and headphones plugged in?",
        "typing key" => "Every program needs that key. Take an F key, Pause or Right Ctrl.",
        "memo key taken" => "Another shortcut already has {}, so recordings have no key. Pick another.",
        "taken by voice memos" => "{} is already used for voice memos.",
        "taken by dictation" => "{} is already used for dictation.",
        "right ctrl busy" => "With Right Ctrl, shortcuts in other programs stop working, and kyrr starts every time.",
        "busy key" => "Many programs use {} themselves. kyrr takes it from them while it runs.",
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
        "Beim ersten Diktieren fragt dein Desktop nach „Remote Desktop“ (oder „Entfernter Bildschirm“): „Fernzugriff zulassen“ einschalten, dann „Freigeben“. kyrr tippt damit nur deinen Text."
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
        KeyNote::Taken { key } if german => {
            format!("{key} hat schon einen anderen Job. Such dir hier eine andere Taste aus.")
        }
        KeyNote::Taken { key } => format!("{key} already has another job. Pick another key here."),
        KeyNote::PressOnly { desktop, key } if german => format!(
            "{desktop} kann die Taste nur drücken, nicht halten: {key} einmal drücken startet das Diktieren, nochmal drücken beendet es."
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
        KeyNote::Command(command) if german => format!(
            "kyrr kann die Taste hier nicht selbst einrichten. Leg in den Tastatur-Einstellungen deines Desktops diesen Befehl auf eine Taste, jeder Druck startet oder beendet das Diktieren:\n{command}"
        ),
        KeyNote::Command(command) => format!(
            "kyrr cannot set the key up here by itself. Put this command on a key in your desktop's keyboard settings, each press starts or ends dictation:\n{command}"
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
            "Hol dir PipeWire aus der Softwareverwaltung deines Systems (such nach „PipeWire“), dann einmal ab- und wieder anmelden.".into()
        } else {
            "Get PipeWire from your system's software manager (search for “PipeWire”), then sign out and back in once."
                .into()
        };
    };
    match step {
        _ if !can_ask && german => format!("Im Terminal geht's so, danach einmal ab- und wieder anmelden:\n{command}"),
        _ if !can_ask => format!("In a terminal it goes like this, then sign out and back in once:\n{command}"),
        PipewireStep::Offer if german => {
            "Ein Klick, dein System fragt nach deinem Passwort, kyrr holt PipeWire (es löst PulseAudio ab) und startet neu.".into()
        }
        PipewireStep::Offer => {
            "One click, your system asks for your password, kyrr fetches PipeWire (it takes over from PulseAudio) and restarts.".into()
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

/// Whether the speech model dictation would use is on the computer: memos
/// get their titles and texts from it.
fn model_installed(config: &Config) -> bool {
    let spec = if config.more_languages() {
        kyrr_dictate::european()
    } else {
        config.language().model()
    };
    spec.is_installed(&config::models_dir())
}

fn megabytes(config: &Config) -> String {
    let spec = if config.more_languages() {
        kyrr_dictate::european()
    } else {
        config.language().model()
    };
    format!("{} MB", spec.total_size() / 1_000_000)
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
    state.set_distance(v.distance);
    state.set_auto_level(v.auto_level);
    state.set_deesser_index(v.deesser as i32);
    state.set_compressor_index(v.compressor as i32);
    state.set_voice_is_default(*v == voice_defaults(v));
}

/// Tallest the window gets, screen permitting; more content scrolls. Room
/// for the tallest settings page (Setup with an update note, in German about
/// 1420).
const MAX_HEIGHT: f32 = 1460.0;
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

/// The recordings page is this tall at least (screen permitting): a list
/// and a text to read want room.
const MEMOS_HEIGHT: f32 = 780.0;

/// The tallest the window gets on the screen it is on.
fn height_cap(w: &MainWindow) -> f32 {
    screen(w).map_or(FALLBACK_HEIGHT, |(_, top, _, bottom)| {
        ((bottom - top) as f32 / w.window().scale_factor() - FRAME).clamp(MIN_HEIGHT, MAX_HEIGHT)
    })
}

/// The height the content wants, up to what the screen has room for. Also
/// tells the window that cap: the main page's height is fixed to its
/// content, and must not outgrow a small screen.
fn wanted_height(w: &MainWindow) -> f32 {
    let cap = height_cap(w);
    w.set_height_cap(cap);
    let memos = !w.get_setup_mode() && w.global::<AppState>().get_page() == 6;
    let content = w.get_content_height();
    (if memos { content.max(MEMOS_HEIGHT) } else { content }).min(cap)
}

#[cfg(target_os = "linux")]
thread_local! {
    /// Ends the maximum that lets a window shrink (see [`resize`]).
    static FIT_HOLD: slint::Timer = slint::Timer::default();
}

/// Sets the window's height (its width is the page's) and moves it up or
/// left when it would end up behind the taskbar or off the screen.
fn resize(w: &MainWindow, height: f32) {
    let win = w.window();
    let width = w.get_page_width();
    #[cfg(target_os = "linux")]
    let shrinking = height < win.size().to_logical(win.scale_factor()).height;
    win.set_size(LogicalSize::new(width, height));
    #[cfg(target_os = "linux")]
    {
        w.set_fit_height(height);
        // Some compositors keep a floating window at the size they last
        // gave it and only follow the limits (Hyprland): growing works
        // through the minimum, shrinking needs the maximum for a moment.
        if shrinking {
            w.set_fit_hold(true);
            // Then the window can be dragged taller again. One timer, so a
            // newer resize keeps its own hold.
            let weak = w.as_weak();
            FIT_HOLD.with(|timer| {
                timer.start(slint::TimerMode::SingleShot, Duration::from_millis(500), move || {
                    if let Some(w) = weak.upgrade() {
                        w.set_fit_hold(false);
                    }
                });
            });
        }
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

/// Room for a memo's text: the window grows as tall as the screen allows
/// (up to a comfortable reading height) while it is open, and fits its
/// content again after.
const SHEET_HEIGHT: f32 = 820.0;

fn fit_sheet(w: &MainWindow, open: bool) {
    if !open {
        fit(&w.as_weak());
        return;
    }
    let win = w.window();
    // Two columns, side by side, fit on the screen.
    if let Some((left, _, right, _)) = screen(w) {
        w.set_widest(((right - left) as f32 / win.scale_factor() - FRAME_SIDES).max(520.0));
    }
    let cap = height_cap(w);
    let height = win.size().to_logical(win.scale_factor()).height;
    let wanted = SHEET_HEIGHT.min(cap);
    if wanted > height + 0.5 {
        resize(w, wanted);
    }
}

/// How often the status line moves on to its next phrase, in 100 ms ticks.
const PHRASE_TICKS: u32 = 150;
/// The phrase fades out this many ticks before the next one fades in
/// (matches the fade in the .slint file).
const PHRASE_FADE_TICKS: u32 = 5;

/// How many phrases every state of the status line has (the lists in
/// app.slint; a test keeps them in step).
const PHRASES: i32 = 20;

/// Moves the status line on to a random phrase, never the one it shows now
/// (every list in the .slint file has `PHRASES` entries, so "the same
/// index" is "the same phrase" in whatever state comes next).
fn next_phrase(state: &AppState) {
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let current = state.get_phrase().rem_euclid(PHRASES);
    // 1..PHRASES steps forward from the current one.
    let step = 1 + (clock / 1000 % (PHRASES - 1) as u32) as i32;
    state.set_phrase((current + step) % PHRASES);
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
    /// The microphone sounds far away: the room echoes on every word.
    MicFar = 8,
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
    far: bool,
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
    if far && mic_active {
        list.push(Alert::MicFar);
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
    // On Omarchy with kyrr's widget the widget's menu is all of kyrr.
    if headless() {
        #[cfg(target_os = "linux")]
        kyrr_linux::omarchy::open_menu();
        return;
    }
    if !window.window().is_visible() {
        // The width first: until the window has one, text is measured as if
        // it wrapped after every word, and the content seems far taller.
        // Wayland compositors may keep the first size they saw.
        let width = window.get_page_width();
        window.window().set_size(LogicalSize::new(width, FALLBACK_HEIGHT));
        resize(window, wanted_height(window));
    }
    window.global::<AppState>().set_window_shown(true);
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
    /// Runs `tick` while the window is open, or the menu of kyrr's widget in
    /// Omarchy's bar; nobody sees the meter otherwise.
    meter: slint::Timer,
    /// When the widget's menu last said it is open.
    widget_open: Cell<Option<Instant>>,
    /// When the open menu last got kyrr's state from the meter's ticks.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    widget_told: Cell<Option<Instant>>,
    /// In the setup, shows the desktop's answers and the first press of the
    /// key at once; runs with the window like the meter.
    guide: slint::Timer,
    heartbeat: Cell<u32>,
    ticks: Cell<u32>,
    /// Which of the red notes is up; moves on with the phrases.
    alert_turn: Cell<u32>,
    /// Ticks in a row the microphone was in use but silent.
    silent_ticks: Cell<u32>,
    model_changed: Cell<Option<Instant>>,
    /// The filter's status says its model is too slow for this computer.
    too_slow: Cell<bool>,
    /// The main page said that the noise filter pauses now and then.
    overload_told: Cell<bool>,
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
    /// Voice memo key: tap to start and stop (else hold).
    memo_toggle: Arc<AtomicBool>,
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

thread_local! {
    /// kyrr runs without a window or tray icon of its own: on Omarchy, with
    /// its widget turned on in the bar, whose menu is all of kyrr.
    static HEADLESS: Cell<bool> = const { Cell::new(false) };
    /// Where the cursor stands in the text open in the widget's menu, as a
    /// byte offset: "Play from here" says from when.
    static SHEET_CURSOR: Cell<i32> = const { Cell::new(0) };
}

fn headless() -> bool {
    HEADLESS.get()
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
        // the CPU: kyrr picks the lean setting, once.
        let old_cpu = fs == FilterState::Running && detail.starts_with("onnx");
        // The noise model took most of the audio's time and is left out a
        // while (see `kyrr_denoise::Load`): the light one, whatever was
        // picked, or the microphone stutters (and on Linux the system ends
        // kyrr). Unless the user wants to stay; then the main page says so,
        // once.
        // Only when it turns too slow: the status says so until the next
        // filter runs, and a pick by hand meanwhile stays.
        let too_slow = detail.starts_with(kyrr_common::TOO_SLOW);
        let turned = too_slow && !self.too_slow.replace(too_slow);
        state.set_old_cpu(old_cpu);
        if turned {
            if self.config.borrow().auto_lean && !self.filter.borrow().light_model {
                eprintln!("kyrr: the noise model is too slow for this computer, switching to lean ({detail})");
                self.set_effort(true);
                state.set_overload_note(OverloadNote::Switched);
            } else if !self.overload_told.replace(true) {
                eprintln!("kyrr: the noise model is too slow for this computer, left out a while ({detail})");
                state.set_overload_note(OverloadNote::Pauses);
            }
        } else if old_cpu && !self.config.borrow().lean_suggested {
            {
                let mut config = self.config.borrow_mut();
                config.lean_suggested = true;
                config.save();
            }
            self.set_effort(true);
        }
        let _ = self.worker.send(Command::Microphone(guid.clone()));
        if let Some(m) = memos::memos() {
            m.set_microphone(guid.clone());
        }
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
            .start(slint::TimerMode::Repeated, Duration::from_millis(100), {
                let app = app.clone();
                move || {
                    if let Some(app) = app.upgrade() {
                        app.tick();
                    }
                }
            });
        self.guide
            .start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
                if let Some(app) = app.upgrade() {
                    app.follow_guide();
                }
            });
    }

    /// In the setup the answers to the desktop's questions and the first
    /// press of the key show at once.
    fn follow_guide(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        if w.get_setup_mode() && w.get_setup_step() == 1 && w.global::<AppState>().get_guide() {
            let before = (
                w.global::<AppState>().get_key_state(),
                w.global::<AppState>().get_key_tried(),
            );
            self.refresh_key_note();
            let after = (
                w.global::<AppState>().get_key_state(),
                w.global::<AppState>().get_key_tried(),
            );
            if before != after {
                fit(&self.window);
            }
        }
    }

    /// The window went away: no meter, no listening.
    /// Minimizes the window. The meter keeps its timer, which idles while
    /// the window is minimized (on Wayland it cannot tell and goes on):
    /// nothing starts it again when the taskbar brings the window back.
    fn minimize(&self) {
        self.stop_listening();
        if let Some(w) = self.window.upgrade() {
            platform::minimize(w.window());
        }
    }

    fn window_closed(&self) {
        self.meter.stop();
        self.guide.stop();
        self.stop_listening();
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_window_shown(false);
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
        // Minimized, nobody looks either (unless the widget's menu does).
        if w.window().is_minimized() && self.widget_open.get().is_none() {
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
            levels.far,
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
        // The open menu follows the status line, a few times a second.
        #[cfg(target_os = "linux")]
        if self.widget_open.get().is_some() && self.widget_told.get().is_none_or(|at| at.elapsed() >= WIDGET_EVERY) {
            self.widget_told.set(Some(Instant::now()));
            tell_widget(&w);
        }
    }

    /// The menu of kyrr's widget in Omarchy's bar opened (it says so every
    /// few seconds while it stays open) or closed. The status line and the
    /// notes run for it as for the window.
    fn widget_looks(self: &Rc<Self>, open: bool) {
        if open {
            self.widget_open.set(Some(Instant::now()));
            self.start_meter();
            return;
        }
        self.widget_open.set(None);
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        if !state.get_window_shown() {
            self.meter.stop();
            self.guide.stop();
            state.set_mic_active(false);
            state.set_alert(0);
        }
    }

    /// A menu that stopped saying it is open (the shell went away) counts
    /// as closed.
    fn widget_still_looks(self: &Rc<Self>) {
        if self.widget_open.get().is_some_and(|at| at.elapsed() > WIDGET_GONE) {
            self.widget_looks(false);
        }
    }

    /// A line from the menu of kyrr's widget in Omarchy's bar, which is all
    /// of kyrr there (the widget's README, "Commands"): does what the same
    /// click in the window does.
    /// Takes what the widget's menu typed into the open text, as typing in
    /// the window's text view would.
    fn omarchy_edit(&self, state: &AppState) {
        let Some((id, text)) = platform::take_widget_edit() else {
            return;
        };
        // Typed into the recording that is still open, not one left since.
        let open = state.get_memo_sheet_open()
            && memos::memos().is_some_and(|m| m.sheet_name().as_deref() == Some(id.as_str()));
        if open && state.get_memo_sheet_text() != text.as_str() {
            state.set_memo_sheet_text(text.as_str().into());
            state.invoke_memo_sheet_edited(text.into());
        }
    }

    fn omarchy_command(self: &Rc<Self>, line: &str) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        let mut words = line.trim().splitn(3, ' ');
        let verb = words.next().unwrap_or_default();
        let what = words.next().unwrap_or_default();
        let value = words.next().unwrap_or_default().trim();
        macro_rules! flip {
            ($get:ident, $set:ident, $call:ident) => {{
                let on = !state.$get();
                state.$set(on);
                state.$call(on);
            }};
        }
        // A memo by the name of its file.
        let memo = || memos::memos().and_then(|m| m.index_of(value));
        match (verb, what) {
            ("toggle", "filter") => flip!(get_filter_enabled, set_filter_enabled, invoke_filter_toggled),
            ("toggle", "voice") => flip!(get_voice_enabled, set_voice_enabled, invoke_voice_toggled),
            ("toggle", "dictation") => flip!(get_dictation_enabled, set_dictation_enabled, invoke_dictation_toggled),
            ("toggle", "memos") => flip!(get_memos_enabled, set_memos_enabled, invoke_memos_toggled),
            ("toggle", "voice.rumble") => flip!(get_low_cut, set_low_cut, invoke_low_cut_toggled),
            ("toggle", "voice.pops") => flip!(get_pops, set_pops, invoke_pops_toggled),
            ("toggle", "voice.distance") => flip!(get_distance, set_distance, invoke_distance_toggled),
            ("toggle", "voice.autoLevel") => flip!(get_auto_level, set_auto_level, invoke_auto_level_toggled),
            ("toggle", "memos.autoText") => {
                flip!(get_memo_auto_text, set_memo_auto_text, invoke_memo_auto_text_toggled)
            }
            ("toggle", "general.liveText") => flip!(get_live_words, set_live_words, invoke_live_words_toggled),
            ("toggle", "general.overload") => flip!(get_auto_lean, set_auto_lean, invoke_auto_lean_toggled),
            ("toggle", "general.updateCheck") => flip!(get_update_auto, set_update_auto, invoke_update_auto_toggled),
            ("toggle", "listen") => state.invoke_listen_toggled(),
            ("toggle", "ai.features") => flip!(get_ai_features, set_ai_features, invoke_ai_features_toggled),
            ("toggle", "ai.on") => flip!(get_local_ai, set_local_ai, invoke_local_ai_toggled),
            ("set", setting) => self.omarchy_set(&state, setting, value),
            ("mic", "unmute") => state.invoke_unmute(),
            ("mic", "repair") => state.invoke_repair(),
            ("mic", "default") => state.invoke_move_to_default(),
            ("mic", index) => {
                let Ok(index) = index.parse::<i32>() else {
                    return;
                };
                // The first-run setup picks one and sets it up with "Next".
                if w.get_setup_mode() {
                    state.set_mic_index(index);
                } else {
                    state.invoke_attach_microphone(index);
                }
            }
            ("memo", "start") if !memos::memos().is_some_and(|m| m.recording()) => state.invoke_memo_record(),
            ("memo", "stop") if memos::memos().is_some_and(|m| m.recording()) => state.invoke_memo_record(),
            ("memo", "play") => {
                if let Some(i) = memo() {
                    state.invoke_memo_play(i);
                }
            }
            ("memo", "text") => {
                if let Some(i) = memo() {
                    state.invoke_memo_write_text(i);
                }
            }
            ("memo", "delete") => {
                if let Some(i) = memo() {
                    state.invoke_memo_delete(i);
                }
            }
            ("memo", "folder") => state.invoke_memo_open_folder(),
            ("memo", "reveal") => {
                if let Some(i) = memo() {
                    state.invoke_memo_show(i);
                }
            }
            ("memo", "cancel") => {
                if let Some(i) = memo() {
                    state.invoke_memo_cancel_text(i);
                }
            }
            ("memo", "rename") => {
                // ["file name", "new title"]: both may hold spaces.
                let Ok((name, title)) = serde_json::from_str::<(String, String)>(value) else {
                    return;
                };
                // Typed into its text under the old name: saved first.
                self.omarchy_edit(&state);
                if let Some(i) = memos::memos().and_then(|m| m.index_of(&name)) {
                    state.invoke_memo_rename(i, title.into());
                }
            }
            ("memo", "search") => {
                state.set_memo_query(value.into());
                state.invoke_memo_search(value.into());
            }
            // The open text: what the menu typed comes first, so each of
            // these works on the text as it is on the screen.
            ("memo", "edit") => self.omarchy_edit(&state),
            ("memo", "copy") => {
                self.omarchy_edit(&state);
                let mut at = value.split(' ').map(|n| n.parse::<i32>().unwrap_or(0));
                let (a, b) = (at.next().unwrap_or(0), at.next().unwrap_or(0));
                state.invoke_memo_copy_text(a, b);
            }
            ("memo", "cursor") => SHEET_CURSOR.set(value.parse().unwrap_or(0)),
            ("memo", "here") => {
                self.omarchy_edit(&state);
                state.invoke_memo_play_here(value.parse().unwrap_or(0));
            }
            ("memo", "stamps") => {
                self.omarchy_edit(&state);
                let i = value.parse::<i32>().unwrap_or(0).clamp(0, 2);
                state.set_memo_sheet_stamps(i);
                state.invoke_memo_sheet_stamps_changed(i);
            }
            // The language model on the open text, as chosen with
            // `set memos.reworkAction|reworkTarget`; typed text goes first.
            ("memo", "rework") => {
                self.omarchy_edit(&state);
                state.invoke_memo_rework(state.get_memo_rework_action(), state.get_memo_rework_target());
            }
            ("memo", "stoprework") => state.invoke_memo_stop_rework(),
            ("memo", "restore") => state.invoke_memo_restore_text(),
            ("memo", "view") => {
                // 0 the text as it is, 1 the text beside the one before.
                let compare = value == "1";
                state.set_memo_sheet_view(i32::from(compare));
                if compare {
                    state.invoke_memo_show_compare();
                }
            }
            ("memo", "opentext") => {
                self.omarchy_edit(&state);
                state.invoke_memo_open_text();
            }
            ("memo", "show") => {
                // What was typed into the one open before stays with it.
                self.omarchy_edit(&state);
                SHEET_CURSOR.set(0);
                if let Some(m) = memos::memos() {
                    m.show_in_widget((!value.is_empty()).then_some(value));
                }
            }
            ("memo", "seen") => {
                if let Some(m) = memos::memos() {
                    m.mark_seen();
                }
            }
            ("update", "install") => state.invoke_install_update(),
            ("update", "check") => state.invoke_check_updates(),
            ("update", "dismiss") => state.invoke_dismiss_update(),
            ("update", "page") => state.invoke_open_update(),
            ("test", "start") => state.invoke_probe_record(),
            ("test", "before") => state.invoke_probe_play(false),
            ("test", "after") => state.invoke_probe_play(true),
            ("test", "delete") => state.invoke_probe_reset(),
            ("model", "download") => state.invoke_download_model(),
            ("model", "cancel") => state.invoke_cancel_download(),
            ("model", "retry") => state.invoke_dictation_retry(),
            ("setup", "next") => match w.get_setup_step() {
                0 if state.get_microphones().row_count() == 0
                    || state.get_filter_state() == FilterState::Unsupported =>
                {
                    state.invoke_skip_setup()
                }
                0 if state.get_filter_state() != FilterState::SettingUp => {
                    state.invoke_attach_microphone(state.get_mic_index())
                }
                1 => state.invoke_next_setup(),
                _ => {}
            },
            ("setup", "skip") => state.invoke_skip_setup(),
            ("setup", "keybutton") => state.invoke_key_button_clicked(),
            ("setup", "overloadSeen") => state.set_overload_note(OverloadNote::None),
            ("setup", "done") => state.invoke_finish_setup(),
            ("open", "feedback") => state.invoke_send_feedback(),
            ("open", "licenses") => state.invoke_open_licenses(),
            _ => {}
        }
        #[cfg(target_os = "linux")]
        tell_widget(&w);
    }

    /// `set <setting> <value>` from the widget's menu.
    fn omarchy_set(self: &Rc<Self>, state: &AppState, setting: &str, value: &str) {
        let pick = |names: &[&str]| -> Option<i32> {
            names
                .iter()
                .position(|n| *n == value)
                .and_then(|i| i32::try_from(i).ok())
                .or_else(|| value.parse().ok())
        };
        let level = || pick(&["off", "light", "medium", "strong"]).map(|i| i.clamp(0, 3));
        let band = |i: i32| {
            if let Ok(db) = value.parse::<f32>() {
                let db = db.clamp(-6.0, 6.0);
                match i {
                    0 => state.set_body_db(db),
                    1 => state.set_presence_db(db),
                    _ => state.set_air_db(db),
                }
                state.invoke_band_changed(i, db);
            }
        };
        match setting {
            "filter.strength" => {
                if let Some(i) = pick(&["gentle", "medium", "strong"]).map(|i| i.clamp(0, 2)) {
                    state.set_strength_index(i);
                    state.invoke_strength_changed(i);
                }
            }
            "voice.tone" => {
                if let Some(i) = pick(&["neutral", "warm", "clear", "bright", "custom"]).map(|i| i.clamp(0, 4)) {
                    state.set_look_index(i);
                    state.invoke_look_changed(i);
                }
            }
            "voice.reset" => state.invoke_voice_reset(),
            "dictation.keyReset" => state.invoke_key_reset(1),
            "memos.keyReset" => state.invoke_key_reset(2),
            "voice.body" => band(0),
            "voice.presence" => band(1),
            "voice.air" => band(2),
            "voice.comp" => {
                if let Some(i) = level() {
                    state.set_compressor_index(i);
                    state.invoke_compressor_changed(i);
                }
            }
            "voice.ess" => {
                if let Some(i) = level() {
                    state.set_deesser_index(i);
                    state.invoke_deesser_changed(i);
                }
            }
            "dictation.key" | "memos.key" => {
                if let Some(vk) = keys::from_xkb_name(value) {
                    self.set_key(if setting == "dictation.key" { 1 } else { 2 }, vk);
                }
            }
            // Held down while talking, or tapped to start and again to end.
            "dictation.hold" => {
                let tap = value != "true";
                state.set_key_toggle(tap);
                state.invoke_key_toggle_changed(tap);
            }
            "memos.hold" => state.invoke_memo_toggle_changed(value != "true"),
            // Local AI, the own speech server and reworking a recording.
            _ if setting.starts_with("ai.") || setting.starts_with("asr.") || setting.starts_with("memos.rework") => {
                servers::widget_set(self, setting, value)
            }
            "general.language" => {
                if let Some(i) = pick(&["de", "en", "multi"]).map(|i| i.clamp(0, 2)) {
                    state.set_language_index(i);
                    state.invoke_language_changed(i);
                }
            }
            "general.effort" => {
                if let Some(i) = pick(&["lean", "best"]).map(|i| i.clamp(0, 1)) {
                    state.set_effort_index(i);
                    state.invoke_effort_changed(i);
                }
            }
            _ => {}
        }
    }

    /// Lean (small noise model, no words while speaking) or the best
    /// quality, both at once.
    fn set_effort(&self, lean: bool) {
        if self.filter.borrow().light_model != lean {
            self.set_model(lean);
        }
        {
            let mut config = self.config.borrow_mut();
            config.dictation_live = !lean;
            config.save();
        }
        let _ = self.worker.send(Command::Live(!lean));
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_live_words(!lean);
        }
        self.show_effort();
    }

    /// The effort setting from the two it stands for: lean, best, or a mix.
    fn show_effort(&self) {
        let lean = self.filter.borrow().light_model;
        let live = self.config.borrow().dictation_live;
        // Lean dictations are short: a slow computer takes long to write
        // a long one.
        let _ = self.worker.send(Command::Short(lean && !live));
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<AppState>();
        state.set_effort_index(match (lean, live) {
            (true, false) => 0,
            (false, true) => 1,
            _ => 2,
        });
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
                eprintln!("kyrr: test: {e}");
                state.set_probe_error(text(self.language(), "probe failed").into());
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
            UPDATE_CHECK_FAILED.store(outcome == update::Outcome::Failed, Ordering::Relaxed);
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
                        installer,
                        news,
                    } => {
                        state.set_update_dismissed(version == dismissed);
                        state.set_update_version(version.as_str().into());
                        // The installer is Windows'; elsewhere kyrr is updated
                        // where it came from (AUR, a new AppImage).
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
        #[cfg(target_os = "linux")]
        skin::apply(&win.global::<Skin>());
        win.set_version(version.into());
        win.set_mode(mode);
        win.set_news(text.into());
        let weak = win.as_weak();
        win.on_close(move || {
            if let Some(win) = weak.upgrade() {
                let _ = win.hide();
            }
        });
        keycapture::install(win.window(), None);
        if win.show().is_ok() {
            *self.news_window.borrow_mut() = Some(win);
        }
    }

    /// The daily check: when it is on and a day has passed since kyrr last
    /// asked.
    fn check_updates_daily(&self) {
        let due = {
            let config = self.config.borrow();
            config.update_check
                && update_due(
                    config.update_checked,
                    now(),
                    UPDATE_CHECK_FAILED.load(Ordering::Relaxed),
                )
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
                eprintln!("kyrr: listening: {e}");
                state.set_probe_error(prefix.into());
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
        let (key, toggle) = {
            let config = self.config.borrow();
            (config.dictation_key(), config.dictation_toggle)
        };
        platform::set_key_wanted(self.config.borrow().dictation);
        platform::join_omarchy(self.language() == Language::German, self.config.borrow().dictation);
        platform::set_hotkey_key(key);
        platform::set_hotkey_tap(toggle);
        self.key_toggle.store(toggle, Ordering::Relaxed);
        let name = keys::label(key, self.language() == Language::German);
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_key_toggle(toggle);
            state.set_key_name(name.as_str().into());
            state.set_key_custom(key != keys::DICTATION_DEFAULT);
        }
        if let Some(t) = tray() {
            t.set_key_name(name.into());
        }
        // The memo key moves off dictation's; then the desktop gets both.
        self.apply_memo_key();
    }

    /// The voice memo key and mode: never dictation's (see
    /// `Config::memo_key`).
    fn apply_memo_key(&self) {
        let (key, choice, toggle) = {
            let config = self.config.borrow();
            (memo_key_now(&config), config.memo_key_choice(), config.memo_toggle)
        };
        let description = match self.language() {
            Language::German => MEMO_DESCRIPTION.0,
            Language::English => MEMO_DESCRIPTION.1,
        };
        platform::set_memo_key(key, description);
        platform::set_memo_hold(!toggle);
        let german = self.language() == Language::German;
        // "" while the key is not taken (memos off, Windows before the setup).
        let name = keys::label(key, german);
        let label = keys::label(choice, german);
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_memo_toggle(toggle);
            state.set_memo_key_custom(choice != keys::MEMO_DEFAULT);
            state.set_memo_key_label(label.into());
            state.set_memo_key_name(name.as_str().into());
        }
        if let Some(t) = tray() {
            t.set_memo_key_name(name.into());
        }
        self.refresh_key_note();
    }

    /// A key field starts waiting for a key (`slot` 1 dictation, 2 voice
    /// memos), or stops (0).
    fn capture(&self, slot: i32) {
        keycapture::set_capturing(slot != 0);
        platform::set_capture(slot != 0);
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_key_capture(slot);
            state.set_key_refusal("".into());
            state.set_key_warning("".into());
        }
    }

    /// A key field heard something: take the key, or say why not.
    fn key_heard(&self, heard: keycapture::Captured) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let slot = w.global::<AppState>().get_key_capture();
        if slot == 0 {
            return;
        }
        match heard {
            keycapture::Captured::Cancelled => self.capture(0),
            keycapture::Captured::Refused => {
                self.capture(0);
                let message = text(self.language(), "typing key");
                w.global::<AppState>().set_key_refusal(message.into());
            }
            keycapture::Captured::Key(vk) => self.set_key(slot, vk),
        }
    }

    /// Takes `vk` as the key of `slot` (1 dictation, 2 voice memos) unless the
    /// other one has it.
    fn set_key(&self, slot: i32, vk: u32) {
        self.capture(0);
        let language = self.language();
        let german = language == Language::German;
        let label = keys::label(vk, german);
        let (other, taken) = {
            let config = self.config.borrow();
            match slot {
                1 => (config.memo_key(), "taken by voice memos"),
                _ => (config.dictation_key(), "taken by dictation"),
            }
        };
        let (mut refusal, mut warning) = (String::new(), String::new());
        if vk == other {
            refusal = text(language, taken).replace("{}", &label);
        } else {
            {
                let mut config = self.config.borrow_mut();
                match slot {
                    1 => config.dictation_key = vk,
                    _ => config.memo_key = vk,
                }
                config.save();
            }
            if vk == keys::RIGHT_CTRL {
                warning = text(language, "right ctrl busy");
            } else if keys::busy(vk) {
                warning = text(language, "busy key").replace("{}", &label);
            }
            match slot {
                1 => self.apply_key(),
                _ => {
                    self.apply_memo_key();
                    platform::bind_key_again();
                }
            }
        }
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_key_refusal(refusal.into());
            state.set_key_warning(warning.into());
        }
    }

    /// Voice memos on or off: the card stays, its key and tray entry go.
    fn set_memos(&self, on: bool) {
        {
            let mut config = self.config.borrow_mut();
            config.memos = on;
            config.save();
        }
        if let Some(m) = memos::memos() {
            m.set_enabled(on);
        }
        self.apply_memo_key();
        // A key the desktop owns (the portal) is bound again with or
        // without the memo's, so F8 is free again when off.
        platform::bind_key_again();
        if let Some(w) = self.window.upgrade() {
            w.global::<AppState>().set_memos_enabled(on);
        }
        if let Some(t) = tray() {
            t.set_memos_enabled(on);
        }
        fit(&self.window);
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
        // A key that only reports its press (GNOME 46 and 47, niri, a command
        // the user binds) cannot be held: each press toggles, whatever the
        // setting says.
        let can_hold = platform::memo_can_hold();
        self.memo_toggle
            .store(self.config.borrow().memo_toggle || !can_hold, Ordering::Relaxed);
        state.set_memo_can_hold(can_hold);
        // Another bind of the user's has the memo key: it does not work, and
        // the settings say so.
        let blocked = platform::memo_key_blocked();
        if blocked.is_some() {
            state.set_memo_key_name("".into());
            if let Some(t) = tray() {
                t.set_memo_key_name("".into());
            }
            let label = keys::label(self.config.borrow().memo_key_choice(), language == Language::German);
            state.set_memo_key_note(text(language, "memo key taken").replace("{}", &label).into());
        } else {
            state.set_memo_key_note("".into());
            let name = keys::label(memo_key_now(&self.config.borrow()), language == Language::German);
            state.set_memo_key_name(name.as_str().into());
            if let Some(t) = tray() {
                t.set_memo_key_name(name.into());
            }
        }
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

    /// `index` is the language choice: German, English, multilingual.
    fn set_language(&self, index: i32) {
        {
            let mut config = self.config.borrow_mut();
            config.language = match index {
                0 => "de",
                1 => "en",
                _ => config::MULTI,
            }
            .into();
            config.save();
        }
        let language = self.language();
        let _ = slint::select_bundled_translation(language.code());
        let _ = self
            .worker
            .send(Command::Language(language, self.config.borrow().more_languages()));
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<AppState>();
            state.set_language_index(index);
            state.set_download_size(megabytes(&self.config.borrow()).into());
        }
        if let Some(m) = memos::memos() {
            m.set_language(language);
            m.set_can_write(model_installed(&self.config.borrow()));
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
        platform::join_omarchy(self.language() == Language::German, on);
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
                            SetupError::Failed(e) => {
                                eprintln!("kyrr: setup: {e}");
                                text(language, "setup failed")
                            }
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

/// After a check that failed (started before the Wi-Fi was up, say), the
/// next try comes this much sooner than a day.
const UPDATE_RETRY: Duration = Duration::from_secs(60 * 60);

/// The last check did not reach GitHub.
static UPDATE_CHECK_FAILED: AtomicBool = AtomicBool::new(false);

/// Whether the daily check is due: a day after the last one, an hour after
/// one that failed, and at once when the last one lies in the future (the
/// clock was wrong then, or is now).
fn update_due(checked: u64, now: u64, failed: bool) -> bool {
    let since = now.saturating_sub(checked);
    checked > now || since >= UPDATE_INTERVAL.as_secs() || (failed && since >= UPDATE_RETRY.as_secs())
}

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

fn run(background: bool, show_requests: Option<mpsc::Receiver<Request>>) -> Result<(), slint::PlatformError> {
    let config = Config::load();
    let language = config.language();
    HEADLESS.set(platform::headless());
    // An installer downloaded for the last update is done with.
    let _ = std::fs::remove_dir_all(update::folder());
    let window = MainWindow::new()?;
    #[cfg(target_os = "linux")]
    skin::refresh(&window);
    // What kyrr does, for its widget in Omarchy's bar.
    #[cfg(target_os = "linux")]
    {
        kyrr_common::wave::on_mode(|mode| {
            kyrr_linux::omarchy::set_state(match mode {
                Mode::Hidden => "idle",
                Mode::Listening => "listening",
                Mode::Writing => "writing",
                Mode::Recording => "recording",
            })
        });
        kyrr_linux::omarchy::set_state("idle");
    }
    // So the desktop matches the window to kyrr's desktop entry and icon.
    #[cfg(target_os = "linux")]
    let _ = slint::set_xdg_app_id("io.github.heast13.kyrr");
    let _ = slint::select_bundled_translation(language.code());

    // Dictation worker, reporting back on the UI thread. Whether it records
    // is also kept here, for the tap-to-toggle key.
    let recording = Arc::new(AtomicBool::new(false));
    let key_toggle = Arc::new(AtomicBool::new(false));
    let memo_toggle = Arc::new(AtomicBool::new(config.memo_toggle));
    let weak = window.as_weak();
    let recording_flag = Arc::clone(&recording);
    // A voice memo records: dictation waits, and the wave is the memo's.
    let memo_active = Arc::new(AtomicBool::new(false));
    let wave = Overlay::spawn(platform::open_wave, recorder::tail);
    let memo_wave = wave.clone();
    let memo_flag = Arc::clone(&memo_active);
    let notify: dictation::Notify = Arc::new(move |event| {
        match event {
            Event::Heard(words) => return wave.set_words(words),
            Event::Ends(at) => return wave.set_end(at),
            Event::Limit(text) => return wave.set_limit(text),
            Event::Nothing(text) => return wave.set_note(text),
            Event::Note(text, tone) => return wave.set_toned(text, tone),
            _ => {}
        }
        if let Event::State(s) = &event {
            recording_flag.store(*s == State::Recording, Ordering::Relaxed);
            if !memo_flag.load(Ordering::Relaxed) {
                wave.set(match s {
                    State::Recording => Mode::Listening,
                    State::Transcribing => Mode::Writing,
                    _ => Mode::Hidden,
                });
            }
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
                    // While the model loads, the key records all the same.
                    platform::set_hotkey_active(matches!(
                        s,
                        State::Ready | State::Loading | State::Recording | State::Transcribing
                    ));
                    // A download just finished, or the model went.
                    if let Some(m) = memos::memos() {
                        m.set_can_write(model_installed(&Config::load()));
                    }
                }
                Event::Progress(p) => state.set_download_progress(p),
                Event::Typed(text) => state.set_last_text(text.into()),
                Event::Heard(_) | Event::Ends(_) | Event::Limit(_) | Event::Nothing(_) | Event::Note(..) => {}
                Event::Message(m) => state.set_dictation_message(m.unwrap_or_default().into()),
                Event::Titled(path, title) => {
                    if let Some(m) = memos::memos() {
                        m.titled(path, title);
                    }
                }
                Event::Text(path, text) => {
                    if let Some(m) = memos::memos() {
                        m.text(path, text);
                    }
                }
            }
        });
    });
    let worker = dictation::spawn(
        config::models_dir(),
        language,
        config.more_languages(),
        config.dictation,
        config.dictation_live,
        notify,
        platform::paste_text,
        platform::copy_to_clipboard,
    );

    // Hold mode: press starts, release stops. Tap mode: each press toggles.
    let description = match language {
        Language::German => "kyrr: Diktieren (Taste halten)",
        Language::English => "kyrr: Dictate (hold the key)",
    };
    // Both keys before the start: the desktop's dialog asks for them at once.
    platform::set_hotkey_key(config.dictation_key());
    platform::set_hotkey_tap(config.dictation_toggle);
    platform::set_memo_hold(!config.memo_toggle);
    let memo_description = match language {
        Language::German => MEMO_DESCRIPTION.0,
        Language::English => MEMO_DESCRIPTION.1,
    };
    platform::set_memo_key(memo_key_now(&config), memo_description);
    if let Some(keys) = platform::start_hotkey(description, config.setup_done) {
        let worker = worker.clone();
        let (recording, key_toggle) = (Arc::clone(&recording), Arc::clone(&key_toggle));
        let (memo_active, memo_toggle) = (Arc::clone(&memo_active), Arc::clone(&memo_toggle));
        std::thread::spawn(move || {
            // The dictation key is down and its press went to the worker,
            // which may not be recording yet: its release goes too.
            let mut held = false;
            // A memo the key started while held: its release ends it.
            let mut memo_held = false;
            while let Ok(key) = keys.recv() {
                match key {
                    platform::KeyEvent::MemoPressed => {
                        // Not in the middle of a dictation.
                        if !held && !recording.load(Ordering::Relaxed) {
                            // Held, a press starts a memo and the release
                            // ends it; one that records already (started
                            // by the button, or by a key that cannot tell
                            // its release) ends with the press.
                            memo_held = !memo_toggle.load(Ordering::Relaxed) && !memo_active.load(Ordering::Relaxed);
                            let _ = slint::invoke_from_event_loop(|| {
                                if let Some(m) = memos::memos() {
                                    m.toggle();
                                }
                            });
                        }
                        continue;
                    }
                    platform::KeyEvent::MemoReleased => {
                        if std::mem::take(&mut memo_held) {
                            let _ = slint::invoke_from_event_loop(|| {
                                if let Some(m) = memos::memos() {
                                    m.end();
                                }
                            });
                        }
                        continue;
                    }
                    _ => {}
                }
                let down = matches!(key, platform::KeyEvent::Pressed);
                // While a memo records, the dictation key does nothing.
                if memo_active.load(Ordering::Relaxed) && (down || !held) {
                    continue;
                }
                let toggle = key_toggle.load(Ordering::Relaxed);
                held = down && !toggle;
                // A worker that is gone (quitting) takes the dictation key
                // along, not the memo key.
                if worker.send(Command::Held(down)).is_err() {
                    continue;
                }
                let cmd = match (key, toggle) {
                    (platform::KeyEvent::Pressed, false) => Command::KeyDown(Instant::now(), false),
                    (platform::KeyEvent::Released, false) => Command::KeyUp(Instant::now()),
                    (platform::KeyEvent::Pressed, true) if recording.load(Ordering::Relaxed) => {
                        Command::KeyUp(Instant::now())
                    }
                    (platform::KeyEvent::Pressed, true) => Command::KeyDown(Instant::now(), true),
                    (platform::KeyEvent::Released, true)
                    | (platform::KeyEvent::MemoPressed | platform::KeyEvent::MemoReleased, _) => continue,
                };
                let _ = worker.send(cmd);
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
        widget_open: Cell::new(None),
        widget_told: Cell::new(None),
        guide: slint::Timer::default(),
        heartbeat: Cell::new(0),
        ticks: Cell::new(0),
        alert_turn: Cell::new(0),
        silent_ticks: Cell::new(0),
        model_changed: Cell::new(None),
        too_slow: Cell::new(false),
        overload_told: Cell::new(false),
        probe_events: mpsc::channel(),
        take: RefCell::new(None),
        listening: RefCell::new(None),
        news_window: RefCell::new(None),
        key_toggle,
        memo_toggle,
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
    let memos = memos::Memos::install(
        &window,
        worker.clone(),
        memo_wave,
        language,
        memo_active,
        Arc::clone(&recording),
        app.config.borrow().memo_text_auto,
    );
    memos.set_can_write(model_installed(&app.config.borrow()));
    memos.set_enabled(app.config.borrow().memos);
    memos.set_stamps(i32::from(app.config.borrow().memo_stamps));
    {
        // Memos from before this version's first start are not "new".
        let mut config = app.config.borrow_mut();
        if config.memos_seen == 0 {
            config.memos_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_secs());
            config.save();
        }
    }
    memos.set_seen(app.config.borrow().memos_seen, {
        let app = Rc::clone(&app);
        move |seconds| {
            let mut config = app.config.borrow_mut();
            config.memos_seen = seconds;
            config.save();
        }
    });

    let state = window.global::<AppState>();
    state.set_version(env!("CARGO_PKG_VERSION").into());
    state.set_language_index(app.config.borrow().language_index());
    state.set_download_size(megabytes(&app.config.borrow()).into());
    state.set_dictation_enabled(app.config.borrow().dictation);
    state.set_strength_index(strength_index(filter.limit_db));
    state.set_autostart(platform::autostart_enabled());
    state.set_update_auto(app.config.borrow().update_check);
    state.set_linux(cfg!(target_os = "linux"));
    state.set_tray(platform::has_tray());
    state.set_can_minimize(platform::can_minimize());
    state.set_live_words(app.config.borrow().dictation_live);
    app.show_effort();
    app.apply_key();
    keycapture::install(window.window(), {
        let app = Rc::clone(&app);
        Some(Box::new(move |heard| app.key_heard(heard)))
    });
    state.set_memos_enabled(app.config.borrow().memos);
    state.set_memo_auto_text(app.config.borrow().memo_text_auto);
    servers::setup(&app, &window);
    tray.set_memos_enabled(app.config.borrow().memos);
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
            // One download at a time: the tray offers it during one too.
            let busy = weak.upgrade().is_some_and(|w| {
                matches!(
                    w.global::<AppState>().get_update_state(),
                    UpdateState::Downloading | UpdateState::Installing
                )
            });
            if busy {
                return;
            }
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
                // The installer ends kyrr without asking: a memo being
                // recorded is closed first, and typed text saved.
                let (closed, done) = mpsc::channel();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(m) = memos::memos() {
                        m.end();
                        m.flush();
                    }
                    let _ = closed.send(());
                });
                let _ = done.recv_timeout(Duration::from_secs(5));
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
    state.on_send_feedback({
        let app = Rc::clone(&app);
        move || {
            let system = platform::system_info();
            platform::open_url(&update::feedback_mailto(
                app.language(),
                env!("CARGO_PKG_VERSION"),
                &system,
            ));
        }
    });
    state.on_open_update({
        let app = Rc::clone(&app);
        move || platform::open_url(update::download_page(app.language()))
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
    state.on_capture_key({
        let app = Rc::clone(&app);
        move |slot| app.capture(slot)
    });
    state.on_key_reset({
        let app = Rc::clone(&app);
        move |slot| {
            app.set_key(
                slot,
                if slot == 1 {
                    keys::DICTATION_DEFAULT
                } else {
                    keys::MEMO_DEFAULT
                },
            )
        }
    });
    state.on_memos_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_memos(on)
    });
    state.on_memo_toggle_changed({
        let app = Rc::clone(&app);
        move |on| {
            {
                let mut config = app.config.borrow_mut();
                config.memo_toggle = on;
                config.save();
            }
            app.apply_memo_key();
        }
    });
    state.on_memo_sheet_stamps_changed({
        let app = Rc::clone(&app);
        move |i| {
            {
                let mut config = app.config.borrow_mut();
                config.memo_stamps = i.clamp(0, 2) as u8;
                config.save();
            }
            if let Some(m) = memos::memos() {
                m.stamps_changed(i);
            }
        }
    });
    state.on_memo_auto_text_toggled({
        let app = Rc::clone(&app);
        move |on| {
            {
                let mut config = app.config.borrow_mut();
                config.memo_text_auto = on;
                config.save();
            }
            if let Some(m) = memos::memos() {
                m.set_auto_text(on);
            }
        }
    });
    state.on_memo_open_list({
        let weak = window.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            if let Some(m) = memos::memos() {
                m.refresh();
                m.mark_seen();
            }
            let state = w.global::<AppState>();
            state.set_memo_sheet_open(false);
            state.set_page(6);
            state.invoke_page_changed();
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
    state.on_live_words_toggled({
        let app = Rc::clone(&app);
        move |on| {
            {
                let mut config = app.config.borrow_mut();
                config.dictation_live = on;
                config.save();
            }
            let _ = app.worker.send(Command::Live(on));
            app.show_effort();
        }
    });
    state.on_effort_changed({
        let app = Rc::clone(&app);
        move |i| {
            if i < 2 {
                // Picked by hand: the choice for an old processor is made.
                {
                    let mut config = app.config.borrow_mut();
                    config.lean_suggested = true;
                    config.save();
                }
                app.set_effort(i == 0);
                if let Some(w) = app.window.upgrade() {
                    w.global::<AppState>().set_overload_note(OverloadNote::None);
                }
            }
        }
    });
    state.set_auto_lean(app.config.borrow().auto_lean);
    state.on_auto_lean_toggled({
        let app = Rc::clone(&app);
        move |on| {
            let mut config = app.config.borrow_mut();
            config.auto_lean = on;
            config.save();
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
    state.on_next_setup({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.set_setup_step(2);
                w.invoke_reset_scroll();
                fit(&weak);
            }
        }
    });
    state.on_page_changed({
        let weak = window.as_weak();
        let app = Rc::clone(&app);
        move || {
            // A key field that waited is gone with its page.
            app.capture(0);
            servers::look_around(&app);
            if let Some(w) = weak.upgrade() {
                w.invoke_reset_scroll();
            }
            fit(&weak);
        }
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
    // A memo compared with the text before widens the window for a second
    // column, and it narrows again after; the height stays.
    window.on_page_width_changed({
        let weak = window.as_weak();
        move || {
            let weak = weak.clone();
            slint::Timer::single_shot(Duration::from_millis(10), move || {
                let Some(w) = weak.upgrade() else { return };
                let win = w.window();
                if headless() || !win.is_visible() || win.is_minimized() {
                    return;
                }
                resize(&w, win.size().to_logical(win.scale_factor()).height);
            });
        }
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
    state.on_widget_open({
        let app = Rc::clone(&app);
        move |open| app.widget_looks(open)
    });
    state.on_omarchy_command({
        let app = Rc::clone(&app);
        move |line| app.omarchy_command(&line)
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
    state.on_distance_toggled({
        let app = Rc::clone(&app);
        move |on| app.set_voice(|v| v.distance = on)
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
        move |i| app.set_language(i)
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
            app.apply_memo_key();
            // With the widget in Omarchy's bar, the widget starts kyrr.
            if !headless() {
                platform::enable_autostart();
            }
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
            platform::tend_filter();
            // The microphones first: the filter's state is read against them.
            app.refresh_microphones();
            app.refresh_filter();
            app.check_mute();
            app.follow_default();
            app.refresh_key_note();
            app.widget_still_looks();
            if let Some(w) = app.window.upgrade() {
                follow_widget(&w);
                #[cfg(target_os = "linux")]
                {
                    skin::refresh(&w);
                    tell_widget(&w);
                }
            }
        }
    });
    if let Some(requests) = show_requests {
        let weak = window.as_weak();
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = weak.upgrade() {
                        answer(&w, request);
                    }
                });
            }
        });
    }

    // First visible start after an update (an existing install that last ran
    // another version): say what the new version brings, once. A start in the
    // background (Windows login) leaves it for the next visible one.
    let updated = !background && !headless() && {
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
    if headless() {
        // The widget's menu stands in for the window, the first-run setup
        // included: it opens once the widget is there (see follow_widget).
        OPEN_WHEN_SEEN.set(!setup_done || !background);
    } else if !setup_done || !background {
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
    // Without a tray host Slint would only log a complaint: wait for one
    // (the retry below) instead of shouting at start. One that is there but
    // takes no icon is waited for the same way, not the end of kyrr.
    // kyrr's widget in Omarchy's bar has the tray menu: no icon besides it.
    let widget = platform::widget_shown() || headless();
    let tray_at_start =
        !widget && platform::has_tray() && tray.show().map_err(|e| eprintln!("kyrr: tray: {e}")).is_ok();
    // One lookup for both, so a host that turns up in between cannot leave
    // the window saying "no tray" next to a visible icon.
    window.global::<AppState>().set_tray(tray_at_start);
    window.global::<AppState>().set_widget(widget);
    if widget {
        // A kyrr started at sign-in gives the shell a minute to come up.
        let grace = if headless() { WIDGET_GONE } else { Duration::ZERO };
        WIDGET_SEEN.set(Some(Instant::now() + grace));
    }
    drop(tray);
    #[cfg(target_os = "linux")]
    tell_widget(&window);
    // A panel that hosts tray icons may start after kyrr (both at login).
    // An icon can only register once one is there: make a new one then.
    let tray_retry = slint::Timer::default();
    if !tray_at_start && !headless() {
        let app = Rc::downgrade(&app);
        let started = Instant::now();
        let done = Cell::new(false);
        tray_retry.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || {
            if done.get()
                || started.elapsed() > Duration::from_secs(120)
                || !platform::has_tray()
                || platform::widget_shown()
            {
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
    // A memo being recorded ends properly, whichever way kyrr quits.
    if let Some(m) = memos::memos() {
        m.finish();
    }
    TRAY.take();
    result
}

/// Does what another start of kyrr or kyrr's widget in Omarchy's bar asks
/// for: what the tray menu does.
fn answer(window: &MainWindow, request: Request) {
    match (request, tray()) {
        (Request::Show, _) => show(window),
        (_, None) => {}
        (Request::ToggleFilter, Some(tray)) => tray.invoke_toggle_filter(),
        (Request::ToggleVoice, Some(tray)) => tray.invoke_toggle_voice(),
        (Request::ToggleDictation, Some(tray)) => tray.invoke_toggle_dictation(),
        // The switch on the main page; the tray menu has none.
        (Request::ToggleMemos, Some(tray)) => window
            .global::<AppState>()
            .invoke_memos_toggled(!tray.get_memos_enabled()),
        (Request::ToggleMemo, Some(tray)) => tray.invoke_toggle_memo(),
        (Request::OpenMemos, Some(tray)) => tray.invoke_open_memos(),
        (Request::InstallUpdate, Some(tray)) => tray.invoke_install_update(),
        (Request::WidgetOpen(open), Some(_)) => window.global::<AppState>().invoke_widget_open(open),
        (Request::OpenPage(page), Some(_)) => {
            show(window);
            window.global::<AppState>().invoke_open_page(page);
        }
        (Request::Quit, Some(tray)) => tray.invoke_quit(),
        (Request::Line(line), Some(_)) => window.global::<AppState>().invoke_omarchy_command(line.into()),
    }
    #[cfg(target_os = "linux")]
    tell_widget(window);
}

thread_local! {
    /// When kyrr last saw its widget in Omarchy's bar.
    static WIDGET_SEEN: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Whether it saw the widget at all since it started.
    static EVER_SEEN: Cell<bool> = const { Cell::new(false) };
    /// Open the widget's menu once the widget is there: the first-run setup,
    /// or kyrr started from the app menu.
    static OPEN_WHEN_SEEN: Cell<bool> = const { Cell::new(false) };
}

/// How long the widget may be away before kyrr counts it as turned off.
const WIDGET_GONE: Duration = Duration::from_secs(30);

/// How often the meter's ticks tell the open menu the state.
#[cfg(target_os = "linux")]
const WIDGET_EVERY: Duration = Duration::from_millis(300);

/// kyrr's widget came into Omarchy's bar or left it. Its menu takes the
/// tray icon's place, and the window needs no buttons to hide or quit.
/// Turning the plugin off in Omarchy's menu turns kyrr off with it.
fn follow_widget(window: &MainWindow) {
    let shown = platform::widget_shown();
    if shown {
        WIDGET_SEEN.set(Some(Instant::now()));
    }
    let state = window.global::<AppState>();
    let Some(tray) = tray() else {
        return;
    };
    if headless() {
        if shown {
            if !EVER_SEEN.replace(true) {
                // Give the widget a moment to read the state file.
                if OPEN_WHEN_SEEN.replace(false) {
                    let weak = window.as_weak();
                    slint::Timer::single_shot(Duration::from_secs(1), move || {
                        if let Some(w) = weak.upgrade() {
                            show(&w);
                        }
                    });
                }
            }
            return;
        }
        if !WIDGET_SEEN.get().is_some_and(|seen| seen.elapsed() > WIDGET_GONE) {
            return;
        }
        if !EVER_SEEN.get() {
            // The plugin is on but its widget never came (the shell broke,
            // say): kyrr as everywhere else, rather than nowhere.
            eprintln!("kyrr: Omarchy's bar shows no kyrr widget, so kyrr opens its window");
            HEADLESS.set(false);
            state.set_widget(false);
            // An icon only where a tray host takes it, as at a normal start.
            let host = platform::has_tray();
            if host {
                tray.set_off(false);
            }
            state.set_tray(host && tray.show().is_ok());
            show(window);
            return;
        }
        // Without the widget kyrr has nothing to show itself in: turning the
        // plugin off turns kyrr off, once nothing is being recorded.
        let busy = memos::memos().is_some_and(|m| m.recording())
            || matches!(
                state.get_dictation_state(),
                DictationState::Recording | DictationState::Transcribing
            );
        if !busy {
            tray.invoke_quit();
        }
        return;
    }
    if state.get_widget() == shown {
        return;
    }
    if shown {
        state.set_widget(true);
        state.set_tray(false);
        let _ = tray.hide();
        return;
    }
    // Gone by kyrr's own clock, which stands still while the computer
    // sleeps: after waking up the widget gets its moment to renew the file.
    if WIDGET_SEEN.get().is_some_and(|seen| seen.elapsed() > WIDGET_GONE) {
        tray.invoke_quit();
    }
}

/// What this version brings, read once from the NEWS.md built in.
#[cfg(target_os = "linux")]
fn bundled_news() -> &'static Option<update::News> {
    static NEWS: std::sync::OnceLock<Option<update::News>> = std::sync::OnceLock::new();
    NEWS.get_or_init(|| update::bundled_news(env!("CARGO_PKG_VERSION")))
}

/// Everything the menu of kyrr's widget in Omarchy's bar shows, which is all
/// of kyrr there: the format is in the widget's README ("State").
#[cfg(target_os = "linux")]
fn tell_widget(window: &MainWindow) {
    use serde_json::json;
    let Some(tray) = tray() else {
        return;
    };
    let state = window.global::<AppState>();
    // Nobody to tell.
    if !state.get_widget() {
        return;
    }
    let pick = |names: &[&'static str], i: i32| names[usize::try_from(i).unwrap_or(0).min(names.len() - 1)];
    let german = state.get_language_index() == 0;
    let model = match state.get_dictation_state() {
        DictationState::Off => "off",
        DictationState::NoModel => "none",
        DictationState::Downloading => "downloading",
        DictationState::Loading => "loading",
        DictationState::Error => "error",
        _ => "ready",
    };
    let update = match state.get_update_state() {
        UpdateState::Checking => "checking",
        UpdateState::Current => "latest",
        UpdateState::Available => "available",
        UpdateState::Failed | UpdateState::DownloadFailed => "failed",
        UpdateState::Downloading | UpdateState::Installing => "downloading",
        UpdateState::Idle => "idle",
    };
    let setting_up = match state.get_filter_state() {
        FilterState::SettingUp => "settingUp",
        FilterState::Error => "error",
        FilterState::Unsupported => "unsupported",
        _ => "",
    };
    let probe = match state.get_probe_state() {
        ProbeState::Idle => "idle",
        ProbeState::Recording => "recording",
        ProbeState::Before | ProbeState::After => "playing",
    };
    let news = |news: Option<update::News>| news.map(|n| if german { n.de } else { n.en }).unwrap_or_default();
    let microphones: Vec<serde_json::Value> = state
        .get_microphones()
        .iter()
        .enumerate()
        .map(|(id, name)| json!({ "id": id, "name": name.as_str() }))
        .collect();
    let key_note = [state.get_key_refusal(), state.get_key_warning(), state.get_key_note()]
        .into_iter()
        .find(|n| !n.is_empty())
        .unwrap_or_default();
    // The little wave next to the status line, as in the window: busy,
    // or on and waiting for an app to use the microphone.
    let filter_state = state.get_filter_state();
    let working = state.get_mic_active()
        || state.get_probe_state() != ProbeState::Idle
        || state.get_model_switching()
        || matches!(
            state.get_dictation_state(),
            DictationState::Recording | DictationState::Transcribing | DictationState::Downloading
        )
        || filter_state == FilterState::SettingUp
        || state.get_memo_recording();
    let waiting = (state.get_filter_enabled() || state.get_voice_enabled())
        && matches!(filter_state, FilterState::Running | FilterState::Checking);
    let memos = memos::memos();
    // The recording whose text the menu shows: the text itself goes to a
    // file of its own, so the state stays small while it plays.
    let sheet_id = memos.as_ref().map(|m| m.sheet_id()).unwrap_or_default();
    let sheet_open = state.get_memo_sheet_open() && !sheet_id.is_empty();
    let sheet_text = state.get_memo_sheet_text();
    kyrr_linux::omarchy::set_sheet(sheet_open.then_some((sheet_id.as_str(), sheet_text.as_str())));
    let comparing = sheet_open
        && state.get_memo_sheet_view() == 1
        && state.get_memo_sheet_original()
        && !state.get_memo_rework_busy();
    let compare = memos.as_ref().filter(|_| comparing).and_then(|m| m.widget_compare());
    kyrr_linux::omarchy::set_compare(compare.as_deref().map(|json| (sheet_id.as_str(), json)));
    // The instruction the menu's Local AI page shows for its task.
    let prompt_saved = state.get_prompt_saved();
    kyrr_linux::omarchy::set_prompt(
        state
            .get_ai_features()
            .then(|| (state.get_prompt_index().clamp(0, 3) as usize, prompt_saved.as_str())),
    );
    let sheet = if sheet_open {
        json!({
            "id": sheet_id,
            "title": state.get_memo_sheet_title().as_str(),
            "when": state.get_memo_sheet_when().as_str(),
            "stamps": state.get_memo_sheet_stamps(),
            "view": state.get_memo_sheet_view(),
            "saved": state.get_memo_sheet_saved(),
            "copied": state.get_memo_copied(),
            "empty": sheet_text.trim().is_empty(),
            "here": state
                .invoke_memo_here_label(sheet_text.clone(), SHEET_CURSOR.get())
                .as_str(),
            "rework": servers::widget_memo_rework(&state),
        })
    } else {
        serde_json::Value::Null
    };
    let status = json!({
        "lang": if german { "de" } else { "en" },
        "version": env!("CARGO_PKG_VERSION"),
        "line": state.invoke_status_line().as_str(),
        "note": state.invoke_alert_note().as_str(),
        // The red notes are problems; a microphone coming back is good news.
        "noteGood": state.get_alert() == 7,
        "alert": state.get_alert(),
        "overloadNote": match state.get_overload_note() {
            OverloadNote::None => "",
            OverloadNote::Switched => "switched",
            OverloadNote::Pauses => "pauses",
        },
        "oldCpu": state.get_old_cpu(),
        // The meters on the Setup page, moving while the menu is open.
        "levels": {
            "active": state.get_mic_active(),
            "before": (state.get_level_before() * 50.0).round() / 50.0,
            "after": (state.get_level_after() * 50.0).round() / 50.0,
        },
        "pulse": if working { "working" } else if waiting { "waiting" } else { "" },
        "setupDone": !window.get_setup_mode(),
        "setupStep": window.get_setup_step() + 1,
        "filter": {
            "on": tray.get_filter_enabled(),
            "strength": state.get_strength_index(),
            "setup": setting_up,
            // Improving the voice needs the filter on a microphone.
            "attached": !matches!(
                state.get_filter_state(),
                FilterState::NotAttached | FilterState::NotInstalled | FilterState::Unsupported
            ),
            "error": state.get_error_text().as_str(),
            "state": match filter_state {
                FilterState::Checking => "checking",
                FilterState::Running => "running",
                FilterState::Off => "off",
                FilterState::Passthrough => "passthrough",
                FilterState::NotAttached => "notAttached",
                FilterState::NotInstalled => "notInstalled",
                FilterState::SettingUp => "settingUp",
                FilterState::Error => "error",
                FilterState::Detached => "detached",
                FilterState::Unsupported => "unsupported",
            },
            "mic": state.get_attached_mic().as_str(),
            "moveTo": state.get_move_to().as_str(),
            "moveUnplugged": state.get_move_unplugged(),
        },
        "voice": {
            "on": tray.get_voice_enabled(),
            "tone": pick(&["neutral", "warm", "clear", "bright", "custom"], state.get_look_index()),
            "body": state.get_body_db().round(),
            "presence": state.get_presence_db().round(),
            "air": state.get_air_db().round(),
            "comp": state.get_compressor_index(),
            "ess": state.get_deesser_index(),
            "rumble": state.get_low_cut(),
            "rumbleHz": if state.get_look_index() == 3 { 100 } else { 80 },
            "pops": state.get_pops(),
            "distance": state.get_distance(),
            "autoLevel": state.get_auto_level(),
            "levelDb": state.get_auto_level().then(|| (state.get_level_gain() * 10.0).round() / 10.0),
            "isDefault": state.get_voice_is_default(),
        },
        "dictation": {
            "on": tray.get_dictation_enabled(),
            "ready": tray.get_dictation_ready(),
            "key": tray.get_key_name().as_str(),
            "hold": !state.get_key_toggle(),
            "note": key_note.as_str(),
            "state": match state.get_dictation_state() {
                DictationState::Off => "off",
                DictationState::NoModel => "noModel",
                DictationState::Downloading => "downloading",
                DictationState::Loading => "loading",
                DictationState::Ready => "ready",
                DictationState::Recording => "recording",
                DictationState::Transcribing => "transcribing",
                DictationState::Error => "error",
            },
            "message": state.get_dictation_message().as_str(),
            "keyCustom": state.get_key_custom(),
            "keyFixed": state.get_key_fixed(),
            "keyUnbound": state.get_key_unbound(),
            "keyButton": state.get_key_button().as_str(),
        },
        "memos": {
            "on": tray.get_memos_enabled(),
            "recording": tray.get_memo_recording(),
            "key": tray.get_memo_key_name().as_str(),
            "hold": !state.get_memo_toggle(),
            "autoText": state.get_memo_auto_text(),
            "folder": memos.as_ref().map(|m| m.widget_folder()).unwrap_or_default(),
            "message": state.get_memo_message().as_str(),
            "elapsed": state.get_memo_elapsed().as_str(),
            "canWrite": state.get_memo_can_write(),
            "canHold": state.get_memo_can_hold(),
            "keyNote": state.get_memo_key_note().as_str(),
            "keyCustom": state.get_memo_key_custom(),
            "query": state.get_memo_query().as_str(),
            "count": state.get_memo_count(),
            "playTime": state.get_memo_play_time().as_str(),
            "playFraction": state.get_memo_play_fraction(),
            "list": memos.as_ref().map(|m| m.widget_list()).unwrap_or_else(|| json!([])),
            "sheet": sheet,
        },
        "general": {
            "language": pick(&["de", "en", "multi"], state.get_language_index()),
            "effort": pick(&["lean", "best", "custom"], state.get_effort_index()),
            "liveText": state.get_live_words(),
            "overload": state.get_auto_lean(),
            "updateCheck": state.get_update_auto(),
        },
        "mic": {
            "name": state.get_attached_mic().as_str(),
            // The one in use, in the list (-1: none of them).
            "current": state
                .get_microphones()
                .iter()
                .position(|name| name == state.get_attached_mic())
                .map_or(-1, |i| i as i64),
            "index": state.get_mic_index(),
            "list": microphones,
        },
        "model": {
            "state": model,
            "progress": state.get_download_progress(),
            "size": state.get_download_size().as_str(),
        },
        "update": {
            "state": update,
            "version": tray.get_update_version().as_str(),
            "progress": state.get_update_progress(),
            "news": news(pending_news().lock().unwrap_or_else(|e| e.into_inner()).clone()),
            "phase": match state.get_update_state() {
                UpdateState::Installing => "installing",
                UpdateState::DownloadFailed => "downloadFailed",
                _ => "",
            },
            "installable": state.get_update_installable(),
            "dismissed": state.get_update_dismissed(),
        },
        // Local AI and the own speech server (kyrr 0.8.0).
        "ai": servers::widget_state(&state),
        "probe": probe,
        "hasProbe": state.get_has_probe(),
        "probeError": state.get_probe_error().as_str(),
        "listen": state.get_listening(),
        "news": news(bundled_news().clone()),
    });
    kyrr_linux::omarchy::set_status(status.to_string());
}

/// A tray icon wired to the app.
fn new_tray(app: &Rc<App>) -> Result<Tray, slint::PlatformError> {
    let tray = Tray::new()?;
    tray.set_off(headless());
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
    tray.on_toggle_memo(|| {
        if let Some(m) = memos::memos() {
            m.toggle();
        }
    });
    tray.on_open_memos({
        let weak = app.window.clone();
        move || {
            if let Some(w) = weak.upgrade() {
                show(&w);
                w.global::<AppState>().invoke_memo_open_list();
            }
        }
    });
    servers::wire_tray(app, &tray);
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
        new.set_memos_enabled(old.get_memos_enabled());
        new.set_memo_recording(old.get_memo_recording());
        new.set_memo_key_name(old.get_memo_key_name());
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
    fn every_status_list_has_the_same_length() {
        let ui = include_str!("../ui/app.slint");
        let mut lists = 0;
        for chunk in ui.split("pick([").skip(1) {
            let list = chunk.split("])").next().unwrap();
            assert_eq!(list.matches("@tr(").count() as i32, PHRASES, "{list}");
            lists += 1;
        }
        assert_eq!(lists, 10);
    }

    #[test]
    fn update_check_timing() {
        let day = UPDATE_INTERVAL.as_secs();
        let t = 1_800_000_000;
        assert!(!update_due(t, t + 60, false));
        assert!(update_due(t, t + day, false));
        // Failed: again after an hour, not a day.
        assert!(!update_due(t, t + 60, true));
        assert!(update_due(t, t + UPDATE_RETRY.as_secs(), true));
        // A check stamped in the future (wrong clock back then).
        assert!(update_due(t + 10 * day, t, false));
        assert!(update_due(0, t, false));
    }

    #[test]
    fn no_notes_when_all_is_well() {
        assert!(alerts(true, true, 8.0, false, false, false, false, false).is_empty());
        assert!(alerts(false, false, 0.0, false, false, false, false, false).is_empty());
    }

    #[test]
    fn gain_at_its_limit_says_so() {
        assert_eq!(
            alerts(true, true, 24.0, false, false, false, false, false),
            [Alert::MicQuiet]
        );
        assert_eq!(
            alerts(true, true, -12.0, false, false, false, false, false),
            [Alert::MicLoud]
        );
        // Just short of the limits is still fine.
        assert!(alerts(true, true, 23.0, false, false, false, false, false).is_empty());
        assert!(alerts(true, true, -11.0, false, false, false, false, false).is_empty());
    }

    #[test]
    fn gain_only_counts_while_it_works() {
        // Automatic volume off, or no app on the microphone: the number is stale.
        assert!(alerts(false, true, 24.0, false, false, false, false, false).is_empty());
        assert!(alerts(true, false, -12.0, false, false, false, false, false).is_empty());
    }

    #[test]
    fn several_notes_come_in_turn() {
        assert_eq!(
            alerts(true, true, 24.0, false, true, false, false, false),
            [Alert::MicQuiet, Alert::Listening]
        );
        // Muted outweighs everything else, even with no app on the microphone.
        assert_eq!(
            alerts(true, true, 24.0, false, true, true, false, false),
            [Alert::Muted]
        );
        assert_eq!(
            alerts(false, false, 0.0, false, false, true, false, false),
            [Alert::Muted]
        );
        // No microphone at all, and one that just came back.
        assert_eq!(
            alerts(true, false, 0.0, false, true, false, true, false),
            [Alert::NoMic]
        );
        assert_eq!(
            alerts(false, false, 0.0, false, false, false, false, true),
            [Alert::Connected]
        );
        assert_eq!(
            alerts(false, false, 0.0, false, true, false, false, false),
            [Alert::Listening]
        );
    }

    #[test]
    fn far_mic_says_so_while_in_use() {
        assert_eq!(
            alerts(true, true, 8.0, true, false, false, false, false),
            [Alert::MicFar]
        );
        assert_eq!(
            alerts(false, true, 0.0, true, true, false, false, false),
            [Alert::MicFar, Alert::Listening]
        );
        // No app on the microphone: an old verdict.
        assert!(alerts(true, false, 8.0, true, false, false, false, false).is_empty());
    }
}
