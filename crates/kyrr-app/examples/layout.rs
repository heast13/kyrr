//! The layout check of the release gate: walks every page and state of the
//! window headless (software renderer, no display, no winit window) and fails
//! on the classic layout bugs: window sizes that do not follow the display
//! scale or the screen, needless scrolling, content in the margins.
//! `cargo run --release --example layout -- <out-dir> [lang=de,en] [scale=1,1.25,1.5,2]
//! [look=light,dark,tokyo-night,catppuccin-latte] [os=windows|linux] [panel=<px>]
//! [sheets=off] [verbose=on] [selftest=on]`.
//! The exit code is 1 when anything is wrong, with one line per problem
//! (also in `<out-dir>/problems.txt`). Besides that, `<out-dir>` gets contact
//! sheets of all pages, one set of PNG files per look, language and scale
//! (`sheet-<look>-<lang>-<scale>-<n>.png` with `.txt` beside it, naming the
//! numbered pages). `scale` takes a list, `selftest=on` runs with sizing rules that are
//! wrong on purpose and must find problems, `verbose=on` prints every page's
//! measurements. `os` is the machine's by default: Windows keeps the taskbar
//! out of the screen the app measures, Linux does not know about panels
//! (`panel` is the one the check assumes, as many px as the app keeps free).
//!
//! What it checks, for each page in German and English, in each look (the
//! system's light and dark scheme, and on Linux two Omarchy themes, a dark
//! and a light one), at the display scales 1, 1.25, 1.5 and 2 and on the
//! screens in `SCREENS`:
//! - the window's height (the way main.rs picks it: content, up to what the
//!   screen has room for) leaves the window within the screen's free height;
//! - the page does not scroll although the screen has room for all of it,
//!   not even by the fraction of a pixel that rounding the window to whole
//!   pixels can leave;
//! - the window's width is the page's (380, 520, 1040 for the memo text next
//!   to the text before, less on a narrow screen), the same in physical
//!   pixels as logical pixels times the scale, and fits the screen;
//! - nothing reaches into the margins left and right (text or a control that
//!   is wider than the window, found in the rendered picture);
//! - the layout does not depend on the scale (the content's height in logical
//!   pixels stays about the same);
//! - the memo's text field and the news window do not scroll although the
//!   screen has room (found in the rendered picture too);
//! - every look has the colours it should (a dark window is dark, a light
//!   one light), and no part of a page is readable in the light look and
//!   lost in the dark one or the other way round (text in a fixed colour
//!   that vanishes on the other background): the two pictures of a pair of
//!   looks are compared tile by tile.
//!
//! What it cannot check: the real window of the operating system (frame,
//! taskbar, panel, what a compositor does with a window). The screen is a
//! model (`Os`); the sizing rules are the app's own (src/sizing.rs is
//! included here, `check_source` sees that main.rs uses them). Text that is cut off inside
//! its own element cannot be told from text that fits, and the layout's own
//! minimum width cannot be read from outside, only what shows in the margins.
//! The unfolded credits on the general settings page and pop-ups cannot be
//! opened from outside. The font is the machine's default, not Segoe UI, so
//! text wraps a little differently than on a user's machine.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, SharedString, VecModel};

slint::include_modules!();

// The sizing rules of the app itself, not a copy.
#[path = "../src/sizing.rs"]
mod sizing;

// ---------------------------------------------------------------------------
// What is walked through.
// ---------------------------------------------------------------------------

const LANGS: [&str; 2] = ["de", "en"];
const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];
/// Screens in logical pixels (width, height): the usual sizes, 720 is a
/// 1080p display at 150 %, 1024 x 768 the narrowest the compare view meets.
const SCREENS: [(u32, u32); 8] = [
    (1024, 768),
    (1280, 720),
    (1366, 768),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
    (1920, 1200),
];

/// The two systems, as far as they differ for the window's size.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Os {
    /// The app asks the monitor for its work area: the taskbar is not in it.
    Windows,
    /// The app asks winit for the monitor: panels are in it (Wayland tells
    /// an app nothing else).
    Linux,
}

impl Os {
    /// Title bar and borders that the window really has, in logical pixels.
    fn frame(self) -> f32 {
        match self {
            // Windows 11: 31 px caption, 1 px borders.
            Os::Windows => 34.0,
            // GNOME's client side decoration, the tallest of the usual ones.
            Os::Linux => 38.0,
        }
    }
}

/// One way the window can look: the system's light or dark scheme, or the
/// colours of an Omarchy theme (the app takes them from the theme in use).
#[derive(Clone, Copy)]
struct Look {
    name: &'static str,
    dark: bool,
    /// Omarchy's theme: background, foreground, accent, red, yellow, green.
    theme: Option<[[u8; 3]; 6]>,
    /// The looks of a group are the same layout in light and dark colours.
    group: &'static str,
}

const LOOKS: [Look; 4] = [
    Look {
        name: "light",
        dark: false,
        theme: None,
        group: "system",
    },
    Look {
        name: "dark",
        dark: true,
        theme: None,
        group: "system",
    },
    // Omarchy's default theme and a light one, with the colours of their palettes.
    Look {
        name: "tokyo-night",
        dark: true,
        theme: Some([
            [0x1a, 0x1b, 0x26],
            [0xa9, 0xb1, 0xd6],
            [0x7a, 0xa2, 0xf7],
            [0xf7, 0x76, 0x8e],
            [0xe0, 0xaf, 0x68],
            [0x9e, 0xce, 0x6a],
        ]),
        group: "omarchy",
    },
    Look {
        name: "catppuccin-latte",
        dark: false,
        theme: Some([
            [0xef, 0xf1, 0xf5],
            [0x4c, 0x4f, 0x69],
            [0x1e, 0x66, 0xf5],
            [0xd2, 0x0f, 0x39],
            [0xdf, 0x8e, 0x1d],
            [0x40, 0xa0, 0x2b],
        ]),
        group: "omarchy",
    },
];

impl Look {
    /// What `kyrr` does for the theme in use: the scheme, the colours, and
    /// the square Omarchy look.
    fn skin(self, skin: &Skin<'_>) {
        let Some([background, foreground, accent, red, yellow, green]) = self.theme else {
            skin.set_omarchy(false);
            return;
        };
        let color = |c: [u8; 3]| slint::Color::from_rgb_u8(c[0], c[1], c[2]);
        skin.set_theme_background(color(background));
        skin.set_theme_foreground(color(foreground));
        skin.set_theme_accent(color(accent));
        skin.set_theme_red(color(red));
        skin.set_theme_yellow(color(yellow));
        skin.set_theme_green(color(green));
        skin.set_omarchy(true);
    }

    /// The colour the window's background must have: dark looks are dark, light ones light.
    fn background_fits(self, bg: [u8; 3]) -> bool {
        let l = luminance(bg);
        if self.dark {
            l < 0.25
        } else {
            l > 0.5
        }
    }
}

/// The taskbar of Windows, in logical pixels (it does not grow with the scale).
const TASKBAR: f32 = 48.0;
/// The panel of GNOME or KDE at the top or bottom, in logical pixels.
const DEFAULT_PANEL: f32 = sizing::LINUX_PANEL;

/// How many logical pixels left and right of the content must stay empty
/// (the window's padding is 18).
const MARGIN: f32 = 10.0;
/// The content's height may differ this much between scales (a line more or
/// less when a word wraps differently).
const SCALE_SLACK: f32 = 40.0;
/// The memo text needs at least this many logical pixels of height to be
/// read and edited.
const TEXT_FIELD_MIN: f32 = 120.0;
/// Taller than this the contact sheet starts another picture.
const SHEET_WIDTH: usize = 3200;
const SHEET_HEIGHT: usize = 3600;

// ---------------------------------------------------------------------------
// The platform: one window that is drawn into memory, and a clock that is
// moved by hand, so animations end at once instead of after real time.
// ---------------------------------------------------------------------------

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

/// A picture of the window, three bytes per pixel.
struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Image {
    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    /// Whether anything but `bg` is in the columns `xs` of row `y`.
    fn row_has_ink(&self, y: usize, xs: std::ops::Range<usize>, bg: [u8; 3]) -> bool {
        xs.into_iter().any(|x| differs(self.px(x, y), bg))
    }
}

/// Colours that differ so much that the eye sees it (not anti-aliasing noise).
fn differs(a: [u8; 3], b: [u8; 3]) -> bool {
    differs_by(a, b, 12)
}

fn differs_by(a: [u8; 3], b: [u8; 3], limit: u8) -> bool {
    a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > limit)
}

struct Rig {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    scale: Cell<f32>,
}

impl Rig {
    fn new() -> Rig {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Headless {
            window: window.clone(),
            clock: clock.clone(),
        }))
        .expect("the platform is set once");
        Rig {
            window,
            clock,
            scale: Cell::new(1.0),
        }
    }

    fn set_scale(&self, scale: f32) {
        self.scale.set(scale);
        self.window
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    }

    /// Lets every animation and timer run to its end.
    fn settle(&self) {
        self.clock.set(self.clock.get() + Duration::from_secs(5));
        slint::platform::update_timers_and_animations();
    }

    /// Logical to physical pixels the way Slint (and winit) round them.
    fn phys(&self, logical: f32) -> u32 {
        (logical * self.scale.get()).round() as u32
    }

    fn set_size(&self, width: f32, height: f32) {
        self.window
            .set_size(PhysicalSize::new(self.phys(width), self.phys(height)));
        self.settle();
    }

    fn draw(&self) -> Image {
        let size = self.window.size();
        let (w, h) = (size.width as usize, size.height as usize);
        let mut buf = vec![PremultipliedRgbaColor::default(); w * h];
        self.window.request_redraw();
        self.window.draw_if_needed(|r| {
            r.render(&mut buf, w);
        });
        let rgb = buf.iter().flat_map(|p| [p.red, p.green, p.blue]).collect();
        Image {
            width: w,
            height: h,
            rgb,
        }
    }
}

// ---------------------------------------------------------------------------
// Contact sheets.
// ---------------------------------------------------------------------------

/// 3 x 5 pixels per digit, one row per three bits.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
const LABEL_BAND: usize = 20;
const GAP: usize = 10;
const SHEET_BG: u8 = 0x70;

