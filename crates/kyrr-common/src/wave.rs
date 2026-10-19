//! The dictation wave on the screen: a small grey pill (dark or light, as
//! the system is) near the bottom of the screen with kyrr's green bars in
//! it. While kyrr listens, each bar follows one pitch band of the voice,
//! low sounds in the middle and high ones at the sides; while it writes, a
//! slow wave runs through them. It fades in and out and is gone between
//! dictations.
//!
//! The drawing is done here, in plain Rust, so Windows and Linux show the
//! same picture; each system only puts the pixels on the screen (a
//! [`Surface`]). The surface is opened for each dictation and closed after
//! it, so nothing is kept on the screen or in a compositor between them.

use std::f32::consts::TAU;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

/// The pill's size in logical pixels.
pub const WIDTH: u32 = 144;
pub const HEIGHT: u32 = 56;

/// From the bottom of the screen (above the taskbar or panel) to the pill,
/// logical pixels.
pub const MARGIN: u32 = 40;

const BARS: usize = 13;
/// One band per distance from the middle bar (the middle bar is the lowest).
const BANDS: usize = BARS / 2 + 1;
const BAR_WIDTH: f32 = 4.0;
const BAR_GAP: f32 = 5.0;
const BAR_MIN: f32 = 6.0;
const BAR_MAX: f32 = 40.0;

/// Straight (not premultiplied) colours, 0..1, with their opacity.
struct Colors {
    background: [f32; 4],
    edge: [f32; 4],
    bar: [f32; 4],
}

/// The window's own greys and greens, for a dark and a light system.
const DARK: Colors = Colors {
    background: [0.165, 0.168, 0.176, 0.88],
    edge: [1.0, 1.0, 1.0, 0.10],
    bar: [0.369, 0.918, 0.831, 1.0],
};
const LIGHT: Colors = Colors {
    background: [0.965, 0.968, 0.975, 0.88],
    edge: [0.0, 0.0, 0.0, 0.12],
    bar: [0.078, 0.722, 0.651, 1.0],
};

/// The voice is heard in the newest 32 ms of the microphone.
const LISTEN_MS: u32 = 32;
/// Most samples ever looked at (32 ms at up to 64 kHz).
pub const TAIL: usize = 2048;
/// Pitch bands, Hz: from the lowest bar's band to the highest.
const BAND_LOW: f32 = 150.0;
const BAND_HIGH: f32 = 5500.0;
/// Speech falls off towards the high end; this lifts it, dB per band.
const TILT_DB: f32 = 1.2;

const FADE_IN: f32 = 0.15;
const FADE_OUT: f32 = 0.25;

/// About 30 pictures a second: smooth enough for a slow wave, and little
/// work for the CPU.
const FRAME: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Hidden,
    /// Recording: the bars follow the voice.
    Listening,
    /// Recognising and typing: a quicker wave.
    Writing,
}

/// Where the pixels go: a window above all others that never takes the
/// focus and lets clicks through.
pub trait Surface {
    /// Physical pixels per logical one, for the next picture.
    fn scale(&mut self) -> f32;
    /// Whether the system is set to dark, asked once when the surface opens.
    fn dark(&mut self) -> bool;
    /// Shows a picture of `width` × `height` premultiplied ARGB pixels, row
    /// by row (`pixels` holds at least that many). False when the surface is
    /// gone (the desktop closed it).
    fn show(&mut self, pixels: &[u32], width: u32, height: u32) -> bool;
}

/// The size of the picture at `scale`, in physical pixels.
pub fn size(scale: f32) -> (u32, u32) {
    let scale = scale.clamp(1.0, 4.0);
    (
        (WIDTH as f32 * scale).ceil() as u32,
        (HEIGHT as f32 * scale).ceil() as u32,
    )
}

/// What the wave looks like right now.
#[derive(Clone, Debug)]
pub struct Wave {
    mode: Mode,
    /// The last mode that showed something, so the fade-out keeps its look.
    look: Mode,
    /// Seconds since the wave appeared.
    t: f32,
    /// A dark system (dark pill) or a light one.
    dark: bool,
    /// Opacity of the whole picture, 0..1.
    alpha: f32,
    /// Bar heights, 0..1, eased towards their target.
    bars: [f32; BARS],
}

