//! kyrr-dictate: speech to text for German, English, Chinese, Japanese and
//! Korean, fully local.
//!
//! [`Dictation`] loads the model for one [`Speech`] and turns a recording
//! (any sample rate, mono) into text. Models are downloaded on demand with
//! [`download::download`] and pinned by SHA256.

pub mod download;
mod model;

use std::path::Path;

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, OfflineTransducerModelConfig,
};

pub use model::{Language, ModelFile, ModelKind, ModelSpec, Speech};

/// Shortest recording worth decoding; shorter ones are accidental key taps.
pub const MIN_SECONDS: f32 = 0.3;

/// Rate the models expect. Recordings are resampled here rather than inside
/// sherpa-onnx, which logs every resampler it creates.
const MODEL_RATE: u32 = 16_000;

/// Longest piece handed to the model in one go. Attention memory and time
/// grow with the square of the length, so a long dictation is cut into
/// pieces of about this size (at pauses) and the texts are joined.
const PIECE_SECONDS: usize = 30;

/// The cut is looked for in the last stretch of a piece, so no piece gets
/// much shorter than `PIECE_SECONDS - SEARCH_SECONDS`.
const SEARCH_SECONDS: usize = 8;

/// Length of the windows compared when looking for the quietest spot.
const WINDOW: usize = MODEL_RATE as usize / 50;

/// A loaded dictation model. Loading takes a few seconds and about 1 GB of
/// memory, so the app keeps it only while dictation is in use.
pub struct Dictation {
    speech: Speech,
    recognizer: OfflineRecognizer,
}

impl Dictation {
    /// Loads the model for `speech` from below `root`. `threads` is the
    /// number of CPU threads used for decoding.
    pub fn load(root: &Path, speech: Speech, threads: usize) -> Result<Dictation, String> {
        let spec = speech.model();
        if !spec.is_installed(root) {
            return Err(format!("model {} is not installed", spec.id));
        }
        let dir = spec.dir(root);
        let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
        let mut config = OfflineRecognizerConfig::default();
        match spec.kind {
            ModelKind::Transducer => {
                config.feat_config.feature_dim = 128;
                config.model_config.transducer = OfflineTransducerModelConfig {
                    encoder: path("encoder.int8.onnx"),
                    decoder: path("decoder.int8.onnx"),
                    joiner: path("joiner.int8.onnx"),
                };
                config.model_config.model_type = Some("nemo_transducer".into());
            }
            ModelKind::SenseVoice => {
                // With the language given the model does not have to guess
                // it, and punctuation and numerals come written out.
                config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                    model: path("model.int8.onnx"),
                    language: Some(speech.code().into()),
                    use_itn: true,
                };
            }
        }
        config.model_config.tokens = path("tokens.txt");
        config.model_config.num_threads = threads.clamp(1, 8) as i32;
        let recognizer =
            OfflineRecognizer::create(&config).ok_or_else(|| format!("model {} failed to load", spec.id))?;
        Ok(Dictation { speech, recognizer })
    }

    pub fn speech(&self) -> Speech {
        self.speech
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
        let mut texts = Vec::new();
        for piece in pieces(&audio) {
            let stream = self.recognizer.create_stream();
            stream.accept_waveform(MODEL_RATE as i32, piece);
            self.recognizer.decode(&stream);
            texts.extend(stream.get_result().map(|r| r.text));
        }
        tidy(&texts.join(" "), self.speech)
    }
}

/// Cuts a 16 kHz recording into pieces of at most `PIECE_SECONDS`, each at
/// the quietest spot in the end of its stretch, so a word is rarely split.
/// A recording that fits is one piece.
fn pieces(audio: &[f32]) -> Vec<&[f32]> {
    let limit = PIECE_SECONDS * MODEL_RATE as usize;
    let search = SEARCH_SECONDS * MODEL_RATE as usize;
    let mut out = Vec::new();
    let mut rest = audio;
    // A tail shorter than the shortest recording worth decoding stays with
    // the piece before it.
    let shortest = (MIN_SECONDS * MODEL_RATE as f32) as usize;
    while rest.len() > limit {
        let from = limit - search;
        let quietest = rest[from..limit]
            .as_chunks::<WINDOW>()
            .0
            .iter()
            .enumerate()
            .map(|(i, w)| (i, w.iter().map(|s| s * s).sum::<f32>()))
            // The latest of equally quiet windows: digital silence is common.
            .min_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
            .map_or(limit, |(i, _)| from + i * WINDOW + WINDOW / 2);
        let quietest = quietest.min(rest.len().saturating_sub(shortest)).max(1);
        let (piece, tail) = rest.split_at(quietest);
        out.push(piece);
        rest = tail;
    }
    out.push(rest);
    out
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

/// Collapses whitespace runs and trims, so the text can be typed as is. In
/// Chinese and Japanese, which are written without spaces, the model's gaps
/// between words go too; in all three Asian languages a lone character is
/// what the model makes of silence and noise, so it counts as nothing.
fn tidy(text: &str, speech: Speech) -> String {
    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if speech.model().kind == ModelKind::SenseVoice && text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return String::new();
    }
    if speech.spaceless() {
        // A command is found with the gaps the model left ("改行 次の") and
        // again without them ("新しい 段落").
        text = close_gaps(&commands(&text));
    }
    commands(&text)
}

