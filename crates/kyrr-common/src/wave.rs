//! The dictation wave on the screen: a small grey pill (dark or light, as
//! the system is) near the bottom of the screen with kyrr's green bars in
//! it. While kyrr listens, each bar follows one pitch band of the voice,
//! low sounds in the middle and high ones at the sides; while it writes, a
//! slow wave runs through them. It fades in and out and is gone between
//! dictations.
//!
//! While the key is held, the words the model hears so far run through the
//! pill as one line: the pill widens, the bars move to its left end, the
//! new words fade in one after the other at an even pace and the oldest run
//! out on the left side. The text is only shown here: what gets typed is
//! still the whole recording, recognised once the key is let go.
//!
//! The drawing is done here, in plain Rust, so Windows and Linux show the
//! same picture; each system only puts the pixels on the screen (a
//! [`Surface`]). The surface is opened for each dictation and closed after
//! it, so nothing is kept on the screen or in a compositor between them.

use std::f32::consts::TAU;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};

/// The picture's size in logical pixels: room for the pill at its widest,
/// with words in it. Around the pill it is clear.
pub const WIDTH: u32 = 460;
pub const HEIGHT: u32 = 56;

/// The pill's width without words.
const PILL: f32 = 144.0;

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

/// The bars next to words: thinner, closer and lower.
const BAR_WIDTH_SMALL: f32 = 3.0;
const BAR_GAP_SMALL: f32 = 2.5;
const BAR_MIN_SMALL: f32 = 5.0;
const BAR_MAX_SMALL: f32 = 28.0;

/// Inter Medium, cut down to the Latin letters (see `fonts/OFL.txt`).
static FONT: LazyLock<FontRef<'static>> = LazyLock::new(|| {
    FontRef::try_from_slice(include_bytes!("../fonts/Inter-Medium.ttf")).expect("the bundled font is valid")
});

/// Size of the words, logical pixels.
const TEXT_SIZE: f32 = 16.0;
/// From the pill's left end to the bars, from the bars to the words and from
/// the words to the pill's right end.
const BARS_PAD: f32 = 20.0;
const BARS_TEXT_GAP: f32 = 14.0;
const TEXT_PAD: f32 = 22.0;
/// Words fade out over this much of the line's left end once it scrolls.
const EDGE_FADE: f32 = 22.0;
/// A new word fades in over this long, rising this far (logical pixels)
/// into its line.
const APPEAR: f32 = 0.5;
const RISE: f32 = 2.0;
/// The words of one pass come one after the other, spread over most of the
/// time until the next pass is expected, so the line runs on evenly as if
/// it were typed, not in bursts. Never closer or further apart than this.
const PACE_SPREAD: f32 = 0.85;
const PACE_MIN: f32 = 0.06;
const PACE_MAX: f32 = 0.3;
/// The pill makes room for a word this long before it starts to show.
const LEAD: f32 = 0.35;
/// A replaced word fades out over this long.
const LEAVE: f32 = 0.25;

/// Straight (not premultiplied) colours, 0..1, with their opacity.
struct Colors {
    background: [f32; 4],
    edge: [f32; 4],
    bar: [f32; 4],
    text: [f32; 4],
}

/// The window's own greys and greens, for a dark and a light system.
const DARK: Colors = Colors {
    background: [0.165, 0.168, 0.176, 0.88],
    edge: [1.0, 1.0, 1.0, 0.10],
    bar: [0.369, 0.918, 0.831, 1.0],
    text: [0.93, 0.94, 0.95, 1.0],
};
const LIGHT: Colors = Colors {
    background: [0.965, 0.968, 0.975, 0.88],
    edge: [0.0, 0.0, 0.0, 0.12],
    bar: [0.078, 0.722, 0.651, 1.0],
    text: [0.11, 0.12, 0.13, 1.0],
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

/// A word of the line.
#[derive(Clone, Debug)]
struct Word {
    text: String,
    /// Logical pixels.
    width: f32,
    /// When it starts to fade in (seconds, as `Wave::t`).
    born: f32,
    /// Where it is drawn, logical pixels from the start of the line; eases
    /// to its place when a word before it changes length.
    x: f32,
}

/// A word the model changed its mind about, fading out where it was.
#[derive(Clone, Debug)]
struct Leaving {
    text: String,
    x: f32,
    /// How far it had faded in when it started to go.
    alpha: f32,
    since: f32,
}

/// The same word, apart from case and punctuation: the model often adds a
/// comma or a capital once it hears what follows.
fn alike(a: &str, b: &str) -> bool {
    let letters = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    letters(a) == letters(b)
}

/// Width of `text` in logical pixels.
fn measure(text: &str) -> f32 {
    let font = FONT.as_scaled(PxScale::from(TEXT_SIZE));
    let mut width = 0.0;
    let mut last = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(last) = last {
            width += font.kern(last, id);
        }
        width += font.h_advance(id);
        last = Some(id);
    }
    width
}