impl Default for Wave {
    fn default() -> Self {
        Wave {
            mode: Mode::Hidden,
            look: Mode::Listening,
            t: 0.0,
            dark: true,
            alpha: 0.0,
            bars: [0.0; BARS],
        }
    }
}

/// Splits the newest bit of the microphone into pitch bands (Goertzel: a
/// few multiplications per sample and band, no FFT) as 0..1 bar heights.
pub struct Analyzer {
    rate: u32,
    window: Vec<f32>,
    /// Goertzel coefficients, three pitches in each band.
    coefs: [[f32; 3]; BANDS],
    /// Turns a pitch's power into the amplitude of a sine that size.
    norm: f32,
}

impl Analyzer {
    pub fn new(rate: u32) -> Analyzer {
        let rate = rate.max(8000);
        let n = ((rate * LISTEN_MS / 1000) as usize).clamp(64, TAIL);
        let window: Vec<f32> = (0..n)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / (n - 1) as f32).cos())
            .collect();
        let norm = 2.0 / window.iter().sum::<f32>();
        let edge = |b: f32| BAND_LOW * (BAND_HIGH / BAND_LOW).powf(b / BANDS as f32);
        let mut coefs = [[0.0; 3]; BANDS];
        for (b, band) in coefs.iter_mut().enumerate() {
            for (k, coef) in band.iter_mut().enumerate() {
                // A quarter, half and three quarters into the band.
                let hz = edge(b as f32 + (k + 1) as f32 / 4.0);
                *coef = 2.0 * (TAU * hz.min(rate as f32 * 0.45) / rate as f32).cos();
            }
        }
        Analyzer {
            rate,
            window,
            coefs,
            norm,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Bar heights of the voice in `tail` (newest sample last); all zero
    /// while it is quieter than a quiet room.
    pub fn bands(&self, tail: &[f32]) -> [f32; BANDS] {
        let n = self.window.len();
        let mut out = [0.0; BANDS];
        let Some(tail) = tail.get(tail.len().saturating_sub(n)..).filter(|t| t.len() == n) else {
            return out;
        };
        for (b, level) in out.iter_mut().enumerate() {
            let mut power = 0.0;
            for &coef in &self.coefs[b] {
                let (mut s1, mut s2) = (0.0f32, 0.0f32);
                for (x, w) in tail.iter().zip(&self.window) {
                    let s0 = x * w + coef * s1 - s2;
                    s2 = s1;
                    s1 = s0;
                }
                power += s1 * s1 + s2 * s2 - coef * s1 * s2;
            }
            let amplitude = (power / 3.0).max(0.0).sqrt() * self.norm;
            let db = 20.0 * amplitude.max(1e-6).log10();
            *level = ((db + TILT_DB * b as f32 + 60.0) / 30.0).clamp(0.0, 1.0);
        }
        out
    }
}

impl Wave {
    pub fn set_mode(&mut self, mode: Mode) {
        if mode != Mode::Hidden {
            self.look = mode;
        }
        self.mode = mode;
    }

    /// Something to show: on, or still fading out.
    pub fn visible(&self) -> bool {
        self.mode != Mode::Hidden || self.alpha > 0.0
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
    }

