//! Renders the window headless into raw RGBA files for checking the layout
//! without a display: `cargo run --example preview -- <out-dir> [de|en] [scale] [dark]
//! [omarchy=<colors.toml> [font=<family>]]`.
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

/// The window at a set height, after its animations ran.
fn render_at(window: &MinimalSoftwareWindow, ui: &MainWindow, height: f32, path: &str) {
    let scale = ui.window().scale_factor();
    ui.show().unwrap();
    let w = (ui.get_page_width() * scale) as u32;
    let h = (height * scale) as u32;
    window.set_size(PhysicalSize::new(w, h));
    std::thread::sleep(std::time::Duration::from_millis(450));
    slint::platform::update_timers_and_animations();
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

fn memos(window: &MinimalSoftwareWindow, ui: &MainWindow, out: &str, lang: &str) {
    let de = lang == "de";
    let state = ui.global::<AppState>();
    let t = |g: &str, e: &str| -> slint::SharedString {
        if de {
            g.into()
        } else {
            e.into()
        }
    };
    let item = |title: slint::SharedString, when: slint::SharedString, text_state: i32| MemoItem {
        title,
        when,
        pending: false,
        text_state,
        text_progress: 0.0,
        text_left: "".into(),
    };
    let mut writing = item(
        t("Gespräch mit dem Steuerberater", "Call with the tax adviser"),
        t("Gestern, 16:20 · 12:48", "Yesterday, 16:20 · 12:48"),
        2,
    );
    writing.text_progress = 0.42;
    writing.text_left = t("noch 3 Min.", "3 min left");
    let mut pending = item("".into(), t("Heute, 18:05 · 0:42", "Today, 18:05 · 0:42"), 0);
    pending.pending = true;
    let items = vec![
        pending,
        item(
            t("Einkaufsliste für Samstag", "Shopping list for Saturday"),
            t("Heute, 09:12 · 0:48", "Today, 09:12 · 0:48"),
            3,
        ),
        item(
            t("Gedanken zur neuen Startseite", "Thoughts on the new home page"),
            t("Gestern, 21:40 · 2:31", "Yesterday, 21:40 · 2:31"),
            0,
        ),
        writing,
        item(
            t("Test Test hört man mich", "Testing can you hear me"),
            "18.10.2026, 11:02 · 0:06".into(),
            0,
        ),
        item(
            t(
                "Besprechung mit dem Steuerberater über die Homeoffice Pauschale und alle Belege",
                "Meeting with the tax adviser about the home office allowance and all the receipts",
            ),
            "17.10.2026, 09:30 · 31:12".into(),
            3,
        ),
    ];
    state.set_memo_items(ModelRc::new(VecModel::from(items)));
    state.set_memo_count(6);
    state.set_memo_can_write(true);
    state.set_memo_playing(1);
    state.set_memo_play_fraction(0.35);
    state.set_memo_play_time("0:17 / 0:48".into());
    state.set_page(6);
    render(window, ui, &format!("{out}/memos-{lang}.rgba"));
    state.set_memo_playing(-1);
    state.set_memo_confirm(4);
    state.set_memo_recording(true);
    state.set_memo_elapsed("1:07".into());
    render(window, ui, &format!("{out}/memos-recording-{lang}.rgba"));
    state.set_memo_confirm(-1);
    state.set_memo_recording(false);

    let paragraphs: Vec<(&str, &str)> = if de {
        vec![
            (
                "0:00",
                "Gedanken zur neuen Startseite. Oben soll sofort klar sein, was kyrr macht, ohne Werbesprech.",
            ),
            (
                "0:34",
                "Darunter der Download, groß, und gleich daneben die Befehle für Linux zum Kopieren.",
            ),
            (
                "1:12",
                "Die Beispiel-Pille als kurze Schleife, nicht als Video, das lädt sonst ewig.",
            ),
            ("2:05", "Und unten nur noch die häufigen Fragen, zusammengeklappt."),
        ]
    } else {
        vec![
            ("0:00", "Thoughts on the new home page. At the top it should be clear right away what kyrr does, no marketing talk."),
            ("0:34", "Below that the download, big, and right next to it the Linux commands to copy."),
            ("1:12", "The example pill as a short loop, not as a video, that takes forever to load."),
            ("2:05", "And at the bottom just the common questions, folded away."),
        ]
    };
    // The field as it shows with the stamps: none, how far in, time of day
    // (this memo began at 21:40).
    let field = |stamps: i32| -> slint::SharedString {
        let minutes = [(21, 40), (21, 40), (21, 41), (21, 42)];
        paragraphs
            .iter()
            .zip(minutes)
            .map(|((time, text), (h, m))| match stamps {
                1 => format!("[{time}] {text}"),
                2 => format!("[{h}:{m:02}] {text}"),
                _ => (*text).to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n\n")
            .into()
    };
    state.set_memo_sheet_index(2);
    state.set_memo_sheet_title(t("Gedanken zur neuen Startseite", "Thoughts on the new home page"));
    state.set_memo_sheet_when(t("Gestern, 21:40 · 2:31", "Yesterday, 21:40 · 2:31"));
    state.set_memo_sheet_text(field(0));
    state.set_memo_sheet_stamps(0);
    state.set_memo_sheet_open(true);
    render_at(window, ui, 820.0, &format!("{out}/memos-text-{lang}.rgba"));
    for stamps in [1, 2] {
        state.set_memo_sheet_stamps(stamps);
        state.set_memo_sheet_text(field(stamps));
        render_at(
            window,
            ui,
            820.0,
            &format!("{out}/memos-text-stamps{stamps}-{lang}.rgba"),
        );
    }
    state.set_memo_sheet_stamps(0);
    state.set_memo_sheet_open(false);
    // Let the text slide away before the next pictures.
    std::thread::sleep(std::time::Duration::from_millis(450));
    slint::platform::update_timers_and_animations();
    state.set_memo_items(ModelRc::new(VecModel::from(Vec::<MemoItem>::new())));
    state.set_memo_count(0);
    render(window, ui, &format!("{out}/memos-empty-{lang}.rgba"));
    state.set_page(0);
    state.set_memo_count(5);
    state.set_memo_recording(true);
    render(window, ui, &format!("{out}/main-memo-recording-{lang}.rgba"));
    state.set_memo_recording(false);
    // A recording not looked at yet: the dot beside the button.
    state.set_memo_new(true);
    render(window, ui, &format!("{out}/main-memo-new-{lang}.rgba"));
    state.set_memo_new(false);
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
    // Omarchy's look: `omarchy=<theme's colors.toml>`, `font=<family>`.
    #[cfg(target_os = "linux")]
    if let Some(path) = args.iter().find_map(|a| a.strip_prefix("omarchy=")) {
        let text = std::fs::read_to_string(path).unwrap();
        let t = kyrr_linux::omarchy::parse_theme(&text, false).expect("a theme");
        let color = |rgb: [u8; 3]| slint::Color::from_rgb_u8(rgb[0], rgb[1], rgb[2]);
        let skin = ui.global::<Skin>();
        skin.set_theme_background(color(t.background));
        skin.set_theme_foreground(color(t.foreground));
        skin.set_theme_accent(color(t.accent));
        skin.set_theme_red(color(t.red));
        skin.set_theme_yellow(color(t.yellow));
        skin.set_theme_green(color(t.green));
        let font = args
            .iter()
            .find_map(|a| a.strip_prefix("font="))
            .unwrap_or("JetBrains Mono");
        skin.set_theme_font(font.into());
        skin.set_omarchy(true);
        ui.invoke_use_scheme(t.dark);
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
    state.set_distance(true);
    state.set_memo_count(5);
    state.set_level_gain(8.6);
    render(&window, &ui, &format!("{out}/main-{lang}.rgba"));
    memos(&window, &ui, &out, &lang);

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
    // The note after the computer could not keep up.
    state.set_overload_note(OverloadNote::Switched);
    render(&window, &ui, &format!("{out}/overload-{lang}.rgba"));
    state.set_overload_note(OverloadNote::None);

    // The settings pages: setup, then one for each card's gear.
    state.set_dictation_state(DictationState::Ready);
    state.set_probe_state(ProbeState::Idle);
    state.set_has_probe(true);
    state.set_mic_active(true);
    state.set_level_before(0.72);
    state.set_level_after(0.55);
    state.set_look_index(2);
    state.set_voice_is_default(false);
    state.set_live_words(true);
    state.set_body_db(1.5);
    state.set_presence_db(-2.0);
    state.set_deesser_index(2);
    for (page, name) in [(1, "settings"), (2, "noise"), (3, "voice"), (4, "dictation")] {
        state.set_page(page);
        render(&window, &ui, &format!("{out}/{name}-{lang}.rgba"));
        if page == 3 {
            state.set_look_index(4);
            render(&window, &ui, &format!("{out}/{name}-custom-{lang}.rgba"));
            state.set_look_index(2);
        }
    }
    state.set_page(5);
    ui.set_memo_folder(if lang == "de" { "Dokumente" } else { "Documents" }.into());
    render(&window, &ui, &format!("{out}/recordings-settings-{lang}.rgba"));
    // The key fields: standard, waiting for a key, one of one's own, refused.
    state.set_memo_key_label("F8".into());
    for (name, page, capture, custom, refusal) in [
        ("keys-standard", 4, 0, false, ""),
        ("keys-listening", 4, 1, false, ""),
        ("keys-custom", 4, 0, true, ""),
        (
            "keys-refused",
            5,
            0,
            false,
            if lang == "de" {
                "F9 ist schon fürs Diktieren belegt."
            } else {
                "F9 is already used for dictation."
            },
        ),
        ("keys-listening-rec", 5, 2, false, ""),
    ] {
        state.set_page(page);
        state.set_key_capture(capture);
        state.set_key_custom(custom);
        state.set_memo_key_custom(custom);
        state.set_key_name(if custom { "Pause" } else { "F9" }.into());
        state.set_memo_key_label(if custom { "Pause" } else { "F8" }.into());
        state.set_key_refusal(refusal.into());
        render(&window, &ui, &format!("{out}/{name}-{lang}.rgba"));
    }
    state.set_key_capture(0);
    state.set_key_custom(false);
    state.set_memo_key_custom(false);
    state.set_key_refusal("".into());
    state.set_key_name("F9".into());
    state.set_memo_key_label("F8".into());
    state.set_page(2);
    // Listening to yourself: the button turns red.
    state.set_listening(true);
    render(&window, &ui, &format!("{out}/noise-listening-{lang}.rgba"));
    state.set_listening(false);
    state.set_page(1);
    state.set_effort_index(2);
    render(&window, &ui, &format!("{out}/settings-custom-{lang}.rgba"));
    state.set_effort_index(1);

    // The voice page with voice off, the defaults and an update on the way.
    state.set_voice_enabled(false);
    state.set_look_index(0);
    state.set_voice_is_default(true);
    state.set_has_probe(false);
    state.set_page(3);
    render(&window, &ui, &format!("{out}/voice-off-{lang}.rgba"));
    state.set_page(1);
    state.set_update_state(UpdateState::Available);
    state.set_update_version("0.4.0".into());
    state.set_update_installable(true);
    render(&window, &ui, &format!("{out}/update-settings-{lang}.rgba"));
    state.set_update_state(UpdateState::Downloading);
    state.set_update_progress(0.37);
    render(&window, &ui, &format!("{out}/update-{lang}.rgba"));
    state.set_update_state(UpdateState::Idle);

    // The user's own models: set up, tested, and the choice on the main page.
    state.set_own_models(true);
    state.set_rework_url("http://192.168.88.112:1234".into());
    state.set_rework_models(ModelRc::new(VecModel::from(vec!["gemma-4-26B-A4B-it".into()])));
    state.set_rework_model_index(0);
    state.set_rework_ready(true);
    state.set_rework_note(
        if lang == "de" {
            "Geht, 1,1 s: „Ich habe heute die Bremsbeläge am Auto gewechselt.“"
        } else {
            "Works, 1.1 s: “Ich habe heute die Bremsbeläge am Auto gewechselt.”"
        }
        .into(),
    );
    state.set_asr_on(true);
    state.set_asr_url("http://192.168.88.118:8081".into());
    state.set_asr_note(
        if lang == "de" {
            "Geht, 0,4 s für eine Sekunde Stille."
        } else {
            "Works, 0.4 s for a second of silence."
        }
        .into(),
    );
    state.set_page(7);
    render(&window, &ui, &format!("{out}/own-models-{lang}.rgba"));
    state.set_rework_index(3);
    state.set_page(0);
    render(&window, &ui, &format!("{out}/main-rework-{lang}.rgba"));
    state.set_page(1);
    render(&window, &ui, &format!("{out}/own-models-on-{lang}.rgba"));
    state.set_rework_ready(false);
    state.set_asr_on(false);
    state.set_own_models(false);

    // Windows on ARM: no filter, dictation only.
    state.set_filter_enabled(false);
    state.set_voice_enabled(false);
    state.set_mic_active(false);
    state.set_filter_state(FilterState::Unsupported);
    state.set_page(2);
    render(&window, &ui, &format!("{out}/arm-noise-{lang}.rgba"));
    state.set_page(1);
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
