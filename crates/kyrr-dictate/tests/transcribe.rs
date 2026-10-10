//! Transcribes the sherpa-onnx sample clips with the installed models.
//! Needs `scripts/fetch-assets.py --test` and the models in `models/`;
//! skipped otherwise.

use std::path::PathBuf;

use kyrr_dictate::{Dictation, Language};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_mono(path: &PathBuf) -> (u32, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("wav opens");
    let spec = reader.spec();
    let scale = 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32;
    let samples: Vec<f32> = reader
        .samples::<i32>()
        .map(|s| s.expect("sample") as f32 * scale)
        .collect();
    let mono = samples
        .chunks(spec.channels as usize)
        .map(|f| f.iter().sum::<f32>() / f.len() as f32)
        .collect();
    (spec.sample_rate, mono)
}

fn check(language: Language, clip: &str, expected: &str) {
    let models = root().join("models");
    let wav = root().join("assets/test-audio").join(clip);
    if !language.model().is_installed(&models) || !wav.is_file() {
        eprintln!("{} model or test audio missing, skipped", language.code());
        return;
    }
    let dictation = Dictation::load(&models, language, 4).expect("model loads");
    let (rate, samples) = read_mono(&wav);
    let text = dictation.transcribe(rate, &samples);
    println!("{}: {text}", language.code());
    assert_eq!(text, expected);
    assert_eq!(
        dictation.transcribe(rate, &samples[..rate as usize / 10]),
        "",
        "too short"
    );
}

#[test]
fn german() {
    check(
        Language::German,
        "de.wav",
        "Alles hat ein Ende, nur die Wurst hat zwei.",
    );
}

#[test]
fn english() {
    check(
        Language::English,
        "en.wav",
        "Ask not what your country can do for you, ask what you can do for your country.",
    );
}