    /// Moves on by `dt` seconds; `voice` is what the microphone hears right
    /// now, by pitch band.
    pub fn step(&mut self, dt: f32, voice: &[f32; BANDS]) {
        let dt = dt.clamp(0.0, 0.1);
        self.t += dt;
        self.alpha = if self.mode == Mode::Hidden {
            (self.alpha - dt / FADE_OUT).max(0.0)
        } else {
            (self.alpha + dt / FADE_IN).min(1.0)
        };
        // Neighbouring bands blend, so the picture stays smooth.
        let mut mix = *voice;
        for b in 0..BANDS {
            let lo = voice[b.saturating_sub(1)];
            let hi = voice[(b + 1).min(BANDS - 1)];
            mix[b] = if b == 0 || b == BANDS - 1 {
                0.75 * voice[b] + 0.25 * if b == 0 { hi } else { lo }
            } else {
                0.25 * lo + 0.5 * voice[b] + 0.25 * hi
            };
        }
        let t = self.t;
        let middle = (BARS / 2) as isize;
        for (i, bar) in self.bars.iter_mut().enumerate() {
            let k = (i as isize - middle).unsigned_abs();
            let target = match self.look {
                Mode::Writing => {
                    // A slow hump that runs outwards from the middle.
                    let s = 0.5 + 0.5 * (TAU * t / 1.6 - k as f32 * 0.6).sin();
                    0.12 + 0.4 * s * s
                }
                _ => {
                    let sway = 0.5 + 0.5 * (TAU * t / 3.0 - k as f32 * 0.5).sin();
                    0.07 + 0.04 * sway + 0.8 * mix[k]
                }
            };
            // Up with the voice quickly, down slowly, like a meter.
            let rate = if target > *bar { 12.0 } else { 3.5 };
            *bar += (target.min(1.0) - *bar) * (rate * dt).min(1.0);
        }
    }

    /// Draws the picture at `scale` into `pixels` (premultiplied ARGB) and
    /// returns its size.
    pub fn paint(&self, scale: f32, pixels: &mut Vec<u32>) -> (u32, u32) {
        let (width, height) = size(scale);
        let scale = width as f32 / WIDTH as f32;
        pixels.clear();
        pixels.resize((width * height) as usize, 0);
        let (w, h) = (WIDTH as f32, HEIGHT as f32);
        let colors = if self.dark { &DARK } else { &LIGHT };
        let span = BARS as f32 * BAR_WIDTH + (BARS - 1) as f32 * BAR_GAP;
        let left = (w - span) / 2.0 + BAR_WIDTH / 2.0;
        let bars: Vec<(f32, f32)> = self
            .bars
            .iter()
            .enumerate()
            .map(|(i, level)| {
                let x = left + i as f32 * (BAR_WIDTH + BAR_GAP);
                (x, (BAR_MIN + (BAR_MAX - BAR_MIN) * level.clamp(0.0, 1.0)) / 2.0)
            })
            .collect();
        for py in 0..height {
            for px in 0..width {
                // The pixel's centre in logical pixels, from the middle.
                let x = (px as f32 + 0.5) / scale;
                let y = (py as f32 + 0.5) / scale;
                let (cx, cy) = (x - w / 2.0, y - h / 2.0);
                let d = rounded_box(cx, cy, w / 2.0, h / 2.0, h / 2.0);
                let inside = coverage(d, scale);
                if inside <= 0.0 {
                    continue;
                }
                let mut color = [0.0f32; 4];
                over(&mut color, colors.background, inside);
                // A hairline just inside the edge.
                over(&mut color, colors.edge, inside - coverage(d + 1.0, scale));
                for &(bx, half) in &bars {
                    if (x - bx).abs() > BAR_WIDTH {
                        continue;
                    }
                    let r = BAR_WIDTH / 2.0;
                    let bar = coverage(rounded_box(x - bx, cy, r, half, r), scale);
                    over(&mut color, colors.bar, bar);
                }
                pixels[(py * width + px) as usize] = pack(color, self.alpha);
            }
        }
        (width, height)
    }
}