/// Whether `c` is written without spaces around it: Han characters, kana,
/// their punctuation and the full-width forms.
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

/// Removes the spaces that touch a wide character: the model puts them
/// between Japanese words, and nobody types them there. Spaces between two
/// Latin words stay.
fn close_gaps(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let touches = |j: Option<usize>| j.and_then(|j| chars.get(j)).is_some_and(|c| is_wide(*c));
        if c == ' ' && (touches(i.checked_sub(1)) || touches(Some(i + 1))) {
            continue;
        }
        out.push(c);
    }
    out
}

/// Spoken line breaks, in every language: what is said and what is typed.
/// The window names the first of each; the others are what people say
/// without thinking about it.
const COMMANDS: [(&str, &str); 22] = [
    ("neuer absatz", "\n\n"),
    ("nächster absatz", "\n\n"),
    ("neue zeile", "\n"),
    ("nächste zeile", "\n"),
    ("zeilenumbruch", "\n"),
    ("new paragraph", "\n\n"),
    ("next paragraph", "\n\n"),
    ("new line", "\n"),
    ("next line", "\n"),
    ("newline", "\n"),
    ("line break", "\n"),
    ("新段落", "\n\n"),
    ("换行", "\n"),
    ("換行", "\n"),
    ("新しい段落", "\n\n"),
    ("改行", "\n"),
    ("새 문단", "\n\n"),
    ("새 단락", "\n\n"),
    ("줄바꿈", "\n"),
    ("줄 바꿈", "\n"),
    ("새 줄", "\n"),
    ("새줄", "\n"),
];