/// All pages of one language and scale on a few PNG files, shelf by shelf.
/// Every picture has its number above it; `.txt` names the numbers.
struct Sheets {
    dir: PathBuf,
    prefix: String,
    canvas: Vec<u8>,
    x: usize,
    y: usize,
    row: usize,
    file: usize,
    next: usize,
    index: String,
    enabled: bool,
}

impl Sheets {
    fn new(dir: &Path, prefix: String, enabled: bool) -> Sheets {
        Sheets {
            dir: dir.to_path_buf(),
            prefix,
            canvas: Vec::new(),
            x: GAP,
            y: GAP,
            row: 0,
            file: 1,
            next: 1,
            index: String::new(),
            enabled,
        }
    }

    fn add(&mut self, name: &str, img: &Image) {
        if !self.enabled {
            return;
        }
        // A very long page (many memos) is cut off at the usual tallest window.
        let (w, h) = (img.width.min(SHEET_WIDTH - 2 * GAP), img.height.min(1460 * 2));
        if self.x + w + GAP > SHEET_WIDTH {
            self.x = GAP;
            self.y += self.row + GAP;
            self.row = 0;
        }
        if self.y + LABEL_BAND + h + GAP > SHEET_HEIGHT {
            self.flush();
        }
        if self.canvas.is_empty() {
            self.canvas = vec![SHEET_BG; SHEET_WIDTH * SHEET_HEIGHT * 3];
        }
        let number = self.next;
        self.next += 1;
        self.label(number);
        let top = self.y + LABEL_BAND;
        for row in 0..h.min(SHEET_HEIGHT - top) {
            let from = row * img.width * 3;
            let to = ((top + row) * SHEET_WIDTH + self.x) * 3;
            self.canvas[to..to + w * 3].copy_from_slice(&img.rgb[from..from + w * 3]);
        }
        let _ = writeln!(self.index, "{number:3}  {name}  ({}x{} px)", img.width, img.height);
        self.x += w + GAP;
        self.row = self.row.max(LABEL_BAND + h);
    }

    /// The number in white, three times its size, above the picture.
    fn label(&mut self, number: usize) {
        let mut x = self.x;
        for digit in number.to_string().bytes() {
            for (row, bits) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
                for col in 0..3 {
                    if bits >> (2 - col) & 1 == 1 {
                        for dy in 0..3 {
                            for dx in 0..3 {
                                let (px, py) = (x + col * 3 + dx, self.y + 2 + row * 3 + dy);
                                let i = (py * SHEET_WIDTH + px) * 3;
                                self.canvas[i..i + 3].fill(0xff);
                            }
                        }
                    }
                }
            }
            x += 4 * 3;
        }
    }

    fn flush(&mut self) {
        if self.canvas.is_empty() {
            return;
        }
        let height = (self.y + self.row + GAP).min(SHEET_HEIGHT);
        let base = self.dir.join(format!("{}-{}", self.prefix, self.file));
        let file = std::fs::File::create(format!("{}.png", base.display())).expect("a sheet file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SHEET_WIDTH as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder.write_header().expect("a PNG header");
        writer
            .write_image_data(&self.canvas[..SHEET_WIDTH * height * 3])
            .expect("PNG data");
        std::fs::write(format!("{}.txt", base.display()), &self.index).expect("a sheet index");
        self.canvas = Vec::new();
        self.index.clear();
        self.file += 1;
        self.x = GAP;
        self.y = GAP;
        self.row = 0;
    }
}

// ---------------------------------------------------------------------------
// The sizing rules.
// ---------------------------------------------------------------------------

/// The sizing rules are in src/sizing.rs, which this file includes. What
/// main.rs does with them cannot be included; this only checks that the
/// window's size is always set in logical pixels.
const MAIN_RS: &str = include_str!("../src/main.rs");
const PLATFORM_RS: &str = include_str!("../src/platform.rs");

/// What the check changes in the rules on purpose (`selftest=on`).
struct Rules {
    selftest: bool,
}

fn check_source() -> Vec<String> {
    let mut problems = Vec::new();
    // The window is always sized in logical pixels, so it follows the scale.
    for (file, source) in [("main.rs", MAIN_RS), ("platform.rs", PLATFORM_RS)] {
        if source.contains("PhysicalSize") {
            problems.push(format!(
                "source: {file} uses PhysicalSize; the window's size must be set in logical pixels to follow the display scale"
            ));
        }
        for (at, _) in source.match_indices(".set_size(") {
            if !source[at + ".set_size(".len()..]
                .trim_start()
                .starts_with("LogicalSize::new(")
            {
                let line = source[..at].matches('\n').count() + 1;
                problems.push(format!(
                    "source: {file}:{line} sets a window size that is not a LogicalSize; it would not follow the display scale"
                ));
            }
        }
    }
    // The app takes its sizes from the rules above.
    for needed in [
        "sizing::height_cap(",
        "sizing::wanted_height(",
        "sizing::sheet_height(",
        "sizing::widest(",
        "sizing::news_height(",
        "sizing::whole_pixels(",
    ] {
        if !MAIN_RS.contains(needed) {
            problems.push(format!(
                "source: main.rs does not use `{needed}…` any more; it must size its windows with the rules in sizing.rs"
            ));
        }
    }
    problems
}

/// A screen, as the app sees it and as it really is, in logical pixels.
struct Screen {
    width: u32,
    height: u32,
    /// The free height the user has: without the taskbar or the panel.
    usable: f32,
    /// What the app measures: `height_cap`.
    cap: f32,
    /// What the app takes as the free width (`fit_sheet`).
    widest: f32,
}

impl Screen {
    fn new(os: Os, size: (u32, u32), scale: f32, panel: f32, rules: &Rules) -> Screen {
        let (width, height) = size;
        let (phys_w, phys_h) = ((width as f32 * scale).round(), (height as f32 * scale).round());
        let (usable, seen_h) = match os {
            Os::Windows => (height as f32 - TASKBAR, phys_h - (TASKBAR * scale).round()),
            Os::Linux => (height as f32 - panel, phys_h),
        };
        // The free screen the app sees (a Linux app keeps room for panels).
        let area = Some((0, 0, phys_w as i32, seen_h as i32));
        let mut cap = sizing::height_cap(area, scale, sizing::panel(os == Os::Linux));
        if rules.selftest {
            // Wrong on purpose: no room for the frame, and a low maximum.
            cap = (cap + sizing::FRAME).min(900.0);
        }
        let widest = sizing::widest((0, 0, phys_w as i32, phys_h as i32), scale);
        Screen {
            width,
            height,
            usable,
            cap,
            widest,
        }
    }

    fn label(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

/// The width a page should have, whatever the code says: 380 for the
/// settings, 520 for the memos and a memo's text, twice that for the
/// comparison (as wide as the screen allows, down to 520).
fn intended_width(kind: Kind, screen: &Screen) -> f32 {
    match kind {
        Kind::Main | Kind::Settings | Kind::Setup => 380.0,
        Kind::Memos | Kind::Text => 520.0,
        Kind::Compare => 1040.0f32.min((screen.width as f32 - 16.0).max(520.0)),
    }
}

/// The height the app gives the window for a page whose content is
/// `content` high (`resize` after `wanted_height`, and `fit_sheet` for a
/// memo's text), in whole physical pixels.
fn window_height(kind: Kind, content: f32, screen: &Screen, scale: f32) -> f32 {
    let memos = sizing::wanted_height(content, screen.cap, true);
    let height = match kind {
        Kind::Memos => memos,
        Kind::Text | Kind::Compare => memos.max(sizing::sheet_height(screen.cap)),
        _ => sizing::wanted_height(content, screen.cap, false),
    };
    sizing::whole_pixels(height, scale)
}

/// The window's height in physical pixels as it ends up. Slint raises a
/// window to its minimum height and rounds that up (the main page on every
/// system, every page on Linux, where the app sets the minimum); a plain
/// request (the other pages on Windows) is rounded to the nearest pixel,
/// which the app prevents by asking for whole pixels.
fn window_px(os: Os, kind: Kind, height: f32, scale: f32) -> u32 {
    let exact = height * scale;
    if kind == Kind::Main || os == Os::Linux {
        (exact - 0.001).ceil() as u32
    } else {
        exact.round() as u32
    }
}

// ---------------------------------------------------------------------------
// The pages.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Main,
    Settings,
    Setup,
    Memos,
    /// A memo's text over the list.
    Text,
    /// The same, with the text before beside it.
    Compare,
}

#[derive(Clone, Copy)]
struct Ctx {
    de: bool,
    os: Os,
    look: Look,
}

impl Ctx {
    fn lang(self) -> &'static str {
        if self.de {
            "de"
        } else {
            "en"
        }
    }

    fn t(self, german: &str, english: &str) -> SharedString {
        if self.de { german } else { english }.into()
    }
}

/// What a page does to the window's state, after the baseline.
type Setup = Box<dyn Fn(&MainWindow, &AppState, Ctx)>;

struct Scenario {
    name: String,
    kind: Kind,
    /// Drawn and looked at; the others only measure the content's height.
    drawn: bool,
    set: Setup,
}

fn strings(list: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        list.iter().map(|s| (*s).into()).collect::<Vec<SharedString>>(),
    ))
}

/// What a running kyrr shows to begin with (the preview's state).
fn baseline(ui: &MainWindow, s: &AppState, c: Ctx) {
    let mic = c.t("Mikrofon (Shure MV7)", "Microphone (Shure MV7)");
    s.set_version(env!("CARGO_PKG_VERSION").into());
    s.set_linux(c.os == Os::Linux);
    s.set_language_index(i32::from(!c.de));
    s.set_microphones(ModelRc::new(VecModel::from(vec![
        mic.clone(),
        "Headset (Logitech G Pro X)".into(),
    ])));
    s.set_attached_mic(mic);
    s.set_filter_state(FilterState::Running);
    s.set_dictation_state(DictationState::Ready);
    s.set_mic_active(true);
    s.set_window_shown(true);
    s.set_level_before(0.72);
    s.set_level_after(0.55);
    s.set_download_size("880 MB".into());
    s.set_phrase(3);
    s.set_auto_level(true);
    s.set_distance(true);
    s.set_memo_count(5);
    s.set_level_gain(8.6);
    s.set_look_index(2);
    s.set_deesser_index(2);
    ui.set_memo_folder(c.t("Dokumente", "Documents"));
}

