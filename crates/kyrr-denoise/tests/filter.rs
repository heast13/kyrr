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

const RATES: [u32; 12] = [
    8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000,
];

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

/// Both engines, both models, at the formats common devices use: noise
/// goes, nothing runs dry, and the CPU it takes is printed. CPUs without
/// AVX2/FMA (older Core i, Celeron and Pentium Silver, Atom) run ONNX
/// Runtime; this machine would pick the native engine, so for ONNX the test
/// hides the packed weights by putting only the .onnx files in a folder of
/// their own.
#[test]
fn every_engine_and_model() {
    let Some(dir) = assets() else {
        eprintln!("assets missing, skipped");
        return;
    };
    let ort = dir.join("onnxruntime.dll");
    let only_onnx = std::env::temp_dir().join(format!("kyrr-onnx-test-{}", std::process::id()));
    std::fs::create_dir_all(&only_onnx).unwrap();
    let mut seed = 11;
    for model in [Model::Light, Model::Quality] {
        std::fs::copy(dir.join(model.file_name()), only_onnx.join(model.file_name())).unwrap();
        let mut engines = vec![];
        if kyrr_denoise::native_possible(&dir.join(model.file_name())) {
            engines.push(("native", &dir, None));
        }
        if cfg!(windows) && ort.is_file() {
            assert!(!kyrr_denoise::native_possible(&only_onnx.join(model.file_name())));
            engines.push(("onnx", &only_onnx, Some(ort.as_path())));
        }
        for (engine, from, runtime) in engines {
            // A built-in mic array at 16 kHz with four channels, a USB mic
            // at 44.1 kHz, the usual 48 kHz stereo, an interface at 96 kHz.
            for (rate, channels) in [(16_000, 4), (44_100, 1), (48_000, 2), (96_000, 1)] {
                let denoiser = load(from, model, runtime).expect("model loads");
                let mut f = VoiceFilter::new(denoiser, rate, channels, rate as usize / 50).expect("format accepted");
                let seconds = 3;
                let input = noise(rate as usize * seconds * channels, 0.05, &mut seed);
                let mut output = vec![0.0; input.len()];
                let block = rate as usize / 100 * channels;
                let started = std::time::Instant::now();
                for (i, o) in input.chunks(block).zip(output.chunks_mut(block)) {
                    f.process(i, o);
                }
                let load = started.elapsed().as_secs_f64() / seconds as f64;
                let what = format!("{engine} {model:?}, {rate} Hz, {channels} ch");
                assert!(output.iter().all(|x| x.is_finite()), "{what}");
                assert_eq!(f.underruns(), 0, "{what}");
                let tail = output.len() / 3;
                let gain_db = 20.0 * (rms(&output[tail..]) / rms(&input[tail..])).log10();
                assert!(gain_db < -20.0, "{what}: noise only reduced by {gain_db:.1} dB");
                println!("{what}: noise {gain_db:.1} dB, {:.1} % of real time", load * 100.0);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&only_onnx);
}

/// The native engine runs from the packed weights alone (the AppImage and
/// the tarball leave out the .onnx files), with the same output.
#[test]
fn native_needs_only_the_weights() {
    let Some(dir) = assets() else {
        eprintln!("assets missing, skipped");
        return;
    };
    if !kyrr_denoise::native_possible(&dir.join(Model::Light.file_name())) {
        eprintln!("no native engine on this CPU, skipped");
        return;
    }
    let only_weights = std::env::temp_dir().join(format!("kyrr-weights-test-{}", std::process::id()));
    std::fs::create_dir_all(&only_weights).unwrap();
    for model in [Model::Light, Model::Quality] {
        std::fs::copy(dir.join(model.weights_name()), only_weights.join(model.weights_name())).unwrap();
        assert!(!only_weights.join(model.file_name()).exists());
        let mut seed = 5;
        let input = noise(48_000, 0.05, &mut seed);
        let run = |from: &std::path::Path| {
            let denoiser = load(from, model, None).expect("model loads");
            let mut f = VoiceFilter::new(denoiser, 48_000, 1, 480).unwrap();
            let mut output = vec![0.0; input.len()];
            for (i, o) in input.chunks(480).zip(output.chunks_mut(480)) {
                f.process(i, o);
            }
            output
        };
        assert_eq!(run(&only_weights), run(&dir), "{model:?}");
    }
    // Damaged weights without an .onnx file: the native engine's reason,
    // not an ONNX Runtime error about a file that is not there.
    let weights = only_weights.join(Model::Light.weights_name());
    let mut bytes = std::fs::read(&weights).unwrap();
    bytes[1000] ^= 1;
    std::fs::write(&weights, bytes).unwrap();
    let e = load(&only_weights, Model::Light, None)
        .err()
        .expect("damaged weights refused");
    assert!(e.contains("expected checksum"), "{e}");
    std::fs::remove_dir_all(&only_weights).unwrap();
}