/// Moves `value` towards `target` at `rate` per second, smoothly.
fn ease(value: &mut f32, target: f32, rate: f32, dt: f32) {
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

/// 0..1 to 0..1, gently at both ends (smoothstep).
fn fade_in(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// 0..1 to 0..1, quick at first and gently at the end.
fn ease_out(p: f32) -> f32 {
    let q = 1.0 - p.clamp(0.0, 1.0);
    1.0 - q * q * q
}

/// Width of the bars at `open` (0 without words, 1 next to them).
fn bars_span(open: f32) -> f32 {
    let bar = lerp(BAR_WIDTH, BAR_WIDTH_SMALL, open);
    let gap = lerp(BAR_GAP, BAR_GAP_SMALL, open);
    BARS as f32 * bar + (BARS - 1) as f32 * gap
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Room for words in a pill of `pill` width with the bars small.
fn line_room(pill: f32) -> f32 {
    pill - BARS_PAD - bars_span(1.0) - BARS_TEXT_GAP - TEXT_PAD
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
    /// The line of words, oldest first.
    words: Vec<Word>,
    leaving: Vec<Leaving>,
    /// 0 without words, 1 with: the bars small at the left end.
    open: f32,
    /// The pill's width, logical pixels, eased.
    pill: f32,
    /// How far the line has run out on the left, logical pixels, eased.
    scroll: f32,
    /// When words were last set (as `t`), to pace the next ones.
    last_words: Option<f32>,
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
            words: Vec::new(),
            leaving: Vec::new(),
            open: 0.0,
            pill: PILL,
            scroll: 0.0,
            last_words: None,
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
        if mode == Mode::Listening && self.mode != Mode::Listening {
            // A new dictation: its own words.
            self.words.clear();
            self.leaving.clear();
            self.last_words = None;
        }
        if mode != Mode::Hidden {
            self.look = mode;
        }
        self.mode = mode;
    }

    /// The words heard so far, oldest first. Words that stay keep their
    /// place (a changed comma or capital just shows); from the first one
    /// that differs on, the old words fade out and the new ones fade in.
    pub fn set_words(&mut self, words: &[String]) {
        let mut keep = 0;
        while keep < self.words.len().min(words.len()) && alike(&self.words[keep].text, &words[keep]) {
            let word = &mut self.words[keep];
            if word.text != words[keep] {
                word.text = words[keep].clone();
                word.width = measure(&word.text);
            }
            keep += 1;
        }
        let t = self.t;
        for word in self.words.drain(keep..) {
            // One that had not started to show just goes.
            if word.born < t {
                let alpha = fade_in((t - word.born) / APPEAR);
                self.leaving.push(Leaving {
                    text: word.text,
                    x: word.x,
                    alpha,
                    since: t,
                });
            }
        }
        let new = words.len().saturating_sub(keep);
        let since = self.last_words.map_or(1.0, |last| t - last);
        let pace = (since * PACE_SPREAD / new.max(1) as f32).clamp(PACE_MIN, PACE_MAX);
        self.last_words = Some(t);
        // After the words still waiting to show.
        let mut born = self.words.last().map_or(t, |w| (w.born + pace).max(t));
        let space = measure(" ");
        for text in &words[keep..] {
            let x = self.words.last().map_or(0.0, |w| w.x + w.width + space);
            self.words.push(Word {
                width: measure(text),
                text: text.clone(),
                born,
                x,
            });
            born += pace;
        }
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
        self.step_words(dt);
    }

    fn step_words(&mut self, dt: f32) {
        let t = self.t;
        let space = measure(" ");
        // The line as far as it shows soon: the pill grows and the line
        // runs on word by word, a little ahead of each, so it has room by
        // the time it has faded in.
        let mut place = 0.0;
        let mut end: f32 = 0.0;
        for word in &mut self.words {
            ease(&mut word.x, place, 10.0, dt);
            if word.born <= t + LEAD {
                end = place + word.width;
            }
            place += word.width + space;
        }
        self.leaving.retain(|w| t - w.since < LEAVE);
        let (open, pill, scroll) = if self.words.is_empty() {
            (0.0, PILL, 0.0)
        } else {
            let pill = (WIDTH as f32 - 2.0).min(PILL.max(end + (WIDTH as f32 - line_room(WIDTH as f32))));
            (1.0, pill, (end - line_room(pill)).max(0.0))
        };
        ease(&mut self.open, open, 7.0, dt);
        ease(&mut self.pill, pill, 6.0, dt);
        ease(&mut self.scroll, scroll, 6.0, dt);
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
        let open = self.open.clamp(0.0, 1.0);
        let pill = self.pill.clamp(PILL, w);
        let pill_left = (w - pill) / 2.0;
        let bar_width = lerp(BAR_WIDTH, BAR_WIDTH_SMALL, open);
        let bar_gap = lerp(BAR_GAP, BAR_GAP_SMALL, open);
        let bar_min = lerp(BAR_MIN, BAR_MIN_SMALL, open);
        let bar_max = lerp(BAR_MAX, BAR_MAX_SMALL, open);
        let span = bars_span(open);
        // In the middle without words, at the left end with them.
        let bars_left = lerp((w - span) / 2.0, pill_left + BARS_PAD, open);
        let bars: Vec<(f32, f32)> = self
            .bars
            .iter()
            .enumerate()
            .map(|(i, level)| {
                let x = bars_left + bar_width / 2.0 + i as f32 * (bar_width + bar_gap);
                (x, (bar_min + (bar_max - bar_min) * level.clamp(0.0, 1.0)) / 2.0)
            })
            .collect();
        let text = self.paint_words(
            scale,
            width,
            height,
            pill_left + BARS_PAD + bars_span(1.0) + BARS_TEXT_GAP,
            pill_left + pill - TEXT_PAD,
        );
        for py in 0..height {
            for px in 0..width {
                // The pixel's centre in logical pixels, from the middle.
                let x = (px as f32 + 0.5) / scale;
                let y = (py as f32 + 0.5) / scale;
                let (cx, cy) = (x - w / 2.0, y - h / 2.0);
                let d = rounded_box(cx, cy, pill / 2.0, h / 2.0, h / 2.0);
                let inside = coverage(d, scale);
                if inside <= 0.0 {
                    continue;
                }
                let mut color = [0.0f32; 4];
                over(&mut color, colors.background, inside);
                // A hairline just inside the edge.
                over(&mut color, colors.edge, inside - coverage(d + 1.0, scale));
                for &(bx, half) in &bars {
                    if (x - bx).abs() > bar_width {
                        continue;
                    }
                    let r = bar_width / 2.0;
                    let bar = coverage(rounded_box(x - bx, cy, r, half, r), scale);
                    over(&mut color, colors.bar, bar);
                }
                if let Some(text) = &text {
                    let ink = text[(py * width + px) as usize];
                    if ink > 0.0 {
                        over(&mut color, colors.text, ink.min(1.0) * open * inside);
                    }
                }
                pixels[(py * width + px) as usize] = pack(color, self.alpha);
            }
        }
        (width, height)
    }

    /// The words between `left` and `right` (logical pixels) as ink, 0..1 for
    /// each physical pixel; None without words.
    fn paint_words(&self, scale: f32, width: u32, height: u32, left: f32, right: f32) -> Option<Vec<f32>> {
        if self.words.is_empty() && self.leaving.is_empty() {
            return None;
        }
        let mut ink = vec![0.0f32; (width * height) as usize];
        let size = PxScale::from(TEXT_SIZE * scale);
        let font = FONT.as_scaled(PxScale::from(TEXT_SIZE));
        // Capitals sit in the middle of the pill.
        let baseline = HEIGHT as f32 / 2.0 + (font.ascent() + font.descent()) / 2.0;
        // The line fades out on the left once it runs out there.
        let fade_left = (self.scroll / EDGE_FADE).clamp(0.0, 1.0);
        let t = self.t;
        let mut draw = |text: &str, x: f32, alpha: f32, appear: Option<f32>| {
            let start = left + x - self.scroll;
            if alpha <= 0.0 || start > right + 1.0 || start + measure(text) < left - EDGE_FADE {
                return;
            }
            let (alpha, rise) = match appear {
                Some(p) => (alpha * fade_in(p), (1.0 - ease_out(p)) * RISE),
                None => (alpha, 0.0),
            };
            let mut pen = start;
            let mut last = None;
            for c in text.chars() {
                let id = font.glyph_id(c);
                if let Some(last) = last {
                    pen += font.kern(last, id);
                }
                last = Some(id);
                let glyph = id.with_scale_and_position(size, point(pen * scale, (baseline + rise) * scale));
                pen += font.h_advance(id);
                let Some(outline) = FONT.outline_glyph(glyph) else {
                    continue;
                };
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, c| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                        return;
                    }
                    let lx = (px as f32 + 0.5) / scale;
                    // Nothing left of the line, whether it has run out yet or not.
                    let edge_left = (lx - left + 1.0).clamp(0.0, 1.0)
                        * (1.0 - fade_left * (1.0 - ((lx - left) / EDGE_FADE).clamp(0.0, 1.0)));
                    let edge_right = ((right + 4.0 - lx) / 4.0).clamp(0.0, 1.0);
                    let cell = &mut ink[(py as u32 * width + px as u32) as usize];
                    *cell += c * alpha * edge_left * edge_right;
                });
            }
        };
        for word in &self.leaving {
            draw(&word.text, word.x, word.alpha * (1.0 - (t - word.since) / LEAVE), None);
        }
        for word in &self.words {
            if word.born > t {
                continue;
            }
            let p = (t - word.born) / APPEAR;
            draw(&word.text, word.x, 1.0, (p < 1.0).then_some(p));
        }
        Some(ink)
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

/// What the wave's thread is told.
enum Message {
    Mode(Mode),
    Words(Vec<String>),
}

impl Wave {
    fn apply(&mut self, message: Message) {
        match message {
            Message::Mode(mode) => self.set_mode(mode),
            Message::Words(words) => self.set_words(&words),
        }
    }
}

/// The wave on its own thread. Cheap to keep: the thread sleeps between
/// dictations and the surface exists only while something shows.
pub struct Overlay {
    tx: Sender<Message>,
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
        let _ = self.tx.send(Message::Mode(mode));
    }

    /// The words heard so far in this dictation, oldest first.
    pub fn set_words(&self, words: Vec<String>) {
        let _ = self.tx.send(Message::Words(words));
    }
}

