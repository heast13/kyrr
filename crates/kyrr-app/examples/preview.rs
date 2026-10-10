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
    state.set_version("0.1.0".into());
    state.set_microphones(ModelRc::new(VecModel::from(vec![
        "Mikrofon (Shure MV7)".into(),
        "Headset (Logitech G Pro X)".into(),
    ])));
    state.set_attached_mic("Mikrofon (Shure MV7)".into());
    state.set_filter_state(FilterState::Running);
    state.set_dictation_state(DictationState::Ready);
    state.set_download_size("880 MB".into());
    render(&window, &ui, &format!("{out}/main-{lang}.rgba"));

    ui.set_setup_mode(true);
    state.set_filter_state(FilterState::NotAttached);
    render(&window, &ui, &format!("{out}/setup1-{lang}.rgba"));

    ui.set_setup_step(1);
    state.set_dictation_state(DictationState::Downloading);
    state.set_download_progress(0.42);
    render(&window, &ui, &format!("{out}/setup2-{lang}.rgba"));
}
