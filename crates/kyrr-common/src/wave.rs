//! The dictation wave on the screen: a small dark pill near the bottom of
//! the screen with kyrr's teal bars in it. While kyrr listens the bars sway
//! slowly and rise with the voice; while it writes, a quicker wave runs
//! through them. It fades in and out and is gone between dictations.
//!
//! The drawing is done here, in plain Rust, so Windows and Linux show the
//! same picture; each system only puts the pixels on the screen (a
//! [`Surface`]). The surface is opened for each dictation and closed after
//! it, so nothing is kept on the screen or in a compositor between them.

use std::f32::consts::TAU;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

/// The pill's size in logical pixels.
pub const WIDTH: u32 = 180;
pub const HEIGHT: u32 = 56;

/// From the bottom of the screen (above the taskbar or panel) to the pill,
/// logical pixels.
pub const MARGIN: u32 = 40;

const BARS: usize = 17;
const BAR_WIDTH: f32 = 4.0;
const BAR_GAP: f32 = 5.0;
const BAR_MIN: f32 = 6.0;
const BAR_MAX: f32 = 40.0;

/// Straight (not premultiplied) colours, 0..1, with their opacity.
const BACKGROUND: [f32; 4] = [0.063, 0.086, 0.102, 0.88];
const EDGE: [f32; 4] = [0.369, 0.918, 0.831, 0.28];
/// The logo's teal on dark, as in the window's own little wave.
const BAR: [f32; 4] = [0.369, 0.918, 0.831, 1.0];

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
    /// The voice, 0..1, smoothed.
    voice: f32,
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
            voice: 0.0,
            alpha: 0.0,
            bars: [0.0; BARS],
        }
    }
}

/// A microphone level (RMS, linear) as 0..1: quiet room at the bottom,
/// talking close to the microphone near the top.
fn loudness(rms: f32) -> f32 {
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 55.0) / 40.0).clamp(0.0, 1.0)
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

    /// Moves on by `dt` seconds; `rms` is the microphone's level right now.
    pub fn step(&mut self, dt: f32, rms: f32) {
        let dt = dt.clamp(0.0, 0.1);
        self.t += dt;
        self.alpha = if self.mode == Mode::Hidden {
            (self.alpha - dt / FADE_OUT).max(0.0)
        } else {
            (self.alpha + dt / FADE_IN).min(1.0)
        };
        // Up with the voice at once, down slowly, like a meter.
        let target = if self.look == Mode::Listening {
            loudness(rms)
        } else {
            0.0
        };
        let rate = if target > self.voice { 18.0 } else { 4.0 };
        self.voice += (target - self.voice) * (rate * dt).min(1.0);
        let t = self.t;
        for (i, bar) in self.bars.iter_mut().enumerate() {
            let i = i as f32;
            // Higher in the middle, like a spoken word.
            let c = (i - (BARS - 1) as f32 / 2.0) / ((BARS - 1) as f32 / 2.0);
            let bell = 1.0 - 0.55 * c * c;
            let target = match self.look {
                Mode::Writing => {
                    // A hump that runs through the bars, left to right.
                    let s = 0.5 + 0.5 * (TAU * t / 0.8 - i * 0.9).sin();
                    0.1 + 0.75 * s * s * s
                }
                _ => {
                    let sway = 0.5 + 0.5 * (TAU * t / 1.8 - i * 0.7).sin();
                    let flutter = 0.5 + 0.5 * (TAU * t / 0.42 + i * 1.9).sin();
                    0.08 + bell * (0.16 * sway + self.voice * (0.35 + 0.45 * flutter))
                }
            };
            *bar += (target.min(1.0) - *bar) * (14.0 * dt).min(1.0);
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
                over(&mut color, BACKGROUND, inside);
                // A hairline just inside the edge.
                over(&mut color, EDGE, inside - coverage(d + 1.0, scale));
                for &(bx, half) in &bars {
                    if (x - bx).abs() > BAR_WIDTH {
                        continue;
                    }
                    let r = BAR_WIDTH / 2.0;
                    let bar = coverage(rounded_box(x - bx, cy, r, half, r), scale);
                    over(&mut color, BAR, bar);
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
    /// `level` is the microphone's level (RMS) right now.
    pub fn spawn(open: fn() -> Option<Box<dyn Surface>>, level: fn() -> f32) -> Overlay {
        let (tx, rx) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("kyrr-wave".into())
            .spawn(move || run(&rx, open, level));
        if let Err(e) = started {
            eprintln!("kyrr: no wave on the screen: {e}");
        }
        Overlay { tx }
    }

    pub fn set(&self, mode: Mode) {
        let _ = self.tx.send(mode);
    }
}

fn run(rx: &Receiver<Mode>, open: fn() -> Option<Box<dyn Surface>>, level: fn() -> f32) {
    let mut wave = Wave::default();
    let mut pixels = Vec::new();
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
        let mut last = Instant::now();
        loop {
            let now = Instant::now();
            wave.step((now - last).as_secs_f32(), level());
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

    fn shown(mode: Mode, seconds: f32, rms: f32) -> Wave {
        let mut wave = Wave::default();
        wave.set_mode(mode);
        for _ in 0..(seconds / 0.033) as usize {
            wave.step(0.033, rms);
        }
        wave
    }

    #[test]
    fn size_follows_the_scale() {
        assert_eq!(size(1.0), (WIDTH, HEIGHT));
        assert_eq!(size(2.0), (2 * WIDTH, 2 * HEIGHT));
        assert_eq!(size(1.25), (225, 70));
    }

    #[test]
    fn corners_are_clear_and_the_middle_is_not() {
        let wave = shown(Mode::Listening, 1.0, 0.0);
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
        let wave = shown(Mode::Writing, 1.0, 0.0);
        let mut pixels = Vec::new();
        wave.paint(2.0, &mut pixels);
        for p in pixels {
            let a = p >> 24;
            assert!((p >> 16 & 0xff) <= a && (p >> 8 & 0xff) <= a && (p & 0xff) <= a);
        }
    }

    #[test]
    fn the_voice_lifts_the_bars() {
        let quiet = shown(Mode::Listening, 1.0, 0.0);
        let loud = shown(Mode::Listening, 1.0, 0.1);
        assert!(loud.bars.iter().sum::<f32>() > quiet.bars.iter().sum::<f32>() + 1.0);
    }

    #[test]
    fn hiding_fades_out_and_ends() {
        let mut wave = shown(Mode::Listening, 0.5, 0.0);
        assert!(wave.visible() && wave.alpha == 1.0);
        wave.set_mode(Mode::Hidden);
        wave.step(0.1, 0.0);
        assert!(wave.visible());
        for _ in 0..10 {
            wave.step(0.033, 0.0);
        }
        assert!(!wave.visible());
    }

    #[test]
    fn a_hidden_wave_is_never_shown() {
        let wave = shown(Mode::Hidden, 1.0, 0.1);
        assert!(!wave.visible());
    }
}