fn run(rx: &Receiver<Message>, open: fn() -> Option<Box<dyn Surface>>, tail: fn(&mut [f32]) -> u32) {
    let mut wave = Wave::default();
    let mut pixels = Vec::new();
    let mut samples = vec![0.0f32; TAIL];
    let mut analyzer: Option<Analyzer> = None;
    let mut told = false;
    while let Ok(message) = rx.recv() {
        wave.apply(message);
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
                Ok(message) => wave.apply(message),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            loop {
                match rx.try_recv() {
                    Ok(message) => wave.apply(message),
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
        assert_eq!(size(1.25), (575, 70));
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
        let left_end = (WIDTH - PILL as u32) / 2 + 4;
        assert!(alpha(pixels[(h / 2 * w + left_end) as usize]) > 200);
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
        let middle = (HEIGHT / 2 * WIDTH + (WIDTH - PILL as u32) / 2 + 4) as usize;
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

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn run_for(wave: &mut Wave, seconds: f32) {
        for _ in 0..(seconds / 0.033) as usize {
            wave.step(0.033, &QUIET);
        }
    }

    #[test]
    fn words_widen_the_pill_and_show_in_it() {
        let mut wave = shown(Mode::Listening, 0.5, &QUIET);
        let mut before = Vec::new();
        wave.paint(1.0, &mut before);
        wave.set_words(&words("Hallo, wie geht es dir?"));
        run_for(&mut wave, 1.5);
        assert!(wave.open > 0.99);
        assert!(wave.pill > PILL + 60.0 && wave.pill < WIDTH as f32);
        assert_eq!(wave.scroll, 0.0);
        let mut after = Vec::new();
        let (w, h) = wave.paint(1.0, &mut after);
        // Ink right of the bars, where the pill was clear before.
        let text_left = ((WIDTH as f32 - wave.pill) / 2.0 + BARS_PAD + bars_span(1.0) + BARS_TEXT_GAP) as u32;
        let inked = (0..h)
            .flat_map(|y| (text_left..text_left + 60).map(move |x| (y * w + x) as usize))
            .filter(|&i| after[i] != before[i])
            .count();
        assert!(inked > 100, "{inked} pixels of text");
    }

    #[test]
    fn words_that_stay_keep_their_place() {
        let mut wave = shown(Mode::Listening, 0.5, &QUIET);
        wave.set_words(&words("hallo wie geht"));
        run_for(&mut wave, 1.0);
        let born: Vec<f32> = wave.words.iter().map(|w| w.born).collect();
        // A comma and a capital later, and one word heard anew.
        wave.set_words(&words("Hallo, wie gut es"));
        assert_eq!(wave.words[0].text, "Hallo,");
        assert_eq!(wave.words[0].born, born[0]);
        assert_eq!(wave.words[1].born, born[1]);
        assert!(wave.words[2].born > born[2]);
        assert_eq!(wave.leaving.len(), 1);
        assert_eq!(wave.leaving[0].text, "geht");
        // The new ones follow each other, spread over the time since the
        // words before came in (1 s), but at most 0.3 s apart.
        let gap = wave.words[3].born - wave.words[2].born;
        assert!((gap - 0.3).abs() < 0.01, "{gap}");
        run_for(&mut wave, 0.5);
        assert!(wave.leaving.is_empty());
    }

    #[test]
    fn a_long_line_runs_out_on_the_left() {
        let mut wave = shown(Mode::Listening, 0.5, &QUIET);
        let line =
            "das ist ein ziemlich langer Satz der nicht mehr in die Pille passt und deshalb nach links hinaus läuft";
        wave.set_words(&words(line));
        run_for(&mut wave, 4.0);
        assert!(wave.pill <= WIDTH as f32 - 1.0 && wave.pill > WIDTH as f32 - 10.0);
        assert!(wave.scroll > 100.0);
        let mut pixels = Vec::new();
        wave.paint(2.0, &mut pixels);
        for p in pixels {
            let a = p >> 24;
            assert!((p >> 16 & 0xff) <= a && (p >> 8 & 0xff) <= a && (p & 0xff) <= a);
        }
    }

    #[test]
    fn a_new_dictation_starts_without_words() {
        let mut wave = shown(Mode::Listening, 0.5, &QUIET);
        wave.set_words(&words("eins zwei"));
        wave.set_mode(Mode::Writing);
        wave.set_mode(Mode::Hidden);
        assert_eq!(wave.words.len(), 2);
        wave.set_mode(Mode::Listening);
        assert!(wave.words.is_empty());
        run_for(&mut wave, 1.0);
        assert!(wave.open < 0.01 && (wave.pill - PILL).abs() < 0.5);
    }

    #[test]
    fn every_letter_has_a_glyph() {
        for c in "abcdefghijklmnopqrstuvwxyzäöüßABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÜ0123456789.,;:!?-'\"„“”‘’…–—()€%&éèêàâçñ"
            .chars()
        {
            assert!(FONT.glyph_id(c).0 != 0, "{c} is missing");
        }
    }
}
