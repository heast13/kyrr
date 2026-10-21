//! Renders the window headless into raw RGBA files for checking the layout
//! without a display: `cargo run --example preview -- <out-dir> [de|en] [scale] [dark]`.
//! The README shots are `en` in `dark`.
//! The scale defaults to 1.5 (the README shots); the social preview's window
//! is 1.2711 (483 px wide).
//! Convert with ImageMagick: `magick -size WxH -depth 8 rgba:main.rgba main.png`.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::include_modules!();

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const DEFAULT_SCALE: f32 = 1.5;

fn render(window: &MinimalSoftwareWindow, ui: &MainWindow, path: &str) {
    let scale = ui.window().scale_factor();
    ui.show().unwrap();
    let w = (ui.get_page_width() * scale) as u32;
    window.set_size(PhysicalSize::new(w, 100));
    let h = (ui.get_content_height() * scale).ceil() as u32;
    window.set_size(PhysicalSize::new(w, h));
    let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w as usize);
    });
    let bytes: Vec<u8> = buf.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    std::fs::write(path, bytes).unwrap();
    std::fs::write(format!("{path}.size"), format!("{w}x{h}")).unwrap();
    ui.hide().unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| ".".into());
    let lang = args.get(2).cloned().unwrap_or_else(|| "de".into());
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
    let scale = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_SCALE);
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });

    let ui = MainWindow::new().unwrap();
    if args.iter().any(|a| a == "dark") {
        ui.invoke_use_dark_scheme();
    }
    if let Err(e) = slint::select_bundled_translation(&lang) {
        eprintln!("translation {lang}: {e}");
    }
    let state = ui.global::<AppState>();
    state.set_version(env!("CARGO_PKG_VERSION").into());
    // Windows names devices in its display language.
    let mic = if lang == "de" {
        "Mikrofon (Shure MV7)"
    } else {
        "Microphone (Shure MV7)"
    };
    state.set_language_index((lang != "de") as i32);
    state.set_microphones(ModelRc::new(VecModel::from(vec![
        mic.into(),
        "Headset (Logitech G Pro X)".into(),
    ])));
    state.set_attached_mic(mic.into());
    state.set_filter_state(FilterState::Running);
    state.set_dictation_state(DictationState::Ready);
    state.set_mic_active(true);
    state.set_level_before(0.72);
    state.set_level_after(0.55);
    state.set_download_size("880 MB".into());
    state.set_phrase(3);
    // Switched on early: a switch flipped right before a shot is caught
    // mid-animation.
    state.set_auto_level(true);
    state.set_level_gain(8.6);
    render(&window, &ui, &format!("{out}/main-{lang}.rgba"));

    // The red notes at the bottom: the mic is out of the automatic volume's
    // reach, and the live listening is still on.
    state.set_alert(1);
    render(&window, &ui, &format!("{out}/main-alert-quiet-{lang}.rgba"));
    state.set_alert(3);
    state.set_listening(true);
    render(&window, &ui, &format!("{out}/main-alert-listen-{lang}.rgba"));
    state.set_alert(0);
    state.set_listening(false);

    // A new version is out: the note above the bottom row.
    state.set_update_version("0.4.0".into());
    state.set_update_installable(true);
    state.set_update_has_news(true);
    state.set_update_state(UpdateState::Available);
    render(&window, &ui, &format!("{out}/main-update-{lang}.rgba"));
    state.set_update_state(UpdateState::Downloading);
    state.set_update_progress(0.37);
    render(&window, &ui, &format!("{out}/main-updating-{lang}.rgba"));
    state.set_update_state(UpdateState::Idle);
    state.set_update_has_news(false);

    // Nothing uses the microphone: standing by, with the meter at rest.
    state.set_mic_active(false);
    state.set_level_before(0.0);
    state.set_level_after(0.0);
    state.set_phrase(0);
    render(&window, &ui, &format!("{out}/standby-{lang}.rgba"));

    // All off.
    state.set_filter_enabled(false);
    state.set_voice_enabled(false);
    state.set_dictation_enabled(false);
    state.set_dictation_state(DictationState::Off);
    render(&window, &ui, &format!("{out}/alloff-{lang}.rgba"));

    // Speech model still to fetch.
    state.set_filter_enabled(true);
    state.set_voice_enabled(true);
    state.set_dictation_enabled(true);
    state.set_filter_state(FilterState::Detached);
    state.set_dictation_state(DictationState::Error);
    state.set_dictation_message("Kein Internet oder der Server antwortet nicht. Nochmal versuchen?".into());
    render(&window, &ui, &format!("{out}/problems-{lang}.rgba"));

    state.set_filter_state(FilterState::Running);
    state.set_dictation_message("".into());
    state.set_dictation_state(DictationState::Downloading);
    state.set_download_progress(0.42);
    state.set_phrase(2);
    render(&window, &ui, &format!("{out}/download-{lang}.rgba"));

    // Settings as most people see them: the basics.
    state.set_dictation_state(DictationState::Ready);
    state.set_page(1);
    state.set_probe_state(ProbeState::Idle);
    state.set_has_probe(true);
    state.set_mic_active(true);
    state.set_level_before(0.72);
    state.set_level_after(0.55);
    state.set_look_index(2);
    state.set_voice_is_default(false);
    render(&window, &ui, &format!("{out}/settings-{lang}.rgba"));
    // Listening to yourself: the button turns red.
    state.set_listening(true);
    render(&window, &ui, &format!("{out}/settings-listening-{lang}.rgba"));
    state.set_listening(false);

    // Everything, with the custom sound so the sliders show.
    state.set_advanced(true);
    state.set_live_words(true);
    state.set_look_index(4);
    state.set_body_db(1.5);
    state.set_presence_db(-2.0);
    state.set_deesser_index(2);
    render(&window, &ui, &format!("{out}/settings-advanced-{lang}.rgba"));
    state.set_effort_index(2);
    render(&window, &ui, &format!("{out}/settings-advanced-custom-{lang}.rgba"));
    state.set_effort_index(1);
    state.set_advanced(false);

    // Settings with voice off and the defaults.
    state.set_voice_enabled(false);
    state.set_look_index(0);
    state.set_voice_is_default(true);
    state.set_has_probe(false);
    state.set_update_state(UpdateState::Available);
    state.set_update_version("0.4.0".into());
    state.set_update_installable(true);
    render(&window, &ui, &format!("{out}/settings-off-{lang}.rgba"));
    state.set_update_state(UpdateState::Downloading);
    state.set_update_progress(0.37);
    render(&window, &ui, &format!("{out}/update-{lang}.rgba"));
    state.set_update_state(UpdateState::Idle);

    // Windows on ARM: no filter, dictation only.
    state.set_filter_enabled(false);
    state.set_voice_enabled(false);
    state.set_mic_active(false);
    state.set_filter_state(FilterState::Unsupported);
    render(&window, &ui, &format!("{out}/arm-settings-{lang}.rgba"));
    state.set_page(0);
    render(&window, &ui, &format!("{out}/arm-{lang}.rgba"));

    state.set_page(0);
    ui.set_setup_mode(true);
    state.set_filter_state(FilterState::NotAttached);
    render(&window, &ui, &format!("{out}/setup1-{lang}.rgba"));

    ui.set_setup_step(1);
    state.set_dictation_state(DictationState::Downloading);
    state.set_download_progress(0.42);
    render(&window, &ui, &format!("{out}/setup2-{lang}.rgba"));
    ui.set_setup_step(2);
    render(&window, &ui, &format!("{out}/setup3-{lang}.rgba"));
    state.set_effort_index(0);
    render(&window, &ui, &format!("{out}/setup3-lean-{lang}.rgba"));
    state.set_effort_index(1);

    // The summary window of a new version.
    let news = NewsWindow::new().unwrap();
    news.set_version("0.5.0".into());
    news.set_news(
        if lang == "de" {
            "• Das Hauptfenster behält jetzt seine Höhe.\n• Eine kleine rote Zeile warnt, wenn dein Mikro zu leise ist oder „Mithören“ noch läuft.\n• Die automatische Lautstärke zeigt +9 dB grün oder −9 dB rot."
        } else {
            "• The main window keeps its height now.\n• A small red note warns when your mic is too quiet or \"Listen to yourself\" is still running.\n• The automatic volume shows +9 dB in green or −9 dB in red."
        }
        .into(),
    );
    news.show().unwrap();
    let h = (320.0 * scale) as u32;
    let w = (380.0 * scale) as u32;
    window.set_size(PhysicalSize::new(w, h));
    let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w as usize);
    });
    let bytes: Vec<u8> = buf.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    let path = format!("{out}/news-{lang}.rgba");
    std::fs::write(&path, bytes).unwrap();
    std::fs::write(format!("{path}.size"), format!("{w}x{h}")).unwrap();
}