/// Signed distance from a box of half size `hw` × `hh` with corner radius
/// `r`, centred on the origin: negative inside.
fn rounded_box(x: f32, y: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let r = r.min(hw).min(hh);
    let qx = x.abs() - hw + r;
    let qy = y.abs() - hh + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// How much of a pixel a shape covers, from its distance in logical pixels.
fn coverage(distance: f32, scale: f32) -> f32 {
    (0.5 - distance * scale).clamp(0.0, 1.0)
}

/// Paints `src` (straight colour) with `amount` coverage over `dst`
/// (premultiplied).
fn over(dst: &mut [f32; 4], src: [f32; 4], amount: f32) {
    let a = src[3] * amount.clamp(0.0, 1.0);
    for c in 0..3 {
        dst[c] = src[c] * a + dst[c] * (1.0 - a);
    }
    dst[3] = a + dst[3] * (1.0 - a);
}

fn pack(color: [f32; 4], alpha: f32) -> u32 {
    let byte = |v: f32| (v * alpha * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
    byte(color[3]) << 24 | byte(color[0]) << 16 | byte(color[1]) << 8 | byte(color[2])
}

/// The wave on its own thread. Cheap to keep: the thread sleeps between
/// dictations and the surface exists only while something shows.
pub struct Overlay {
    tx: Sender<Mode>,
}

impl Overlay {
    /// `open` makes a surface (None where the system cannot show one);
    /// `tail` fills the slice with the newest microphone samples (newest
    /// last) and returns their rate, or 0 while nothing is recorded.
    pub fn spawn(open: fn() -> Option<Box<dyn Surface>>, tail: fn(&mut [f32]) -> u32) -> Overlay {
        let (tx, rx) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("kyrr-wave".into())
            .spawn(move || run(&rx, open, tail));
        if let Err(e) = started {
            eprintln!("kyrr: no wave on the screen: {e}");
        }
        Overlay { tx }
    }

    pub fn set(&self, mode: Mode) {
        let _ = self.tx.send(mode);
    }
}

fn run(rx: &Receiver<Mode>, open: fn() -> Option<Box<dyn Surface>>, tail: fn(&mut [f32]) -> u32) {
    let mut wave = Wave::default();
    let mut pixels = Vec::new();
    let mut samples = vec![0.0f32; TAIL];
    let mut analyzer: Option<Analyzer> = None;
    let mut told = false;
    while let Ok(mode) = rx.recv() {
        wave.set_mode(mode);
        if !wave.visible() {
            continue;
        }
        let Some(mut surface) = open() else {
            if !std::mem::replace(&mut told, true) {
                eprintln!("kyrr: this desktop cannot show the wave on the screen");
            }
            wave = Wave::default();
            continue;
        };
        wave.set_dark(surface.dark());
        let mut last = Instant::now();
        loop {
            let now = Instant::now();
            // Only listening looks at the microphone.
            let mut voice = [0.0; BANDS];
            if wave.mode == Mode::Listening {
                let rate = tail(&mut samples);
                if rate > 0 {
                    if analyzer.as_ref().map(Analyzer::rate) != Some(rate) {
                        analyzer = Some(Analyzer::new(rate));
                    }
                    if let Some(analyzer) = &analyzer {
                        voice = analyzer.bands(&samples);
                    }
                }
            }
            wave.step((now - last).as_secs_f32(), &voice);
            last = now;
            if !wave.visible() {
                break;
            }
            let (width, height) = wave.paint(surface.scale(), &mut pixels);
            if !surface.show(&pixels, width, height) {
                break;
            }
            match rx.recv_timeout(FRAME.saturating_sub(now.elapsed())) {
                Ok(mode) => wave.set_mode(mode),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            loop {
                match rx.try_recv() {
                    Ok(mode) => wave.set_mode(mode),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
        }
        // Closed (`surface` goes here); the next dictation starts afresh.
        let mode = wave.mode;
        wave = Wave::default();
        wave.set_mode(mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    const QUIET: [f32; BANDS] = [0.0; BANDS];

    fn shown(mode: Mode, seconds: f32, voice: &[f32; BANDS]) -> Wave {
        let mut wave = Wave::default();
        wave.set_mode(mode);
        for _ in 0..(seconds / 0.033) as usize {
            wave.step(0.033, voice);
        }
        wave
    }

    /// A sine of `hz` with `amplitude`, as long as the analyzer looks.
    fn tone(hz: f32, amplitude: f32, rate: u32) -> Vec<f32> {
        (0..TAIL)
            .map(|i| amplitude * (TAU * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn size_follows_the_scale() {
        assert_eq!(size(1.0), (WIDTH, HEIGHT));
        assert_eq!(size(2.0), (2 * WIDTH, 2 * HEIGHT));
        assert_eq!(size(1.25), (180, 70));
    }

    #[test]
    fn corners_are_clear_and_the_middle_is_not() {
        let wave = shown(Mode::Listening, 1.0, &QUIET);
        let mut pixels = Vec::new();
        let (w, h) = wave.paint(1.0, &mut pixels);
        assert_eq!(pixels.len(), (w * h) as usize);
        assert_eq!(alpha(pixels[0]), 0);
        assert_eq!(alpha(pixels[(w - 1) as usize]), 0);
        assert_eq!(alpha(pixels[(w * h - 1) as usize]), 0);
        assert!(alpha(pixels[(h / 2 * w + 4) as usize]) > 200);
    }

    #[test]
    fn pixels_are_premultiplied() {
        for dark in [true, false] {
            let mut wave = shown(Mode::Writing, 1.0, &QUIET);
            wave.set_dark(dark);
            let mut pixels = Vec::new();
            wave.paint(2.0, &mut pixels);
            for p in pixels {
                let a = p >> 24;
                assert!((p >> 16 & 0xff) <= a && (p >> 8 & 0xff) <= a && (p & 0xff) <= a);
            }
        }
    }

    #[test]
    fn dark_and_light_differ() {
        let mut dark = shown(Mode::Listening, 1.0, &QUIET);
        let mut light = dark.clone();
        dark.set_dark(true);
        light.set_dark(false);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        dark.paint(1.0, &mut a);
        light.paint(1.0, &mut b);
        let middle = (HEIGHT / 2 * WIDTH + 4) as usize;
        let brightness = |p: u32| (p >> 16 & 0xff) + (p >> 8 & 0xff) + (p & 0xff);
        assert!(brightness(b[middle]) > brightness(a[middle]) + 300);
    }

    #[test]
    fn the_voice_lifts_the_bars() {
        let quiet = shown(Mode::Listening, 1.0, &QUIET);
        let loud = shown(Mode::Listening, 1.0, &[0.8; BANDS]);
        assert!(loud.bars.iter().sum::<f32>() > quiet.bars.iter().sum::<f32>() + 1.0);
    }

    #[test]
    fn each_bar_follows_its_band() {
        // Only the lowest band: the middle bar rises, the outer ones do not.
        let mut voice = QUIET;
        voice[0] = 1.0;
        let wave = shown(Mode::Listening, 1.0, &voice);
        assert!(wave.bars[BARS / 2] > 0.6);
        assert!(wave.bars[0] < 0.2 && wave.bars[BARS - 1] < 0.2);
        // And it is the same on both sides.
        for k in 0..BARS / 2 {
            assert!((wave.bars[k] - wave.bars[BARS - 1 - k]).abs() < 1e-6);
        }
    }

    #[test]
    fn the_pitch_decides_the_bar() {
        for rate in [16_000, 44_100, 48_000] {
            let analyzer = Analyzer::new(rate);
            let low = analyzer.bands(&tone(200.0, 0.1, rate));
            let high = analyzer.bands(&tone(3500.0, 0.1, rate));
            let top = |b: &[f32; BANDS]| (0..BANDS).max_by(|&x, &y| b[x].total_cmp(&b[y])).unwrap();
            assert!(top(&low) <= 1, "{rate} Hz: low tone in band {}", top(&low));
            assert!(top(&high) >= BANDS - 2, "{rate} Hz: high tone in band {}", top(&high));
        }
    }

    #[test]
    fn silence_and_a_loud_voice_are_told_apart() {
        let analyzer = Analyzer::new(16_000);
        assert!(analyzer.bands(&vec![0.0; TAIL]).iter().all(|&l| l == 0.0));
        assert!(analyzer.bands(&tone(300.0, 0.3, 16_000)).iter().any(|&l| l > 0.8));
        // Too few samples to listen to: nothing.
        assert_eq!(analyzer.bands(&[0.5; 10]), QUIET);
    }

    #[test]
    fn hiding_fades_out_and_ends() {
        let mut wave = shown(Mode::Listening, 0.5, &QUIET);
        assert!(wave.visible() && wave.alpha == 1.0);
        wave.set_mode(Mode::Hidden);
        wave.step(0.1, &QUIET);
        assert!(wave.visible());
        for _ in 0..10 {
            wave.step(0.033, &QUIET);
        }
        assert!(!wave.visible());
    }

    #[test]
    fn a_hidden_wave_is_never_shown() {
        let wave = shown(Mode::Hidden, 1.0, &[0.8; BANDS]);
        assert!(!wave.visible());
    }
}