/// Replaces spoken commands ("neue Zeile", "new paragraph", "换行") with line
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
                    while out.ends_with([' ', ',', '，', '、']) {
                        out.pop();
                    }
                    out.push_str(typed);
                    i += n;
                    while i < chars.len()
                        && matches!(
                            chars[i],
                            '.' | ',' | '!' | '?' | ';' | ':' | ' ' | '。' | '，' | '、' | '！' | '？' | '；' | '：'
                        )
                    {
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
        assert_eq!(Speech::from_code("ja-JP"), Some(Speech::Japanese));
        assert_eq!(Speech::from_code("zh_CN"), Some(Speech::Chinese));
        assert_eq!(Speech::from_code("KO"), Some(Speech::Korean));
        assert_eq!(Speech::from_code("fr"), None);
        for speech in Speech::ALL {
            assert_eq!(Speech::from_code(speech.code()), Some(speech));
        }
        assert_eq!(Language::German.speech(), Speech::German);
        assert_eq!(Language::English.speech(), Speech::English);
    }

    #[test]
    fn spoken_line_breaks() {
        assert_eq!(commands("Hallo. Neue Zeile. Wie geht's?"), "Hallo.\nWie geht's?");
        assert_eq!(commands("Liebe Grüße, neuer Absatz Flo"), "Liebe Grüße\n\nFlo");
        assert_eq!(
            commands("First point. New line. Second point."),
            "First point.\nSecond point."
        );
        assert_eq!(commands("Hallo, nächste Zeile, Tschüss."), "Hallo\nTschüss.");
        assert_eq!(commands("Eins. Nächster Absatz. Zwei."), "Eins.\n\nZwei.");
        assert_eq!(commands("Eins, Zeilenumbruch, zwei."), "Eins\nzwei.");
        assert_eq!(commands("One. Next line. Two."), "One.\nTwo.");
        assert_eq!(commands("Die neue Zeilenlänge passt."), "Die neue Zeilenlänge passt.");
        assert_eq!(
            commands("In der nächsten Zeile steht es."),
            "In der nächsten Zeile steht es."
        );
        assert_eq!(commands("Kein Befehl hier."), "Kein Befehl hier.");
    }

    #[test]
    fn a_short_recording_is_one_piece() {
        let audio = vec![0.1; PIECE_SECONDS * MODEL_RATE as usize];
        assert_eq!(pieces(&audio), vec![&audio[..]]);
    }

    #[test]
    fn no_piece_is_too_short_to_decode() {
        let limit = PIECE_SECONDS * MODEL_RATE as usize;
        for extra in [1, 100, 4800, 5000] {
            let audio = vec![0.1; limit + extra];
            let cut = pieces(&audio);
            assert_eq!(cut.concat(), audio);
            let shortest = (MIN_SECONDS * MODEL_RATE as f32) as usize;
            assert!(cut.iter().all(|p| p.len() >= shortest), "{extra}");
        }
    }

    #[test]
    fn a_long_recording_is_cut_at_pauses_without_losing_audio() {
        let rate = MODEL_RATE as usize;
        // Speech everywhere except a pause 3 s before each 30 s mark.
        let audio: Vec<f32> = (0..95 * rate)
            .map(|i| {
                let second = i / rate;
                if second % 30 == 27 {
                    0.0
                } else {
                    0.3
                }
            })
            .collect();
        let cut = pieces(&audio);
        assert_eq!(cut.len(), 4);
        assert!(cut.iter().all(|p| p.len() <= PIECE_SECONDS * rate));
        assert_eq!(cut.concat(), audio);
        // The cuts fall into the pauses.
        let mut at = 0;
        for piece in &cut[..3] {
            at += piece.len();
            assert_eq!(audio[at - 1], 0.0);
            assert_eq!(audio[at], 0.0);
        }
    }

    #[test]
    fn tidy_whitespace() {
        assert_eq!(tidy("  Hallo   Welt. \n", Speech::German), "Hallo Welt.");
    }

    #[test]
    fn japanese_and_chinese_have_no_gaps_between_words() {
        assert_eq!(
            tidy(
                "抜群 の運動 神経を 持ち合わせ、どんな要求に も応え てきた。",
                Speech::Japanese
            ),
            "抜群の運動神経を持ち合わせ、どんな要求にも応えてきた。"
        );
        assert_eq!(
            tidy(
                "ヤンマル クイナ と の 出会 いは 18 歳 の 時 だっ た。",
                Speech::Japanese
            ),
            "ヤンマルクイナとの出会いは18歳の時だった。"
        );
        assert_eq!(tidy("我用 iPhone 打 电话", Speech::Chinese), "我用iPhone打电话");
        // Latin words keep their spaces.
        assert_eq!(tidy("今天开 the meeting 吧", Speech::Chinese), "今天开the meeting吧");
        assert_eq!(tidy("see you at noon", Speech::Japanese), "see you at noon");
    }

    #[test]
    fn korean_keeps_its_spaces() {
        assert_eq!(
            tidy("그는  괜찮은 척하려고 애쓰는 것 같았다.", Speech::Korean),
            "그는 괜찮은 척하려고 애쓰는 것 같았다."
        );
    }

    #[test]
    fn a_lone_character_is_what_the_model_makes_of_silence() {
        for speech in [Speech::Chinese, Speech::Japanese, Speech::Korean] {
            assert_eq!(tidy("我.", speech), "");
            assert_eq!(tidy("그.", speech), "");
            assert_eq!(tidy("", speech), "");
        }
        assert_eq!(tidy("好的。", Speech::Chinese), "好的。");
        // Not for the Parakeet models: they return nothing for silence, and
        // "I" or "a" is a fine dictation.
        assert_eq!(tidy("I", Speech::English), "I");
    }

    #[test]
    fn spoken_line_breaks_in_asian_languages() {
        assert_eq!(tidy("你好。换行。再见。", Speech::Chinese), "你好。\n再见。");
        assert_eq!(tidy("你好，新段落，再见", Speech::Chinese), "你好\n\n再见");
        assert_eq!(
            tidy("こんにちは。改行。さようなら。", Speech::Japanese),
            "こんにちは。\nさようなら。"
        );
        assert_eq!(
            tidy("안녕하세요. 줄바꿈. 안녕히 가세요.", Speech::Korean),
            "안녕하세요.\n안녕히 가세요."
        );
        assert_eq!(
            tidy("안녕하세요. 새 줄. 안녕히 가세요.", Speech::Korean),
            "안녕하세요.\n안녕히 가세요."
        );
        // A command alone still counts, and a word that only contains one is no command.
        assert_eq!(tidy("换行", Speech::Chinese), "\n");
        assert_eq!(tidy("改行コードを直す。", Speech::Japanese), "改行コードを直す。");
        assert_eq!(tidy("更换行李。", Speech::Chinese), "更换行李。");
        assert_eq!(
            tidy("こんにちは。 改行 次の 話 です。", Speech::Japanese),
            "こんにちは。\n次の話です。"
        );
        assert_eq!(
            tidy("こんにちは。 新しい 段落。 次の 話", Speech::Japanese),
            "こんにちは。\n\n次の話"
        );
    }

    #[test]
    fn specs_are_consistent() {
        for speech in Speech::ALL {
            let spec = speech.model();
            let expected = match spec.kind {
                ModelKind::Transducer => 4,
                ModelKind::SenseVoice => 2,
            };
            assert_eq!(spec.files.len(), expected, "{}", spec.id);
            for f in spec.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(f.urls.len() >= 2, "{}: one host only", f.name);
                for url in f.urls {
                    assert!(url.ends_with(f.name), "{url}");
                }
            }
        }
    }
}
