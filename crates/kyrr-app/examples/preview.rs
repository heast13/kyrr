//! Renders the window headless into raw RGBA files for checking the layout
//! without a display: `cargo run --example preview -- <out-dir> [de|en]`.
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

const SCALE: f32 = 1.5;

fn render(window: &MinimalSoftwareWindow, ui: &MainWindow, path: &str) {
    ui.show().unwrap();
    window.set_size(PhysicalSize::new((380.0 * SCALE) as u32, 100));
    let (w, h) = ((380.0 * SCALE) as u32, (ui.get_content_height() * SCALE).ceil() as u32);
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
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: SCALE });

    let ui = MainWindow::new().unwrap();
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
    state.set_credits(
        "German: primeline/parakeet-primeline by primeLine AI (CC-BY-4.0), based on NVIDIA parakeet-tdt-0.6b-v3.\n\n\
         English: NVIDIA parakeet-unified-en-0.6b, licensed by NVIDIA Corporation under the NVIDIA Open Model License."
            .into(),
    );
    state.set_phrase(4);
    render(&window, &ui, &format!("{out}/main-{lang}.rgba"));

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

    // Settings with the custom sound, so the sliders and Reset show.
    state.set_dictation_state(DictationState::Ready);
    state.set_page(1);
    state.set_probe_state(ProbeState::Idle);
    state.set_has_probe(true);
    state.set_mic_active(true);
    state.set_level_before(0.72);
    state.set_level_after(0.55);
    state.set_look_index(4);
    state.set_body_db(1.5);
    state.set_presence_db(-2.0);
    state.set_deesser_index(2);
    state.set_voice_is_default(false);
    render(&window, &ui, &format!("{out}/settings-{lang}.rgba"));

    // Settings with voice off and the defaults.
    state.set_voice_enabled(false);
    state.set_look_index(0);
    state.set_voice_is_default(true);
    state.set_has_probe(false);
    state.set_update_state(UpdateState::Available);
    state.set_update_version("0.2.0".into());
    render(&window, &ui, &format!("{out}/settings-off-{lang}.rgba"));
    state.set_update_state(UpdateState::Idle);

    state.set_page(0);
    ui.set_setup_mode(true);
    state.set_filter_state(FilterState::NotAttached);
    render(&window, &ui, &format!("{out}/setup1-{lang}.rgba"));

    ui.set_setup_step(1);
    state.set_dictation_state(DictationState::Downloading);
    state.set_download_progress(0.42);
    render(&window, &ui, &format!("{out}/setup2-{lang}.rgba"));
}