fn memo_items(c: Ctx, many: bool) -> Vec<MemoItem> {
    let item = |title: SharedString, when: SharedString, text_state: i32| MemoItem {
        title,
        when,
        pending: false,
        text_state,
        text_progress: 0.0,
        text_left: "".into(),
    };
    let mut writing = item(
        c.t("Gespräch mit dem Steuerberater", "Call with the tax adviser"),
        c.t("Gestern, 16:20 · 12:48", "Yesterday, 16:20 · 12:48"),
        2,
    );
    writing.text_progress = 0.42;
    writing.text_left = c.t("noch 3 Min.", "3 min left");
    let mut pending = item("".into(), c.t("Heute, 18:05 · 0:42", "Today, 18:05 · 0:42"), 0);
    pending.pending = true;
    let mut items = vec![
        pending,
        item(
            c.t("Einkaufsliste für Samstag", "Shopping list for Saturday"),
            c.t("Heute, 09:12 · 0:48", "Today, 09:12 · 0:48"),
            3,
        ),
        item(
            c.t("Gedanken zur neuen Startseite", "Thoughts on the new home page"),
            c.t("Gestern, 21:40 · 2:31", "Yesterday, 21:40 · 2:31"),
            0,
        ),
        writing,
        item(
            c.t("Test Test hört man mich", "Testing can you hear me"),
            "18.10.2026, 11:02 · 0:06".into(),
            0,
        ),
        item(
            c.t(
                "Besprechung mit dem Steuerberater über die Homeoffice Pauschale und alle Belege",
                "Meeting with the tax adviser about the home office allowance and all the receipts",
            ),
            "17.10.2026, 09:30 · 31:12".into(),
            3,
        ),
    ];
    if many {
        for i in 0..54 {
            items.push(item(
                c.t(
                    &format!("Sprachaufzeichnung Nummer {i}"),
                    &format!("Voice recording number {i}"),
                ),
                format!(
                    "{}.09.2026, 1{}:{:02} · {}:{:02}",
                    1 + i % 28,
                    i % 10,
                    i % 60,
                    i % 9,
                    i % 60
                )
                .into(),
                [0, 3, 4, 1][i % 4],
            ));
        }
    }
    items
}

