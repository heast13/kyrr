//! The pill's pixels, frame by frame, against what the plain pixel-by-pixel
//! drawing made of them (hashes taken from it): the quick paths of
//! `Wave::paint` must not change a single pixel.

use std::f32::consts::TAU;

use kyrr_common::wave::{Mode, Wave};

const WORDS: &str = "Hallo Hanna, ich wollte dir nur kurz sagen, dass das Treffen morgen um zehn Uhr beginnt.";

/// Hash of the frames of a dictation with words (every fifth frame at 30 per
/// second, 14 s), or without.
fn frames(scale: f32, dark: bool, with_words: bool) -> u64 {
    let words: Vec<String> = WORDS.split(' ').map(str::to_string).collect();
    let mut wave = Wave::default();
    wave.set_dark(dark);
    wave.set_mode(Mode::Listening);
    let mut pixels = Vec::new();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut shown = 0;
    for frame in 0..420 {
        let t = frame as f32 / 30.0;
        if t >= 11.0 {
            wave.set_mode(Mode::Writing);
        }
        let due = ((t - 0.5) / 0.4).floor().clamp(0.0, words.len() as f32) as usize;
        if with_words && t < 10.0 && due > shown {
            shown = due;
            wave.set_words(&words[..shown]);
        }
        let mut voice = [0.0f32; 7];
        for (b, v) in voice.iter_mut().enumerate() {
            *v = 0.7 * (0.5 + 0.5 * (TAU * (t * 4.1 + b as f32 * 0.17)).sin());
        }
        wave.step(1.0 / 30.0, &voice);
        if frame % 5 == 0 {
            wave.paint(scale, &mut pixels);
            for &p in &pixels {
                hash = (hash ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hash
}

#[test]
fn the_quick_paths_paint_the_same_pixels() {
    assert_eq!(frames(1.0, true, true), 0x5bec_60a3_b703_be34);
    assert_eq!(frames(1.5, false, true), 0x2f0a_8596_bbea_3172);
    assert_eq!(frames(2.0, true, false), 0xcd49_4ac9_ae7f_0d0d);
    assert_eq!(frames(1.25, false, false), 0x42d5_7580_8033_24fd);
}
