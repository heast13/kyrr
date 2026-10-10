//! Format handling of VoiceFilter with the real DPDFNet model, synthetic
//! signals only. Needs `scripts/fetch-assets.py`; skipped without assets.

use std::path::PathBuf;

use kyrr_denoise::{load, Mode, Model, VoiceFilter};

fn assets() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    dir.join(Model::Light.file_name()).is_file().then_some(dir)
}

fn filter(rate: u32, channels: usize, max_block: usize) -> Option<VoiceFilter> {
    let dir = assets()?;
    let denoiser = load(&dir, Model::Light, None).expect("model loads");
    Some(VoiceFilter::new(denoiser, rate, channels, max_block).expect("format accepted"))
}

/// Deterministic white noise in -amp..amp.
fn noise(len: usize, amp: f32, seed: &mut u32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            (*seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

const RATES: [u32; 8] = [8_000, 16_000, 22_050, 32_000, 44_100, 48_000, 88_200, 96_000];

#[test]
fn never_runs_dry_with_odd_block_sizes() {
    if assets().is_none() {
        eprintln!("assets missing, skipped");
        return;
    }
    let mut seed = 7;
    for rate in RATES {
        for channels in [1, 2] {
            let period = rate as usize / 100;
            for block in [period, 37, 512, 1024, 3 * period + 5] {
                let mut f = filter(rate, channels, period * 2).unwrap();
                let total = rate as usize * 2;
                let mut done = 0;
                while done < total {
                    let frames = block.min(total - done);
                    let input = noise(frames * channels, 0.1, &mut seed);
                    let mut output = vec![0.0; input.len()];
                    f.process(&input, &mut output);
                    assert!(output.iter().all(|x| x.is_finite()));
                    done += frames;
                }
                assert_eq!(f.underruns(), 0, "{rate} Hz, {channels} ch, block {block}");
            }
        }
    }
}

#[test]
fn reported_latency_matches_bypass_click() {
    if assets().is_none() {
        eprintln!("assets missing, skipped");
        return;
    }
    for rate in RATES {
        let mut f = filter(rate, 1, rate as usize / 100).unwrap();
        f.set_mode(Mode::Bypass);
        let len = rate as usize;
        let click = rate as usize / 4;
        let mut input = vec![0.0f32; len];
        input[click] = 1.0;
        let mut output = vec![0.0f32; len];
        for (i, o) in input
            .chunks(rate as usize / 100)
            .zip(output.chunks_mut(rate as usize / 100))
        {
            f.process(i, o);
        }
        let peak = output
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .unwrap();
        let measured = peak - click;
        let reported = f.latency_frames();
        let diff = measured.abs_diff(reported);
        assert!(diff <= 2, "{rate} Hz: measured {measured}, reported {reported}");
        println!(
            "{rate} Hz: latency {measured} frames = {:.1} ms",
            measured as f64 * 1000.0 / rate as f64
        );
    }
}

#[test]
fn removes_steady_noise() {
    if assets().is_none() {
        eprintln!("assets missing, skipped");
        return;
    }
    let mut seed = 3;
    for rate in [44_100, 48_000] {
        let mut f = filter(rate, 2, rate as usize / 100).unwrap();
        let input = noise(rate as usize * 3 * 2, 0.05, &mut seed);
        let mut output = vec![0.0; input.len()];
        f.process(&input, &mut output);
        let tail = output.len() / 3;
        let gain_db = 20.0 * (rms(&output[tail..]) / rms(&input[tail..])).log10();
        assert!(gain_db < -20.0, "{rate} Hz: noise only reduced by {gain_db:.1} dB");
        println!("{rate} Hz: white noise {gain_db:.1} dB");
    }
}

#[test]
fn non_finite_input_stays_finite() {
    if assets().is_none() {
        eprintln!("assets missing, skipped");
        return;
    }
    let mut f = filter(48_000, 1, 480).unwrap();
    let mut buf = vec![0.01f32; 4800];
    buf[100] = f32::NAN;
    buf[200] = f32::INFINITY;
    f.process_in_place(&mut buf);
    assert!(buf.iter().all(|x| x.is_finite()));
}
