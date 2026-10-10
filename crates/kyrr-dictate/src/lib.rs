//! kyrr-dictate: speech to text for German and English, fully local.
//!
//! [`Dictation`] loads the model for one [`Language`] and turns a recording
//! (any sample rate, mono) into text. Models are downloaded on demand with
//! [`download::download`] and pinned by SHA256.

pub mod download;
mod model;

use std::path::Path;

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};

pub use model::{Language, ModelFile, ModelSpec};

/// Shortest recording worth decoding; shorter ones are accidental key taps.
pub const MIN_SECONDS: f32 = 0.3;

/// Rate the models expect. Recordings are resampled here rather than inside
/// sherpa-onnx, which logs every resampler it creates.
const MODEL_RATE: u32 = 16_000;

/// A loaded dictation model. Loading takes a few seconds and about 1 GB of
/// memory, so the app keeps it only while dictation is in use.
pub struct Dictation {
    language: Language,
    recognizer: OfflineRecognizer,
}

impl Dictation {
    /// Loads the model for `language` from below `root`. `threads` is the
    /// number of CPU threads used for decoding.
    pub fn load(root: &Path, language: Language, threads: usize) -> Result<Dictation, String> {
        let spec = language.model();
        if !spec.is_installed(root) {
            return Err(format!("model {} is not installed", spec.id));
        }
        let dir = spec.dir(root);
        let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
        let mut config = OfflineRecognizerConfig::default();
        config.feat_config.feature_dim = 128;
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: path("encoder.int8.onnx"),
            decoder: path("decoder.int8.onnx"),
            joiner: path("joiner.int8.onnx"),
        };
        config.model_config.tokens = path("tokens.txt");
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = threads.clamp(1, 8) as i32;
        let recognizer =
            OfflineRecognizer::create(&config).ok_or_else(|| format!("model {} failed to load", spec.id))?;
        Ok(Dictation { language, recognizer })
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Transcribes a mono recording. Returns an empty string for recordings
    /// that are too short or contain no speech.
    pub fn transcribe(&self, sample_rate: u32, samples: &[f32]) -> String {
        if sample_rate == 0 || (samples.len() as f32) < MIN_SECONDS * sample_rate as f32 {
            return String::new();
        }
        let Ok(audio) = resample(sample_rate, samples) else {
            return String::new();
        };
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(MODEL_RATE as i32, &audio);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| tidy(&r.text)).unwrap_or_default()
    }
}

/// Converts a mono recording to 16 kHz.
fn resample(rate: u32, samples: &[f32]) -> Result<Vec<f32>, String> {
    if rate == MODEL_RATE {
        return Ok(samples.to_vec());
    }
    let mut resampler =
        Fft::<f32>::new(rate as usize, MODEL_RATE as usize, 1024, 1, FixedSync::Input).map_err(|e| e.to_string())?;
    let input = InterleavedSlice::new(samples, 1, samples.len()).map_err(|e| e.to_string())?;
    let output = resampler
        .process_all(&input, samples.len(), None)
        .map_err(|e| e.to_string())?;
    Ok(output.take_data())
}

/// Collapses whitespace runs and trims, so the text can be typed as is.
fn tidy(text: &str) -> String {
    commands(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Spoken line breaks, in both languages: what is said and what is typed.
const COMMANDS: [(&str, &str); 4] = [
    ("neuer absatz", "\n\n"),
    ("neue zeile", "\n"),
    ("new paragraph", "\n\n"),
    ("new line", "\n"),
];

/// Replaces spoken commands ("neue Zeile", "new paragraph") with line
/// breaks. The model writes them as words with punctuation around them
/// ("Hallo. Neue Zeile. Wie geht's?"); that punctuation goes too.
pub fn commands(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let boundary = |i: usize| i >= chars.len() || !chars[i].is_alphanumeric();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < chars.len() {
        if i == 0 || boundary(i - 1) {
            for (spoken, typed) in COMMANDS {
                let n = spoken.chars().count();
                if i + n <= chars.len() && lower[i..i + n].iter().copied().eq(spoken.chars()) && boundary(i + n) {
                    // A comma before the command goes; a full stop stays.
                    while out.ends_with([' ', ',']) {
                        out.pop();
                    }
                    out.push_str(typed);
                    i += n;
                    while i < chars.len() && matches!(chars[i], '.' | ',' | '!' | '?' | ';' | ':' | ' ') {
                        i += 1;
                    }
                    continue 'outer;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.trim_matches(' ').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        assert_eq!(Language::from_code("de-AT"), Some(Language::German));
        assert_eq!(Language::from_code("en_US"), Some(Language::English));
        assert_eq!(Language::from_code("EN"), Some(Language::English));
        assert_eq!(Language::from_code("fr-FR"), None);
    }

    #[test]
    fn spoken_line_breaks() {
        assert_eq!(commands("Hallo. Neue Zeile. Wie geht's?"), "Hallo.\nWie geht's?");
        assert_eq!(commands("Liebe Grüße, neuer Absatz Flo"), "Liebe Grüße\n\nFlo");
        assert_eq!(
            commands("First point. New line. Second point."),
            "First point.\nSecond point."
        );
        assert_eq!(commands("Die neue Zeilenlänge passt."), "Die neue Zeilenlänge passt.");
        assert_eq!(commands("Kein Befehl hier."), "Kein Befehl hier.");
    }

    #[test]
    fn tidy_whitespace() {
        assert_eq!(tidy("  Hallo   Welt. \n"), "Hallo Welt.");
    }

    #[test]
    fn specs_are_consistent() {
        for lang in Language::ALL {
            let spec = lang.model();
            assert_eq!(spec.files.len(), 4);
            for f in spec.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(f.url.ends_with(f.name));
            }
        }
    }
}