/// The memo text in the field: four paragraphs.
fn memo_text(c: Ctx, stamps: i32) -> SharedString {
    let paragraphs: [(&str, &str); 4] = if c.de {
        [
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
        [
            ("0:00", "Thoughts on the new home page. At the top it should be clear right away what kyrr does, no marketing talk."),
            ("0:34", "Below that the download, big, and right next to it the Linux commands to copy."),
            ("1:12", "The example pill as a short loop, not as a video, that takes forever to load."),
            ("2:05", "And at the bottom just the common questions, folded away."),
        ]
    };
    paragraphs
        .iter()
        .map(|(time, text)| match stamps {
            1 => format!("[{time}] {text}"),
            2 => format!("[21:4{}] {text}", time.len() % 3),
            _ => (*text).to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        .into()
}

/// What the comparison shows, as `memo::compare` writes it.
fn compare_pairs(de: bool) -> Vec<ComparePair> {
    let (gone, came) = ("#d9534f", "#2e9e5b");
    let styled = |m: String| slint::StyledText::from_markdown(&m).expect("markdown");
    let pairs: Vec<(String, String)> = if de {
        vec![
            (
                format!("<font color=\"{gone}\">Gedanken zur neuen Startseite. Oben soll sofort klar sein, was kyrr macht, ohne Werbesprech.</font>"),
                format!("<font color=\"{came}\">Überlegungen zur neuen Startseite: Oben muss sofort erkennbar sein, was kyrr leistet, ohne Werbesprache.</font>"),
            ),
            (
                format!("Darunter <font color=\"{gone}\">der Download, groß, und gleich daneben</font> die Befehle für Linux zum Kopieren\\."),
                format!("Darunter <font color=\"{came}\">folgt gut sichtbar der Download, direkt daneben</font> die Befehle für Linux zum Kopieren\\."),
            ),
            (
                "Die Beispiel\\-Pille als kurze Schleife, nicht als Video, das lädt sonst ewig\\.".into(),
                "The example pill as a short loop rather than a video, which would take too long to load\\.".into(),
            ),
            (
                "Und unten nur noch die häufigen Fragen, zusammengeklappt\\.".into(),
                "Und unten nur noch die häufigen Fragen, zusammengeklappt\\.".into(),
            ),
        ]
    } else {
        vec![
            (
                format!("<font color=\"{gone}\">Thoughts on the new home page.</font> At the top it should be clear right away what kyrr does, no marketing talk\\."),
                format!("<font color=\"{came}\">Notes on the new home page:</font> the top must make clear at once what kyrr does, without any marketing talk\\."),
            ),
            (
                "Below that the download, big, and right next to it the Linux commands to copy\\.".into(),
                "Darunter der Download, groß, und gleich daneben die Befehle für Linux zum Kopieren\\.".into(),
            ),
        ]
    };
    pairs
        .into_iter()
        .map(|(before, after)| ComparePair {
            before: styled(before),
            after: styled(after),
        })
        .collect()
}

fn open_text(s: &AppState, c: Ctx, stamps: i32) {
    s.set_memo_items(ModelRc::new(VecModel::from(memo_items(c, false))));
    s.set_memo_count(6);
    s.set_memo_can_write(true);
    s.set_page(6);
    s.set_memo_sheet_index(2);
    s.set_memo_sheet_title(c.t("Gedanken zur neuen Startseite", "Thoughts on the new home page"));
    s.set_memo_sheet_when(c.t("Gestern, 21:40 · 2:31", "Yesterday, 21:40 · 2:31"));
    s.set_memo_sheet_stamps(stamps);
    s.set_memo_sheet_text(memo_text(c, stamps));
    s.set_memo_sheet_open(true);
}

/// The language model on the open text: ready, with the original kept.
fn reworked(s: &AppState, c: Ctx) {
    s.set_memo_rework_ready(true);
    s.set_memo_rework_action(3);
    s.set_memo_rework_target(1);
    s.set_memo_sheet_original(true);
    s.set_memo_rework_note(c.t(
        "Professionell formuliert. 1 von 4 Absätzen unverändert. Das Original ist gesichert.",
        "Made professional. 1 of 4 paragraphs unchanged. The original is kept.",
    ));
}

const LONG_MIC_DE: &str = "Mikrofonarray (Intel® Smart Sound Technology für digitale Mikrofone)";
const LONG_MIC_EN: &str = "Microphone Array (Intel® Smart Sound Technology for digital microphones)";

fn add(
    list: &mut Vec<Scenario>,
    name: impl Into<String>,
    kind: Kind,
    set: impl Fn(&MainWindow, &AppState, Ctx) + 'static,
) {
    list.push(Scenario {
        name: name.into(),
        kind,
        drawn: true,
        set: Box::new(set),
    });
}

/// Every page and state that the window can show.
fn scenarios() -> Vec<Scenario> {
    let mut v: Vec<Scenario> = Vec::new();

    // The main page and the notes that can come with it.
    add(&mut v, "main", Kind::Main, |_, _, _| {});
    add(&mut v, "main-standby", Kind::Main, |_, s, _| {
        s.set_mic_active(false);
        s.set_level_before(0.0);
        s.set_level_after(0.0);
        s.set_phrase(0);
    });
    add(&mut v, "main-memo-recording", Kind::Main, |_, s, c| {
        s.set_memo_recording(true);
        s.set_memo_elapsed("12:07".into());
        s.set_memo_message(c.t("Das Mikrofon ist gerade belegt.", "The microphone is busy right now."));
    });
    add(&mut v, "main-memo-new", Kind::Main, |_, s, _| s.set_memo_new(true));
    add(&mut v, "main-memos-off", Kind::Main, |_, s, _| {
        s.set_memos_enabled(false)
    });
    add(&mut v, "main-dictation-recording", Kind::Main, |_, s, _| {
        s.set_dictation_state(DictationState::Recording);
    });
    add(&mut v, "main-dictation-transcribing", Kind::Main, |_, s, _| {
        s.set_dictation_state(DictationState::Transcribing);
    });
    add(&mut v, "main-listening", Kind::Main, |_, s, _| {
        s.set_listening(true);
        s.set_alert(3);
    });
    for (alert, name) in [
        (1, "quiet"),
        (2, "loud"),
        (4, "muted"),
        (6, "no-mic"),
        (7, "connected"),
        (8, "gym"),
    ] {
        add(&mut v, format!("main-alert-{name}"), Kind::Main, move |_, s, c| {
            s.set_alert(alert);
            s.set_connected_name(c.t(LONG_MIC_DE, LONG_MIC_EN));
        });
    }
    for (state, installable, news, name) in [
        (UpdateState::Available, true, true, "offer"),
        (UpdateState::Available, false, false, "offer-page"),
        (UpdateState::Downloading, true, true, "downloading"),
        (UpdateState::Installing, true, true, "installing"),
        (UpdateState::DownloadFailed, true, false, "failed"),
    ] {
        add(&mut v, format!("main-update-{name}"), Kind::Main, move |_, s, _| {
            s.set_update_version("0.10.2".into());
            s.set_update_installable(installable);
            s.set_update_has_news(news);
            s.set_update_progress(0.37);
            s.set_update_state(state);
        });
    }
    for (note, name) in [(OverloadNote::Switched, "switched"), (OverloadNote::Pauses, "pauses")] {
        add(&mut v, format!("main-overload-{name}"), Kind::Main, move |_, s, _| {
            s.set_overload_note(note)
        });
    }
    add(&mut v, "main-all-notes", Kind::Main, |_, s, c| {
        // Everything at once that adds a line to the main page.
        s.set_update_version("0.10.2".into());
        s.set_update_installable(true);
        s.set_update_has_news(true);
        s.set_update_state(UpdateState::Available);
        s.set_overload_note(OverloadNote::Pauses);
        s.set_alert(7);
        s.set_connected_name(c.t(LONG_MIC_DE, LONG_MIC_EN));
        s.set_memo_recording(true);
        s.set_memo_message(c.t("Das Mikrofon ist gerade belegt.", "The microphone is busy right now."));
        s.set_ai_features(true);
        s.set_local_ai(true);
        s.set_rework_ready(true);
        s.set_dictation_state(DictationState::Error);
        s.set_dictation_message(c.t(
            "Kein Internet oder der Server antwortet nicht. Nochmal versuchen?",
            "No internet or the server does not answer. Try again?",
        ));
        s.set_move_to("Headset (Logitech G Pro X)".into());
        s.set_filter_state(FilterState::Detached);
    });
    add(&mut v, "main-all-off", Kind::Main, |_, s, _| {
        s.set_filter_enabled(false);
        s.set_voice_enabled(false);
        s.set_dictation_enabled(false);
        s.set_dictation_state(DictationState::Off);
    });
    add(&mut v, "main-model-missing", Kind::Main, |_, s, _| {
        s.set_dictation_state(DictationState::NoModel);
    });
    add(&mut v, "main-model-downloading", Kind::Main, |_, s, _| {
        s.set_dictation_state(DictationState::Downloading);
        s.set_download_progress(0.42);
    });
    add(&mut v, "main-model-loading", Kind::Main, |_, s, _| {
        s.set_dictation_state(DictationState::Loading)
    });
    add(&mut v, "main-dictation-error", Kind::Main, |_, s, c| {
        s.set_dictation_state(DictationState::Error);
        s.set_dictation_message(c.t(
            "Kein Internet oder der Server antwortet nicht. Nochmal versuchen?",
            "No internet or the server does not answer. Try again?",
        ));
    });
    add(&mut v, "main-dictation-hint", Kind::Main, |_, s, c| {
        s.set_dictation_message(c.t(
            "Das Sprachmodell wird gerade aktualisiert, Diktate dauern etwas länger.",
            "The speech model is being updated, dictations take a little longer.",
        ));
    });
    add(&mut v, "main-key-unbound", Kind::Main, |_, s, c| {
        s.set_key_unbound(true);
        s.set_key_button(c.t("Taste einrichten", "Set up key"));
    });
    add(&mut v, "main-key-note", Kind::Main, |_, s, c| {
        s.set_key_note(c.t(
            "Dein Desktop muss die Taste einmal erlauben: Einstellungen, Tastatur, Tastenkürzel.",
            "Your desktop has to allow the key once: Settings, Keyboard, Shortcuts.",
        ));
    });
    for (state, name) in [
        (FilterState::NotAttached, "not-attached"),
        (FilterState::NotInstalled, "not-installed"),
        (FilterState::SettingUp, "setting-up"),
        (FilterState::Error, "error"),
        (FilterState::Detached, "detached"),
        (FilterState::Passthrough, "passthrough"),
        (FilterState::Unsupported, "unsupported"),
        (FilterState::Checking, "checking"),
    ] {
        add(&mut v, format!("main-filter-{name}"), Kind::Main, move |_, s, c| {
            s.set_filter_state(state);
            s.set_attached_mic(c.t(LONG_MIC_DE, LONG_MIC_EN));
            s.set_error_text(c.t(
                "Das Mikrofon lässt sich nicht öffnen. Ist es von einer anderen App gesperrt?",
                "The microphone cannot be opened. Is another app holding it?",
            ));
            s.set_pipewire_button(true);
            s.set_pipewire_note(c.t(
                "PipeWire fehlt. kyrr kann es für dich einrichten, dafür fragt dein System nach dem Passwort.",
                "PipeWire is missing. kyrr can set it up for you; your system asks for the password.",
            ));
        });
    }
    for unplugged in [false, true] {
        add(
            &mut v,
            format!("main-move-to-{}", if unplugged { "unplugged" } else { "other" }),
            Kind::Main,
            move |_, s, _| {
                s.set_move_to("Headset (Logitech G Pro X)".into());
                s.set_move_unplugged(unplugged);
            },
        );
    }
    add(&mut v, "main-widget", Kind::Main, |_, s, _| s.set_widget(true));
    add(&mut v, "main-local-ai-off", Kind::Main, |_, s, _| {
        s.set_ai_features(true);
        s.set_local_ai(false);
    });
    add(&mut v, "main-local-ai-new", Kind::Main, |_, s, _| {
        s.set_ai_features(true);
        s.set_local_ai(true);
        s.set_rework_found("Ollama".into());
    });
    add(&mut v, "main-local-ai-ready", Kind::Main, |_, s, _| {
        s.set_ai_features(true);
        s.set_local_ai(true);
        s.set_rework_ready(true);
        s.set_rework_index(3);
        s.set_translate_index(3);
    });
    // "Fix mistakes" with the word order switch under it, switched on.
    add(&mut v, "main-local-ai-fix-order", Kind::Main, |_, s, _| {
        s.set_ai_features(true);
        s.set_local_ai(true);
        s.set_rework_ready(true);
        s.set_rework_index(1);
        s.set_fix_order(true);
    });
    // The status line wraps to a second line for a long phrase: the main
    // window grows and shrinks with it. Heights only.
    for (state, phrase) in [("idle", 0..20), ("memo", 0..20)] {
        for p in phrase {
            v.push(Scenario {
                name: format!("main-phrase-{state}-{p}"),
                kind: Kind::Main,
                drawn: false,
                set: Box::new(move |_, s, _| {
                    s.set_phrase(p);
                    s.set_memo_recording(state == "memo");
                }),
            });
        }
    }

    // The settings pages.
    add(&mut v, "settings", Kind::Settings, |_, s, _| s.set_page(1));
    for (state, installable, name) in [
        (UpdateState::Checking, true, "checking"),
        (UpdateState::Current, true, "current"),
        (UpdateState::Failed, true, "failed"),
        (UpdateState::Available, true, "offer"),
        (UpdateState::Available, false, "offer-page"),
        (UpdateState::Downloading, true, "downloading"),
        (UpdateState::Installing, true, "installing"),
        (UpdateState::DownloadFailed, true, "download-failed"),
    ] {
        add(
            &mut v,
            format!("settings-update-{name}"),
            Kind::Settings,
            move |_, s, _| {
                s.set_page(1);
                s.set_update_state(state);
                s.set_update_version("0.10.2".into());
                s.set_update_installable(installable);
                s.set_update_progress(0.37);
            },
        );
    }
    add(&mut v, "settings-all", Kind::Settings, |_, s, c| {
        // The longest the page gets: an update on offer, everything on.
        s.set_page(1);
        s.set_update_state(UpdateState::Available);
        s.set_update_version("0.10.2".into());
        s.set_update_installable(true);
        s.set_ai_features(true);
        s.set_language_index(2);
        s.set_effort_index(2);
        s.set_auto_lean(true);
        s.set_asr_on(true);
        s.set_attached_mic(c.t(LONG_MIC_DE, LONG_MIC_EN));
    });
    add(&mut v, "settings-multilingual", Kind::Settings, |_, s, _| {
        s.set_page(1);
        s.set_language_index(2);
    });
    add(&mut v, "settings-custom-effort", Kind::Settings, |_, s, _| {
        s.set_page(1);
        s.set_effort_index(2);
        s.set_auto_lean(false);
    });
    add(&mut v, "settings-unsupported", Kind::Settings, |_, s, _| {
        s.set_page(1);
        s.set_filter_state(FilterState::Unsupported);
    });
    add(&mut v, "settings-ai-on", Kind::Settings, |_, s, _| {
        s.set_page(1);
        s.set_ai_features(true);
        s.set_asr_on(true);
    });
    add(&mut v, "noise", Kind::Settings, |_, s, _| s.set_page(2));
    add(&mut v, "noise-old-cpu", Kind::Settings, |_, s, _| {
        s.set_page(2);
        s.set_old_cpu(true);
        s.set_has_probe(true);
    });
    add(&mut v, "noise-not-attached", Kind::Settings, |_, s, _| {
        s.set_page(2);
        s.set_filter_state(FilterState::NotAttached);
    });
    add(&mut v, "noise-unsupported", Kind::Settings, |_, s, _| {
        s.set_page(2);
        s.set_filter_state(FilterState::Unsupported);
    });
    for (probe, name) in [
        (ProbeState::Idle, "idle"),
        (ProbeState::Recording, "recording"),
        (ProbeState::Before, "before"),
        (ProbeState::After, "after"),
    ] {
        add(&mut v, format!("noise-probe-{name}"), Kind::Settings, move |_, s, c| {
            s.set_page(2);
            s.set_has_probe(probe != ProbeState::Idle);
            s.set_probe_state(probe);
            s.set_probe_error(c.t(
                "Die Aufnahme ging nicht: das Mikrofon liefert keinen Ton.",
                "The recording failed: the microphone delivers no sound.",
            ));
        });
    }
    add(&mut v, "noise-listening", Kind::Settings, |_, s, _| {
        s.set_page(2);
        s.set_listening(true);
    });
    add(&mut v, "voice", Kind::Settings, |_, s, _| s.set_page(3));
    add(&mut v, "voice-custom", Kind::Settings, |_, s, _| {
        s.set_page(3);
        s.set_look_index(4);
        s.set_voice_is_default(false);
        s.set_body_db(1.5);
        s.set_presence_db(-2.0);
    });
    add(&mut v, "voice-off", Kind::Settings, |_, s, _| {
        s.set_page(3);
        s.set_voice_enabled(false);
        s.set_look_index(0);
    });
    add(&mut v, "dictation", Kind::Settings, |_, s, _| s.set_page(4));
    for (name, capture, custom, refusal) in [
        ("listening", 1, false, ""),
        ("custom", 0, true, ""),
        ("refused", 0, false, "x"),
    ] {
        add(
            &mut v,
            format!("dictation-key-{name}"),
            Kind::Settings,
            move |_, s, c| {
                s.set_page(4);
                s.set_key_capture(capture);
                s.set_key_custom(custom);
                s.set_key_name(if custom { "Pause" } else { "F9" }.into());
                if !refusal.is_empty() {
                    s.set_key_refusal(c.t(
                        "F8 ist schon fürs Aufnehmen belegt.",
                        "F8 is already used for recording.",
                    ));
                    s.set_key_warning(c.t(
                        "Diese Taste ist in manchen Programmen belegt.",
                        "Some programs use this key themselves.",
                    ));
                }
            },
        );
    }
    add(&mut v, "dictation-key-note", Kind::Settings, |_, s, c| {
        s.set_page(4);
        s.set_key_note(c.t(
            "Dein Desktop muss die Taste einmal erlauben: Einstellungen, Tastatur, Tastenkürzel.",
            "Your desktop has to allow the key once: Settings, Keyboard, Shortcuts.",
        ));
        s.set_key_button(c.t("Taste einrichten", "Set up key"));
        s.set_key_toggle(true);
    });
    add(&mut v, "dictation-key-fixed", Kind::Settings, |_, s, c| {
        s.set_page(4);
        s.set_key_fixed(true);
        s.set_key_button(c.t("Taste ändern", "Change key"));
    });
    add(&mut v, "recording-settings", Kind::Settings, |_, s, _| s.set_page(5));
    add(&mut v, "recording-settings-refused", Kind::Settings, |ui, s, c| {
        s.set_page(5);
        s.set_memo_key_custom(true);
        s.set_memo_key_label("Pause".into());
        s.set_key_capture(2);
        s.set_key_refusal(c.t(
            "F9 ist schon fürs Diktieren belegt.",
            "F9 is already used for dictation.",
        ));
        s.set_memo_key_note(c.t(
            "Ein anderes Kürzel von dir belegt F8 ebenfalls.",
            "Another shortcut of yours has F8 as well.",
        ));
        ui.set_memo_folder(c.t(
            "Dokumente\\Meine Sprachaufzeichnungen\\2026",
            "Documents/My voice recordings/2026",
        ));
    });
    add(&mut v, "recording-settings-fixed", Kind::Settings, |_, s, c| {
        s.set_page(5);
        s.set_key_fixed(true);
        s.set_key_button(c.t("Taste ändern", "Change key"));
        s.set_memo_can_hold(false);
    });
    add(&mut v, "speech-server-off", Kind::Settings, |_, s, _| s.set_page(7));
    add(&mut v, "speech-server-on", Kind::Settings, |_, s, c| {
        s.set_page(7);
        s.set_asr_on(true);
        s.set_asr_url("http://192.168.1.11:8081".into());
        s.set_asr_models(strings(&["whisper-large-v3-turbo", "parakeet-tdt-0.6b-v3"]));
        s.set_asr_note(c.t(
            "Funktioniert (0,4 s für eine Sekunde Stille).",
            "Works (0.4 s for a second of silence).",
        ));
    });
    add(&mut v, "speech-server-found", Kind::Settings, |_, s, c| {
        s.set_page(7);
        s.set_asr_on(true);
        s.set_asr_found("Speaches".into());
        s.set_asr_note(c.t(
            "Der Server antwortet nicht. Läuft er, und stimmt die Adresse?",
            "The server does not answer. Is it running, and is the address right?",
        ));
    });
    add(&mut v, "local-ai", Kind::Settings, |_, s, _| {
        s.set_page(8);
        s.set_ai_features(true);
        s.set_prompt_text(kyrr_server::rework::DEFAULT_PROMPTS[0].into());
        s.set_prompt_saved(kyrr_server::rework::DEFAULT_PROMPTS[0].into());
    });
    add(&mut v, "local-ai-tips", Kind::Settings, |_, s, c| {
        s.set_page(8);
        s.set_ai_features(true);
        s.set_rework_tips(true);
        s.set_rework_url("http://192.168.1.10:1234".into());
        s.set_rework_models(strings(&["gemma-4-26B-A4B-it"]));
        s.set_rework_ready(true);
        s.set_rework_found("Ollama".into());
        s.set_rework_note(c.t(
            "Funktioniert (1,1 s): „Ich habe heute die Bremsbeläge am Auto gewechselt.“",
            "Works (1.1 s): “Ich habe heute die Bremsbeläge am Auto gewechselt.”",
        ));
        s.set_prompt_index(3);
        s.set_prompt_own(true);
        s.set_prompt_text(kyrr_server::rework::DEFAULT_PROMPTS[3].into());
        s.set_prompt_saved(kyrr_server::rework::DEFAULT_PROMPTS[2].into());
    });

    // The first-run setup.
    add(&mut v, "setup-1", Kind::Setup, |ui, s, _| {
        ui.set_setup_mode(true);
        s.set_filter_state(FilterState::NotAttached);
    });
    add(&mut v, "setup-1-error", Kind::Setup, |ui, s, c| {
        ui.set_setup_mode(true);
        s.set_filter_state(FilterState::Error);
        s.set_error_text(c.t(
            "Das Mikrofon lässt sich nicht öffnen. Ist es von einer anderen App gesperrt?",
            "The microphone cannot be opened. Is another app holding it?",
        ));
        s.set_attached_mic(c.t(LONG_MIC_DE, LONG_MIC_EN));
    });
    add(&mut v, "setup-1-no-pipewire", Kind::Setup, |ui, s, c| {
        ui.set_setup_mode(true);
        s.set_linux(true);
        s.set_filter_state(FilterState::Unsupported);
        s.set_pipewire_button(true);
        s.set_pipewire_note(c.t(
            "PipeWire fehlt. kyrr kann es für dich einrichten, dafür fragt dein System nach dem Passwort.",
            "PipeWire is missing. kyrr can set it up for you; your system asks for the password.",
        ));
    });
    add(&mut v, "setup-2", Kind::Setup, |ui, s, _| {
        ui.set_setup_mode(true);
        ui.set_setup_step(1);
        s.set_dictation_state(DictationState::Downloading);
        s.set_download_progress(0.42);
    });
    add(&mut v, "setup-2-no-model", Kind::Setup, |ui, s, _| {
        ui.set_setup_mode(true);
        ui.set_setup_step(1);
        s.set_dictation_state(DictationState::NoModel);
    });
    add(&mut v, "setup-2-server", Kind::Setup, |ui, s, c| {
        ui.set_setup_mode(true);
        ui.set_setup_step(1);
        s.set_asr_on(true);
        s.set_asr_url("http://192.168.1.11:8081".into());
        s.set_asr_note(c.t(
            "Funktioniert (0,4 s für eine Sekunde Stille).",
            "Works (0.4 s for a second of silence).",
        ));
    });
    add(&mut v, "setup-2-multilingual", Kind::Setup, |ui, s, _| {
        ui.set_setup_mode(true);
        ui.set_setup_step(1);
        s.set_language_index(2);
        s.set_dictation_state(DictationState::NoModel);
    });
    add(&mut v, "setup-2-key-note", Kind::Setup, |ui, s, c| {
        ui.set_setup_mode(true);
        ui.set_setup_step(1);
        s.set_key_note(c.t(
            "Dein Desktop muss die Taste einmal erlauben: Einstellungen, Tastatur, Tastenkürzel.",
            "Your desktop has to allow the key once: Settings, Keyboard, Shortcuts.",
        ));
    });
    for (key, typing) in [(0, 0), (1, 0), (3, 0), (2, 0), (2, 1), (2, 3), (2, 2)] {
        add(
            &mut v,
            format!("setup-2-guide-key{key}-typing{typing}"),
            Kind::Setup,
            move |ui, s, _| {
                ui.set_setup_mode(true);
                ui.set_setup_step(1);
                s.set_linux(true);
                s.set_guide(true);
                s.set_key_state(key);
                s.set_typing_state(typing);
                s.set_key_tried(key == 2 && typing == 2);
            },
        );
    }
    for effort in [1, 0] {
        add(
            &mut v,
            format!("setup-3-effort{effort}"),
            Kind::Setup,
            move |ui, s, _| {
                ui.set_setup_mode(true);
                ui.set_setup_step(2);
                s.set_effort_index(effort);
            },
        );
    }

    // The voice memos.
    add(&mut v, "memos-few", Kind::Memos, |_, s, c| {
        s.set_memo_items(ModelRc::new(VecModel::from(memo_items(c, false))));
        s.set_memo_count(6);
        s.set_memo_can_write(true);
        s.set_memo_playing(1);
        s.set_memo_play_fraction(0.35);
        s.set_memo_play_time("0:17 / 0:48".into());
        s.set_page(6);
    });
    add(&mut v, "memos-many", Kind::Memos, |_, s, c| {
        s.set_memo_items(ModelRc::new(VecModel::from(memo_items(c, true))));
        s.set_memo_count(60);
        s.set_memo_can_write(true);
        s.set_page(6);
    });
    add(&mut v, "memos-empty", Kind::Memos, |_, s, _| {
        s.set_memo_count(0);
        s.set_page(6);
    });
    add(&mut v, "memos-nothing-found", Kind::Memos, |_, s, c| {
        s.set_memo_count(6);
        s.set_memo_query(c.t("Steuer", "tax"));
        s.set_page(6);
    });
    add(&mut v, "memos-recording-confirm", Kind::Memos, |_, s, c| {
        s.set_memo_items(ModelRc::new(VecModel::from(memo_items(c, false))));
        s.set_memo_count(6);
        s.set_memo_can_write(true);
        s.set_memo_recording(true);
        s.set_memo_elapsed("1:07".into());
        s.set_memo_confirm(4);
        s.set_memo_editing(2);
        s.set_memo_message(c.t("Das Mikrofon ist gerade belegt.", "The microphone is busy right now."));
        s.set_page(6);
    });
    add(&mut v, "memo-text", Kind::Text, |_, s, c| open_text(s, c, 0));
    for stamps in [1, 2] {
        add(
            &mut v,
            format!("memo-text-stamps{stamps}"),
            Kind::Text,
            move |_, s, c| open_text(s, c, stamps),
        );
    }
    add(&mut v, "memo-text-empty", Kind::Text, |_, s, c| {
        open_text(s, c, 0);
        s.set_memo_sheet_text("".into());
    });
    add(&mut v, "memo-text-long-title", Kind::Text, |_, s, c| {
        open_text(s, c, 0);
        s.set_memo_sheet_title(c.t(
            "Besprechung mit dem Steuerberater über die Homeoffice Pauschale und alle Belege",
            "Meeting with the tax adviser about the home office allowance and all the receipts",
        ));
        s.set_memo_playing(2);
        s.set_memo_play_time("0:17 / 2:31".into());
    });
    add(&mut v, "memo-text-ai-ready", Kind::Text, |_, s, c| {
        open_text(s, c, 0);
        s.set_memo_rework_ready(true);
        s.set_memo_rework_note(c.t(
            "Wählen, was mit dem Text geschehen soll.",
            "Choose what should happen to the text.",
        ));
    });
    add(&mut v, "memo-text-ai-busy", Kind::Text, |_, s, c| {
        open_text(s, c, 1);
        s.set_memo_rework_ready(true);
        s.set_memo_rework_action(3);
        s.set_memo_rework_target(1);
        s.set_memo_rework_busy(true);
        s.set_memo_rework_progress(0.4);
        s.set_memo_rework_note(c.t("Der Text wird überarbeitet …", "Reworking the text …"));
    });
    add(&mut v, "memo-text-ai-done", Kind::Text, |_, s, c| {
        open_text(s, c, 0);
        reworked(s, c);
    });
    add(&mut v, "memo-compare", Kind::Compare, |_, s, c| {
        open_text(s, c, 0);
        reworked(s, c);
        s.set_memo_sheet_compare(ModelRc::new(VecModel::from(compare_pairs(c.de))));
        s.set_memo_sheet_view(1);
    });
    add(&mut v, "memo-compare-empty", Kind::Compare, |_, s, c| {
        open_text(s, c, 0);
        reworked(s, c);
        s.set_memo_sheet_compare(ModelRc::new(VecModel::from(Vec::<ComparePair>::new())));
        s.set_memo_sheet_view(1);
    });

    v
}

// ---------------------------------------------------------------------------
// The news window.
// ---------------------------------------------------------------------------

struct NewsCase {
    name: &'static str,
    mode: i32,
    version: String,
    text: String,
}

/// The bullets of a NEWS.md section in the app's words (`update::section`):
/// "• item" lines, the items of the other system left out.
fn news_bullets(block: &str, linux: bool) -> String {
    let mut items: Vec<(Option<&str>, String)> = Vec::new();
    for line in block.lines() {
        if let Some(item) = line.strip_prefix("- ") {
            let mark = ["Windows", "Linux"]
                .into_iter()
                .find(|m| item.starts_with(&format!("({m}) ")));
            let text = mark.map_or(item, |m| &item[m.len() + 3..]);
            items.push((mark, text.trim().to_string()));
        } else if !line.trim().is_empty() {
            if let Some((_, last)) = items.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        }
    }
    let own = if linux { "Linux" } else { "Windows" };
    items
        .into_iter()
        .filter(|(mark, _)| mark.is_none_or(|m| m == own))
        .map(|(_, text)| format!("• {text}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One version's summary, English or German.
fn news_section(markdown: &str, version: &str, german: bool, linux: bool) -> String {
    let mut lines = markdown.lines().skip_while(|l| l.trim_end() != format!("## {version}"));
    lines.next();
    let (mut english, mut deutsch, mut in_german) = (String::new(), String::new(), false);
    for line in lines {
        if line.starts_with("## ") {
            break;
        }
        if line.trim_end() == "### Deutsch" {
            in_german = true;
            continue;
        }
        let part = if in_german { &mut deutsch } else { &mut english };
        part.push_str(line);
        part.push('\n');
    }
    news_bullets(if german { &deutsch } else { &english }, linux)
}

fn news_cases(c: Ctx) -> Vec<NewsCase> {
    let markdown = include_str!("../../../NEWS.md");
    let linux = c.os == Os::Linux;
    let version = env!("CARGO_PKG_VERSION");
    let mut all = String::new();
    for v in markdown.lines().filter_map(|l| l.strip_prefix("## ")).map(str::trim) {
        let numbers: Vec<u32> = v.split('.').filter_map(|n| n.parse().ok()).collect();
        // "Unreleased" is not shown; Linux starts with 0.5.3.
        if numbers.len() != 3 || (linux && numbers[..] < [0, 5, 3][..]) {
            continue;
        }
        let text = news_section(markdown, v, c.de, linux);
        if !text.is_empty() {
            if !all.is_empty() {
                all.push_str("\n\n");
            }
            all.push_str(&format!("kyrr {v}\n{text}"));
        }
    }
    let current = news_section(markdown, version, c.de, linux);
    vec![
        NewsCase {
            name: "news-offer",
            mode: 0,
            version: "0.10.2".into(),
            // A release asks for 2000 characters at most.
            text: current.clone(),
        },
        NewsCase {
            name: "news-current",
            mode: 1,
            version: version.into(),
            text: current,
        },
        NewsCase {
            name: "news-all",
            mode: 2,
            version: version.into(),
            text: all,
        },
    ]
}

// ---------------------------------------------------------------------------
// Looking at the pictures.
// ---------------------------------------------------------------------------

/// Whether anything but `bg` is in the columns `xs` of row `y`, for the
/// pictures where the window's own colour is not the field's.
fn ink_between(img: &Image, y: usize, xs: std::ops::Range<usize>, bg: [u8; 3], skip: std::ops::Range<usize>) -> bool {
    xs.into_iter().any(|x| !skip.contains(&x) && differs(img.px(x, y), bg))
}

/// Content in the margins left and right of the window: wider than the
/// window lets it be. The margin is empty in the app (18 px of padding).
fn margin_problem(img: &Image, scale: f32) -> Option<String> {
    let bg = img.px(0, 0);
    let width = (MARGIN * scale).round() as usize;
    let mut left = usize::MAX;
    let mut right = usize::MAX;
    let mut rows = 0;
    let mut first = None;
    for y in 0..img.height {
        let mut hit = false;
        for x in 0..width.min(img.width) {
            if differs(img.px(x, y), bg) {
                left = left.min(x);
                hit = true;
            }
            let mirror = img.width - 1 - x;
            if differs(img.px(mirror, y), bg) {
                right = right.min(x);
                hit = true;
            }
        }
        if hit {
            rows += 1;
            first.get_or_insert(y);
        }
    }
    let first = first?;
    let logical = |px: usize| px as f32 / scale;
    let mut sides = Vec::new();
    if left != usize::MAX {
        sides.push(format!("{:.1} px from the left edge", logical(left)));
    }
    if right != usize::MAX {
        sides.push(format!("{:.1} px from the right edge", logical(right)));
    }
    Some(format!(
        "content reaches into the margin: {} (the padding is 18 px; {rows} rows, first at y={:.0} px)",
        sides.join(", "),
        logical(first)
    ))
}

/// The memo text's field in a picture of the window: its first and one past
/// its last row, found as the longest stretch of one colour (not the
/// window's) a little inside the left padding.
fn find_field(img: &Image, scale: f32) -> Option<(usize, usize)> {
    let x = (28.0 * scale).round() as usize;
    let bg = img.px(0, 0);
    let (mut best, mut start) = ((0, 0), 0);
    for y in 1..=img.height {
        // The field is only a shade off the window's colour.
        if y == img.height || differs_by(img.px(x, y), img.px(x, start), 3) {
            if differs_by(img.px(x, start), bg, 3) && y - start > best.1 - best.0 {
                best = (start, y);
            }
            start = y;
        }
    }
    (best.1 - best.0 >= (50.0 * scale) as usize).then_some(best)
}

/// How a memo's text field fares at one window size.
struct FieldFacts {
    /// Height of the field, in physical pixels.
    height: usize,
    /// The height the text would need, in physical pixels; None for no text.
    needed: Option<usize>,
}

/// The text field at the window's current size, and what its text needs
/// (seen in a window much taller, where it does not scroll). The tall
/// picture is made once for each width.
fn field_facts(rig: &Rig, needed_at: &mut BTreeMap<u32, Option<usize>>) -> Option<FieldFacts> {
    let scale = rig.scale.get();
    let img = rig.draw();
    let (top, bottom) = find_field(&img, scale)?;
    let size = rig.window.size();
    let needed = *needed_at.entry(size.width).or_insert_with(|| {
        let (width, height) = (size.width as f32 / scale, size.height as f32 / scale);
        rig.set_size(width, 2400.0);
        let tall = rig.draw();
        rig.set_size(width, height);
        let (t, b) = find_field(&tall, scale)?;
        let field = tall.px((28.0 * scale).round() as usize, t + 2);
        // Inside the field's rounded corners and the focus border around it.
        let margin = (30.0 * scale) as usize;
        let border = (4.0 * scale) as usize;
        // The line between the two columns of the comparison is as tall as
        // the field; it is not text.
        let middle = tall.width / 2 - 3..tall.width / 2 + 4;
        let last = (t + border..b - border)
            .rev()
            .find(|&y| ink_between(&tall, y, margin..tall.width - margin, field, middle.clone()))?;
        // The text, and the same air below it as above.
        Some(last + 1 - t + (12.0 * scale) as usize)
    });
    Some(FieldFacts {
        height: bottom - top,
        needed,
    })
}

/// Rows from the bottom of the window up that the Close button of the news
/// window takes: its first row.
fn last_block_top(img: &Image, scale: f32) -> Option<usize> {
    let bg = img.px(0, 0);
    let (x0, x1) = ((18.0 * scale) as usize, img.width - (18.0 * scale) as usize);
    let ink = |y: usize| img.row_has_ink(y, x0..x1, bg);
    let bottom = (0..img.height).rev().find(|&y| ink(y))?;
    let mut top = bottom;
    while top > 0 && ink(top - 1) {
        top -= 1;
    }
    Some(top)
}

/// The last row of text above the button of the news window.
fn news_text_bottom(img: &Image, scale: f32) -> Option<usize> {
    let bg = img.px(0, 0);
    let (x0, x1) = ((18.0 * scale) as usize, img.width - (18.0 * scale) as usize);
    let button = last_block_top(img, scale)?;
    (0..button).rev().find(|&y| img.row_has_ink(y, x0..x1, bg))
}

// ---------------------------------------------------------------------------
// The run.
// ---------------------------------------------------------------------------

/// Relative luminance of an sRGB colour, 0 (black) to 1 (white).
fn luminance(c: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// The contrast ratio of two colours, 1 (the same) to 21 (black on white).
fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (a, b) = (luminance(a) + 0.05, luminance(b) + 0.05);
    a.max(b) / a.min(b)
}

/// Edge of a tile of the readability comparison, in logical pixels.
const TILE: usize = 16;
/// A tile whose strongest ink has this contrast to its background is
/// readable; below `LOST` it is as good as gone.
const READABLE: f32 = 4.5;
const LOST: f32 = 1.5;
/// So many tiles that are readable in one look and lost in the other make a
/// problem (an icon's edge or a shadow alone do not).
const LOST_TILES: usize = 8;

/// For each tile of a picture the contrast of its strongest ink to the
/// colour the tile mostly has (1 for a tile of one colour).
struct TileMap {
    width: usize,
    height: usize,
    cols: usize,
    ratio: Vec<f32>,
}

impl TileMap {
    fn of(img: &Image, scale: f32) -> TileMap {
        let side = ((TILE as f32 * scale).round() as usize).max(1);
        // Every pixel of a 1x picture, every second one at 2x: text strokes are thicker than that.
        let step = (scale.floor() as usize).max(1);
        let (cols, rows) = (img.width.div_ceil(side), img.height.div_ceil(side));
        let mut ratio = Vec::with_capacity(cols * rows);
        for ty in 0..rows {
            for tx in 0..cols {
                let (x0, y0) = (tx * side, ty * side);
                let (x1, y1) = ((x0 + side).min(img.width), (y0 + side).min(img.height));
                // The tile's colour: the one most of 25 samples share (ink is the minority).
                let samples: Vec<[u8; 3]> = (0..25)
                    .map(|i| img.px(x0 + (x1 - x0 - 1) * (i % 5) / 4, y0 + (y1 - y0 - 1) * (i / 5) / 4))
                    .collect();
                let bg = *samples
                    .iter()
                    .max_by_key(|c| samples.iter().filter(|o| !differs(**c, **o)).count())
                    .expect("25 samples");
                let mut best = 1.0f32;
                for y in (y0..y1).step_by(step) {
                    for x in (x0..x1).step_by(step) {
                        let px = img.px(x, y);
                        if differs(px, bg) {
                            best = best.max(contrast(px, bg));
                        }
                    }
                }
                ratio.push(best);
            }
        }
        TileMap {
            width: img.width,
            height: img.height,
            cols,
            ratio,
        }
    }

    /// The tiles readable here and lost in the other map (`None` when the
    /// pictures differ in size), as the logical position of the first.
    fn lost_in(&self, other: &TileMap, scale: f32) -> Option<(usize, Option<(f32, f32)>)> {
        if (self.width, self.height) != (other.width, other.height) {
            return None;
        }
        let side = TILE as f32 * scale;
        let hits: Vec<usize> = (0..self.ratio.len())
            .filter(|&i| self.ratio[i] >= READABLE && other.ratio[i] < LOST)
            .collect();
        let first = hits.first().map(|&i| {
            (
                (i % self.cols) as f32 * side / scale,
                (i / self.cols) as f32 * side / scale,
            )
        });
        Some((hits.len(), first))
    }
}

/// The tile maps of the first look of each pair, waiting for the second:
/// by page, language, scale and group of looks.
type Tiles = BTreeMap<(String, &'static str, u32, &'static str), (Look, TileMap)>;

/// What the run has found so far.
#[derive(Default)]
struct Findings {
    problems: Vec<Problem>,
    tiles: Tiles,
}

/// What is wrong with a picture of a page in its look: the background of the
/// wrong brightness, and, as the second of a pair of looks, anything that
/// is readable in one look and lost in the other.
fn look_check(tiles: &mut Tiles, page: &str, c: Ctx, scale: f32, img: &Image) -> Vec<String> {
    let mut found = Vec::new();
    let bg = img.px(0, 0);
    if !c.look.background_fits(bg) {
        found.push(format!(
            "the window's background is #{:02x}{:02x}{:02x}, not {} as the {} look is",
            bg[0],
            bg[1],
            bg[2],
            if c.look.dark { "dark" } else { "light" },
            c.look.name
        ));
    }
    let map = TileMap::of(img, scale);
    let key = (page.to_string(), c.lang(), (scale * 100.0) as u32, c.look.group);
    let Some((other, theirs)) = tiles.remove(&key) else {
        tiles.insert(key, (c.look, map));
        return found;
    };
    let Some((first_lost, first_at)) = theirs.lost_in(&map, scale) else {
        found.push(format!(
            "the {} and the {} look do not lay the page out the same ({}x{} and {}x{} px)",
            other.name, c.look.name, theirs.width, theirs.height, map.width, map.height
        ));
        return found;
    };
    let (second_lost, second_at) = map.lost_in(&theirs, scale).unwrap_or((0, None));
    for (count, at, readable, lost) in [
        (first_lost, first_at, other.name, c.look.name),
        (second_lost, second_at, c.look.name, other.name),
    ] {
        if count >= LOST_TILES {
            let (x, y) = at.unwrap_or_default();
            found.push(format!(
                "{count} places of the page are readable in the {readable} look and lost in the {lost} look (text in a fixed colour on the wrong background?), the first at x={x:.0}, y={y:.0} px"
            ));
        }
    }
    found
}

struct Problem {
    page: String,
    lang: &'static str,
    look: &'static str,
    scale: f32,
    screen: String,
    text: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "page={} look={} lang={} scale={} screen={}: {}",
            self.page, self.look, self.lang, self.scale, self.screen, self.text
        )
    }
}

struct Options {
    out: PathBuf,
    langs: Vec<&'static str>,
    scales: Vec<f32>,
    looks: Vec<Look>,
    os: Os,
    panel: f32,
    sheets: bool,
    /// Prints the measurements of every page, not only the problems.
    verbose: bool,
    /// Runs with sizing rules that are wrong on purpose (a window capped at
    /// 900 px, no room for the frame): the check must then find problems.
    selftest: bool,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        out: PathBuf::from("."),
        langs: LANGS.to_vec(),
        scales: SCALES.to_vec(),
        looks: Vec::new(),
        os: if cfg!(windows) { Os::Windows } else { Os::Linux },
        panel: DEFAULT_PANEL,
        sheets: true,
        verbose: false,
        selftest: false,
    };
    for arg in std::env::args().skip(1) {
        match arg.split_once('=') {
            Some(("lang", v)) => {
                o.langs = LANGS.into_iter().filter(|l| v.split(',').any(|x| x == *l)).collect();
            }
            Some(("scale", v)) => {
                o.scales = v
                    .split(',')
                    .map(|x| x.parse().map_err(|_| format!("scale={v}: not a list of numbers")))
                    .collect::<Result<_, _>>()?;
            }
            Some(("look", v)) => {
                o.looks = v
                    .split(',')
                    .map(|x| {
                        LOOKS
                            .iter()
                            .find(|l| l.name == x)
                            .copied()
                            .ok_or(format!("look={v}: no look {x}"))
                    })
                    .collect::<Result<_, _>>()?;
            }
            Some(("os", "windows")) => o.os = Os::Windows,
            Some(("os", "linux")) => o.os = Os::Linux,
            Some(("panel", v)) => o.panel = v.parse().map_err(|_| format!("panel={v}: not a number"))?,
            Some(("sheets", "off")) => o.sheets = false,
            Some(("verbose", "on")) => o.verbose = true,
            Some(("selftest", "on")) => o.selftest = true,
            _ if !arg.contains('=') => o.out = PathBuf::from(arg),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if o.looks.is_empty() {
        // Omarchy's themes only exist on Linux.
        o.looks = LOOKS
            .iter()
            .filter(|l| l.theme.is_none() || o.os == Os::Linux)
            .copied()
            .collect();
    }
    Ok(o)
}

/// Everything about one page at one language and scale, on every screen.
#[allow(clippy::too_many_arguments)]
fn check_page(
    rig: &Rig,
    sc: &Scenario,
    c: Ctx,
    o: &Options,
    rules: &Rules,
    reference: &mut BTreeMap<(&'static str, String), (f32, f32)>,
    sheets: &mut Sheets,
    found: &mut Findings,
) {
    let scale = rig.scale.get();
    let lang = c.lang();
    let mut report = |screen: &str, text: String| {
        found.problems.push(Problem {
            page: sc.name.clone(),
            lang,
            look: c.look.name,
            scale,
            screen: screen.into(),
            text,
        });
    };
    let ui = MainWindow::new().expect("the main window");
    let s = ui.global::<AppState>();
    baseline(&ui, &s, c);
    ui.invoke_use_scheme(c.look.dark);
    c.look.skin(&ui.global::<Skin>());
    (sc.set)(&ui, &s, c);
    ui.set_widest(1040.0 + sizing::FRAME_SIDES);
    ui.show().expect("show");
    // The width first, as main.rs does, then a window far taller than any
    // page, to see how tall the content wants to be.
    rig.set_size(ui.get_page_width(), 660.0);
    rig.set_size(ui.get_page_width(), 20000.0);
    let content = ui.get_content_height();
    // The picture of the page at the height it would get on a big screen.
    let biggest = Screen::new(o.os, (3840, 2160), scale, o.panel, rules);
    // A page that scrolls on a big screen is drawn whole, so its scrollbar
    // does not stand in the margin.
    let natural = match sc.kind {
        Kind::Text | Kind::Compare => window_height(sc.kind, content, &biggest, scale),
        _ => window_height(sc.kind, content, &biggest, scale).max(content) + 2.0,
    };
    rig.set_size(ui.get_page_width(), natural);
    if sc.drawn {
        let img = rig.draw();
        if let Some(text) = margin_problem(&img, scale) {
            report("any", text);
        }
        for text in look_check(&mut found.tiles, &sc.name, c, scale, &img) {
            report("any", text);
        }
        sheets.add(&sc.name, &img);
    }
    // The layout is the same at every scale, up to a wrapped word.
    let (first_scale, first_content) = *reference
        .entry((c.look.name, format!("{lang}/{}", sc.name)))
        .or_insert((scale, content));
    if (content - first_content).abs() > SCALE_SLACK {
        report(
            "any",
            format!(
                "the content is {content:.0} px tall at this scale and {first_content:.0} px at scale {first_scale}: the layout does not follow the scale"
            ),
        );
    }

    if o.verbose {
        eprintln!(
            "{lang} x{scale} {}: content {content:.2} px, {first_content:.2} px at x{first_scale}",
            sc.name
        );
    }
    let mut needed_at = BTreeMap::new();
    for size in SCREENS {
        let screen = Screen::new(o.os, size, scale, o.panel, rules);
        let label = screen.label();
        if sc.kind == Kind::Compare {
            ui.set_widest(screen.widest);
        }
        let width = ui.get_page_width();
        let want = intended_width(sc.kind, &screen);
        if (width - want).abs() > 0.5 {
            report(&label, format!("the page is {width} px wide, it should be {want} px"));
        }
        let physical = rig.phys(width) as f32;
        if (physical - want * scale).abs() > 1.0 {
            report(
                &label,
                format!(
                    "the window is {physical} physical px wide, not {want} x {scale} = {:.1}",
                    want * scale
                ),
            );
        }
        if width + sizing::FRAME_SIDES > screen.width as f32 + 0.5 {
            report(
                &label,
                format!(
                    "the window is {width} px wide (+ {} for its frame) on a screen {} px wide",
                    sizing::FRAME_SIDES,
                    screen.width
                ),
            );
        }
        let height = window_height(sc.kind, content, &screen, scale);
        let free = screen.usable;
        if height + o.os.frame() > free + 0.5 {
            report(
                &label,
                format!(
                    "the window is {height:.0} px tall + {} for its frame = {:.0} px, the screen has {free:.0} px free ({} minus {} for the {}): {:.0} px too tall",
                    o.os.frame(),
                    height + o.os.frame(),
                    screen.height,
                    screen.height as f32 - free,
                    if o.os == Os::Windows { "taskbar" } else { "panel" },
                    height + o.os.frame() - free
                ),
            );
        }
        // The scrollbar shows as soon as the content is a hair taller than
        // the window (the window is whole pixels).
        let px = window_px(o.os, sc.kind, height, scale);
        let seen = px as f32 / scale;
        if content > seen + 0.02 && content + sizing::FRAME <= free {
            report(
                &label,
                format!(
                    "the page scrolls ({content:.2} px of content in a window of {px} physical px = {seen:.2} px) although the screen has room for it"
                ),
            );
        }
        // The text field is found by its shade of the system's look.
        if matches!(sc.kind, Kind::Text | Kind::Compare) && c.look.theme.is_none() {
            rig.set_size(width, height);
            match field_facts(rig, &mut needed_at) {
                None => report(&label, "the text field of the memo was not found in the picture".into()),
                Some(f) => {
                    let (field, needed) = (f.height as f32 / scale, f.needed.map(|n| n as f32 / scale));
                    if o.verbose {
                        eprintln!(
                            "  {label}: window {height:.0} px, text field {field:.0} px, text needs {needed:?} px"
                        );
                    }
                    if field < TEXT_FIELD_MIN {
                        report(
                            &label,
                            format!("the memo text field is only {field:.0} px tall in a window of {height:.0} px"),
                        );
                    }
                    if let Some(needed) = needed.filter(|n| *n > field + 3.0) {
                        let wanted = height + needed - field;
                        if wanted + sizing::FRAME <= free {
                            report(
                                &label,
                                format!(
                                    "the memo text scrolls ({needed:.0} px of text in a field of {field:.0} px, window {height:.0} px) although the screen has room for a window of {wanted:.0} px ({free:.0} px free)"
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    ui.hide().expect("hide");
    // The comparison widened the window; the next page starts narrow.
    rig.set_size(380.0, 660.0);
}

/// The news window, which the app sizes to its text, up to what the screen
/// has room for.
fn check_news(
    rig: &Rig,
    case: &NewsCase,
    c: Ctx,
    o: &Options,
    rules: &Rules,
    sheets: &mut Sheets,
    found: &mut Findings,
) {
    let scale = rig.scale.get();
    let lang = c.lang();
    let win = NewsWindow::new().expect("the news window");
    win.invoke_use_scheme(c.look.dark);
    c.look.skin(&win.global::<Skin>());
    win.set_version(case.version.as_str().into());
    win.set_mode(case.mode);
    win.set_news(case.text.as_str().into());
    win.show().expect("show");
    let mut report = |screen: String, text: String| {
        found.problems.push(Problem {
            page: case.name.into(),
            lang,
            look: c.look.name,
            scale,
            screen,
            text,
        });
    };
    // The text's height at the window's width, as the app measures it: in a
    // tall window, so no scrollbar narrows the text.
    rig.set_size(sizing::NEWS_WIDTH, sizing::MAX_HEIGHT);
    let content = win.get_content_height();
    let height_on = |screen: &Screen| sizing::whole_pixels(sizing::news_height(content, screen.cap), scale);
    let biggest = Screen::new(o.os, (3840, 2160), scale, o.panel, rules);
    let natural = height_on(&biggest);
    win.set_wanted_height(natural);
    rig.set_size(sizing::NEWS_WIDTH, natural);
    let img = rig.draw();
    if let Some(text) = margin_problem(&img, scale) {
        report("any".into(), text);
    }
    for text in look_check(&mut found.tiles, case.name, c, scale, &img) {
        report("any".into(), text);
    }
    let viewport_bottom = last_block_top(&img, scale).map(|top| top as f32 - 12.0 * scale);
    sheets.add(case.name, &img);
    rig.set_size(sizing::NEWS_WIDTH, 6000.0);
    let tall = rig.draw();
    let text_bottom = news_text_bottom(&tall, scale);
    win.hide().expect("hide");
    // Where the text ends and the button starts is found by the system's
    // look of the button; Omarchy's square one is not what that looks for.
    if c.look.theme.is_none() {
        let (Some(viewport_bottom), Some(text_bottom)) = (viewport_bottom, text_bottom) else {
            report(
                "any".into(),
                "the text and the button of the news window were not found in the picture".into(),
            );
            return;
        };
        // The text's last line ends a little above its box.
        let missing = (text_bottom as f32 + 4.0 * scale - viewport_bottom) / scale;
        if o.verbose {
            eprintln!(
                "{lang} x{scale} {}: content {content:.2} px, window {natural:.2} px, {missing:.0} px hidden",
                case.name
            );
        }
        // A window exactly as tall as the text still shows 2 to 3 px "missing"
        // here: where the picture puts the last line is a few pixels off.
        if missing > 4.0 && content + sizing::FRAME <= biggest.usable {
            report(
            "all".into(),
            format!(
                "the news text scrolls ({missing:.0} px of it are hidden in the window of {} x {natural:.0} px that opens) although the screen has room for a window of {content:.0} px",
                sizing::NEWS_WIDTH
            ),
        );
        }
    }
    for size in SCREENS {
        let screen = Screen::new(o.os, size, scale, o.panel, rules);
        let height = height_on(&screen);
        if height + o.os.frame() > screen.usable + 0.5 {
            report(
                screen.label(),
                format!(
                    "the news window is {height:.0} px tall + {} for its frame, the screen has {:.0} px free: {:.0} px too tall",
                    o.os.frame(),
                    screen.usable,
                    height + o.os.frame() - screen.usable
                ),
            );
        }
        if height + 0.5 < content && content + sizing::FRAME <= screen.usable {
            report(
                screen.label(),
                format!(
                    "the news window is {height:.0} px tall for {content:.0} px of text, although the screen has room for it"
                ),
            );
        }
    }
}

fn main() {
    let o = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("layout: {e}");
            std::process::exit(2);
        }
    };
    let started = Instant::now();
    std::fs::create_dir_all(&o.out).expect("the output directory");
    let mut lines: Vec<String> = check_source();
    let rules = Rules { selftest: o.selftest };
    let rig = Rig::new();
    // The bundled translations only exist once a component has been made.
    let _first = MainWindow::new().expect("the main window");
    let mut found = Findings::default();
    let mut reference = BTreeMap::new();
    for &look in &o.looks {
        for lang in &o.langs {
            let de = *lang == "de";
            if let Err(e) = slint::select_bundled_translation(lang) {
                eprintln!("layout: no translation {lang}: {e}");
                std::process::exit(2);
            }
            let c = Ctx { de, os: o.os, look };
            let list = scenarios();
            for &scale in &o.scales {
                rig.set_scale(scale);
                let mut sheets = Sheets::new(&o.out, format!("sheet-{}-{lang}-{scale}", look.name), o.sheets);
                for sc in &list {
                    check_page(&rig, sc, c, &o, &rules, &mut reference, &mut sheets, &mut found);
                }
                for case in news_cases(c) {
                    check_news(&rig, &case, c, &o, &rules, &mut sheets, &mut found);
                }
                sheets.flush();
                eprintln!(
                    "layout: {} look, {lang} at scale {scale} done, {} problems so far",
                    look.name,
                    found.problems.len()
                );
            }
        }
    }
    lines.extend(merged(&found.problems));
    report(&o.out, &lines, started);
}

/// One line for the same problem on several screens.
fn merged(problems: &[Problem]) -> Vec<String> {
    let mut order: Vec<(String, Vec<&str>)> = Vec::new();
    for p in problems {
        let key = format!("page={} look={} lang={} scale={}", p.page, p.look, p.lang, p.scale);
        let text = format!("{key} screen=@: {}", p.text);
        match order.iter_mut().find(|(t, _)| *t == text) {
            Some((_, screens)) => screens.push(&p.screen),
            None => order.push((text, vec![&p.screen])),
        }
    }
    order
        .into_iter()
        .map(|(text, screens)| {
            let all = if screens.len() == SCREENS.len() {
                "all".to_string()
            } else {
                screens.join(",")
            };
            text.replacen('@', &all, 1)
        })
        .collect()
}

/// Prints the problems (all of them go to `problems.txt`) and ends the run.
fn report(out: &Path, lines: &[String], started: Instant) -> ! {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(out.join("problems.txt"), if lines.is_empty() { "" } else { &text }).expect("problems.txt");
    const SHOWN: usize = 400;
    for line in lines.iter().take(SHOWN) {
        println!("{line}");
    }
    if lines.len() > SHOWN {
        println!(
            "... and {} more in {}",
            lines.len() - SHOWN,
            out.join("problems.txt").display()
        );
    }
    println!(
        "layout: {} problem(s), {:.0} s; pages and contact sheets in {}",
        lines.len(),
        started.elapsed().as_secs_f32(),
        out.display()
    );
    std::process::exit(i32::from(!lines.is_empty()));
}
