//! Renders the dictation pill with words appearing as raw RGBA frames (for the
//! website's animation): `cargo run --example wave_frames -- <out.rgba> [de|en]
//! [dark|light] [scale] [text]`. The size goes to stderr. The website's loops
//! are this at scale 2.4, on a transparent 1160x160 canvas, encoded to WebM
//! with alpha (VP9), so the pill floats on the page without a frame. Each
//! saying is one run; the site's loops join the runs of one language and theme
//! into one file, which needs the same size for each. Encoding:
//! `ffmpeg -f lavfi -i color=c=0x00000000:s=1160x160:r=25,format=rgba -f rawvideo -pix_fmt rgba
//! -s WxH -r 25 -i out.rgba -filter_complex "[0][1]overlay=(W-w)/2:(H-h)/2:shortest=1,format=yuva420p" -an -c:v libvpx-vp9 -pix_fmt yuva420p -b:v 0 -crf 40 -auto-alt-ref 0 -row-mt 1 pill.webm`.

use std::f32::consts::TAU;
use std::io::Write;

use kyrr_common::wave::{Mode, Wave};

const FPS: f32 = 25.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).expect("output file");
    let de = args.get(2).is_none_or(|l| l == "de");
    let dark = args.get(3).is_none_or(|t| t == "dark");
    let scale: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(2.0);

    let default = if de {
        "Hallo Hanna, ich wollte dir nur kurz sagen, dass das Treffen morgen um zehn Uhr beginnt."
    } else {
        "Hi Anna, I just wanted to quickly say that the meeting starts tomorrow at ten o'clock."
    };
    let text = args.get(5).map_or(default, String::as_str);
    let words: Vec<String> = text.split(' ').map(str::to_string).collect();

    let mut wave = Wave::default();
    wave.set_dark(dark);
    let mut file = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    let mut pixels = Vec::new();
    let mut size = (0, 0);

    // Seconds: wave appears, words come in, the key is let go, writing, gone.
    let start = 0.5;
    let pace = 0.3;
    let release = start + 0.4 + pace * words.len() as f32 + 1.2;
    let write_end = release + 1.0;
    let end = write_end + 0.8;
    let mut t = 0.0;
    let mut shown = 0;
    wave.set_mode(Mode::Listening);
    while t < end {
        if t >= write_end {
            wave.set_mode(Mode::Hidden);
        } else if t >= release {
            wave.set_mode(Mode::Writing);
        }
        let due = (((t - start - 0.4) / pace).floor() + 1.0).clamp(0.0, words.len() as f32) as usize;
        if t < release && due > shown {
            shown = due;
            wave.set_words(&words[..shown]);
        }
        // A made-up voice: syllables, a bit different in each pitch band.
        let talking = if t >= start && t < release { 1.0 } else { 0.0 };
        let mut voice = [0.0f32; 7];
        for (b, v) in voice.iter_mut().enumerate() {
            let syl = 0.5 + 0.5 * (TAU * (t * 4.1 + b as f32 * 0.17)).sin();
            let wob = 0.5 + 0.5 * (TAU * (t * 1.3 + b as f32 * 0.41)).sin();
            *v = talking * (0.75 - b as f32 * 0.08) * (0.25 + 0.75 * syl * wob);
        }
        wave.step(1.0 / FPS, &voice);
        let (w, h) = wave.paint(scale, &mut pixels);
        size = (w, h);
        for p in &pixels {
            let (a, r, g, b) = ((p >> 24) & 255, (p >> 16) & 255, (p >> 8) & 255, p & 255);
            let un = |c: u32| (c * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
            file.write_all(&[un(r), un(g), un(b), a as u8]).unwrap();
        }
        t += 1.0 / FPS;
    }
    eprintln!("{}x{}", size.0, size.1);
}
