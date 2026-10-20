//! Robustness sweeps: every rate, channel count and setting, with hostile
//! input, must stay finite and within full scale.

use kyrr_common::{Amount, Look, VoiceSettings};
use kyrr_voice::Voice;

fn all_on(look: Look) -> VoiceSettings {
    VoiceSettings {
        enabled: true,
        look,
        body: 60,
        presence: -60,
        air: 60,
        low_cut: true,
        deesser: Amount::Strong,
        compressor: Amount::Strong,
        pops: true,
        auto_level: true,
        distance: true,
    }
}
fn noise(n: usize, amp: f32, seed: &mut u32) -> Vec<f32> {
    (0..n)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            (*seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

#[test]
fn many_channels_and_rates() {
    let mut seed = 1;
    for rate in [1u32, 100, 8000, 11025, 44100, 48000, 192000, 384000] {
        for ch in [1usize, 2, 6, 8] {
            for look in [Look::Neutral, Look::Custom, Look::Bright] {
                let mut v = Voice::new(rate, ch);
                v.set(&all_on(look));
                let mut buf = noise(ch * 5000, 0.9, &mut seed);
                buf[7] = f32::NAN;
                buf[8] = f32::INFINITY;
                buf[9] = f32::NEG_INFINITY;
                for block in buf.chunks_mut(ch * 480) {
                    v.pre(block);
                    v.post(block);
                }
                assert!(
                    buf.iter().all(|x| x.is_finite() && x.abs() <= 1.0),
                    "rate {rate} ch {ch}"
                );
            }
        }
    }
}

#[test]
fn settings_flip_mid_stream_stays_finite_and_bounded() {
    let mut seed = 5;
    let mut v = Voice::new(48000, 2);
    let looks = [Look::Neutral, Look::Warm, Look::Clear, Look::Bright, Look::Custom];
    let mut max_step = 0f32;
    let mut prev = 0f32;
    for i in 0..400 {
        let mut s = all_on(looks[i % 5]);
        s.enabled = i % 7 != 0;
        s.deesser = Amount::ALL[i % 4];
        s.compressor = Amount::ALL[(i / 2) % 4];
        s.low_cut = i % 3 == 0;
        v.set(&s);
        let mut buf = noise(960, 0.5, &mut seed);
        v.pre(&mut buf);
        v.post(&mut buf);
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
        for x in &buf {
            max_step = max_step.max((x - prev).abs());
            prev = *x;
        }
    }
    println!("max step {max_step}");
}

#[test]
fn zero_input_stays_zero_after_loud() {
    let mut v = Voice::new(48000, 1);
    v.set(&VoiceSettings {
        enabled: true,
        compressor: Amount::Strong,
        deesser: Amount::Strong,
        ..VoiceSettings::default()
    });
    let mut seed = 9;
    let mut loud = noise(48000, 0.8, &mut seed);
    v.pre(&mut loud);
    v.post(&mut loud);
    let mut silence = vec![0f32; 48000 * 20];
    let t = std::time::Instant::now();
    for b in silence.chunks_mut(480) {
        v.pre(b);
        v.post(b);
    }
    println!("20 s of silence took {:?}", t.elapsed());
    assert!(silence.iter().rev().take(1000).all(|x| x.abs() < 1e-6));
}
