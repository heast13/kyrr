//! Transcribes the sherpa-onnx sample clips with the installed models.
//! Needs `scripts/fetch-assets.py --test` and the models in `models/`;
//! skipped otherwise.

use std::path::PathBuf;

use kyrr_dictate::{Dictation, Speech};

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

fn check(speech: Speech, clip: &str, expected: &str) {
    let models = root().join("models");
    let wav = root().join("assets/test-audio").join(clip);
    if !speech.model().is_installed(&models) || !wav.is_file() {
        eprintln!("{} model or test audio missing, skipped", speech.code());
        return;
    }
    let dictation = Dictation::load(&models, speech, 4).expect("model loads");
    let (rate, samples) = read_mono(&wav);
    let text = dictation.transcribe(rate, &samples);
    println!("{}: {text}", speech.code());
    assert_eq!(text, expected);
    assert_eq!(
        dictation.transcribe(rate, &samples[..rate as usize / 10]),
        "",
        "too short"
    );
}

#[test]
fn german() {
    check(Speech::German, "de.wav", "Alles hat ein Ende, nur die Wurst hat zwei.");
}

#[test]
fn english() {
    check(
        Speech::English,
        "en.wav",
        "Ask not what your country can do for you, ask what you can do for your country.",
    );
}

/// The same sentence as in the English clip, spoken in Chinese, Japanese and
/// Korean. The int8 model's output differs a little between CPUs with and
/// without VNNI, so the clips are checked for their key phrases.
fn check_contains(speech: Speech, clip: &str, phrases: &[&str], not: &[&str]) {
    let models = root().join("models");
    let wav = root().join("assets/test-audio").join(clip);
    if !speech.model().is_installed(&models) || !wav.is_file() {
        eprintln!("{} model or test audio missing, skipped", speech.code());
        return;
    }
    let dictation = Dictation::load(&models, speech, 4).expect("model loads");
    let (rate, samples) = read_mono(&wav);
    let text = dictation.transcribe(rate, &samples);
    println!("{}: {text}", speech.code());
    for phrase in phrases {
        assert!(text.contains(phrase), "{phrase} missing in {text}");
    }
    for phrase in not {
        assert!(!text.contains(phrase), "{phrase} in {text}");
    }
    // Noise and silence are nothing, not a made-up character.
    assert_eq!(dictation.transcribe(16_000, &vec![0.0; 16_000 * 3]), "");
}

#[test]
fn chinese() {
    check_contains(
        Speech::Chinese,
        "zh.wav",
        &["不要问你的国家能为你做什么", "要问你能为你的国家做什么"],
        &[" "],
    );
}

#[test]
fn japanese() {
    // No gaps between words, as the model puts them.
    check_contains(
        Speech::Japanese,
        "ja.wav",
        &["のために何ができるか", "あなたが国のために"],
        &[" "],
    );
}

#[test]
fn korean() {
    check_contains(Speech::Korean, "ko.wav", &["조국", "물으십시오"], &[]);
}

/// Windows user names with umlauts or other scripts end up in the model
/// path (%LOCALAPPDATA%). The model folder is hard-linked there, no copy.
#[test]
fn model_path_with_umlauts() {
    let models = root().join("models");
    let wav = root().join("assets/test-audio/de.wav");
    let spec = Speech::German.model();
    if !spec.is_installed(&models) || !wav.is_file() {
        eprintln!("de model or test audio missing, skipped");
        return;
    }
    let odd = std::env::temp_dir().join(format!("kyrr-Jürgen-Ærøskøbing-日本-{}", std::process::id()));
    let dir = spec.dir(&odd);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(spec.dir(&models)).unwrap() {
        let entry = entry.unwrap();
        let to = dir.join(entry.file_name());
        if std::fs::hard_link(entry.path(), &to).is_err() {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
    let loaded = Dictation::load(&odd, Speech::German, 4);
    let _ = std::fs::remove_dir_all(&odd);
    let dictation = loaded.expect("model loads from a path with umlauts");
    let (rate, samples) = read_mono(&wav);
    assert_eq!(
        dictation.transcribe(rate, &samples),
        "Alles hat ein Ende, nur die Wurst hat zwei."
    );
}
