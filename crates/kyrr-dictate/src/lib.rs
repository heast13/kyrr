//! kyrr-dictate: speech to text for German, English and other European
//! languages, fully local.
//!
//! [`Dictation`] loads one [`ModelSpec`] and turns a recording
//! (any sample rate, mono) into text. Models are downloaded on demand with
//! [`download::download`] and pinned by SHA256.

pub mod download;
pub mod live;
mod model;
pub mod numerals;
mod stitch;

use std::path::Path;

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};

pub use live::{Live, Word};
pub use model::{european, Language, ModelFile, ModelSpec};
use stitch::{Rules, Stitcher};

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

/// One step of the models' output: a word is placed this exactly.
const STEP_SECONDS: f32 = 0.08;

/// ONNX Runtime settings for the speech models, which sherpa-onnx reads
/// from a file (see [`provider`]):
/// - No memory arena. An arena keeps the working memory of the longest
///   piece (about 600 MB for 30 s) until the model is let go; without it
///   that memory goes back after each piece, for about 5 % more time.
/// - Threads waiting for work sleep instead of spinning: a sixth less CPU
///   time at the same speed, which leaves the noise filter and other apps
///   their share.
const ORT_CONFIG: &str = "EnableCpuMemArena=0\nSessionConfig.session.intra_op.allow_spinning=0\n";

/// The sherpa-onnx provider: the CPU with the settings above, from a file
/// in `root`. sherpa-onnx ends the program when that file cannot be read,
/// so it is only named when it is there, as it should be, under a plain
/// path; otherwise the defaults.
fn provider(root: &Path) -> String {
    let path = root.join("onnxruntime.config");
    let ready = |p: &Path| std::fs::read_to_string(p).is_ok_and(|s| s == ORT_CONFIG);
    let usable = path.to_str().filter(|p| p.trim() == *p && !p.contains('\n'));
    match usable {
        Some(name) if ready(&path) || (std::fs::write(&path, ORT_CONFIG).is_ok() && ready(&path)) => {
            format!("cpu:{name}")
        }
        _ => "cpu".into(),
    }
}

/// A loaded dictation model. Loading takes a few seconds and 1 to 1.3 GB of
/// memory, so the app keeps it only while dictation is in use.
pub struct Dictation {
    model: &'static str,
    recognizer: OfflineRecognizer,
}

impl Dictation {
    /// Loads the model `spec` from below `root`. `threads` is the number of
    /// CPU threads used for decoding.
    pub fn load(root: &Path, spec: &'static ModelSpec, threads: usize) -> Result<Dictation, String> {
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
        config.model_config.provider = Some(provider(root));
        let recognizer =
            OfflineRecognizer::create(&config).ok_or_else(|| format!("model {} failed to load", spec.id))?;
        Ok(Dictation {
            model: spec.id,
            recognizer,
        })
    }

    /// The folder name of the loaded model (see [`ModelSpec::id`]).
    pub fn model(&self) -> &'static str {
        self.model
    }

    /// Transcribes a mono recording. Returns an empty string for recordings
    /// that are too short or contain no speech.
    pub fn transcribe(&self, sample_rate: u32, samples: &[f32]) -> String {
        let mut transcript = Transcript::new(sample_rate);
        transcript.push(samples);
        self.finish(transcript)
    }

    /// Recognises the oldest piece of a running recording once it is
    /// complete, so that at the end only the last piece is left (see
    /// [`Transcript`]). True when it recognised one.
    pub fn ahead(&self, transcript: &mut Transcript) -> bool {
        let Some(piece) = transcript.complete_piece() else {
            return false;
        };
        let text = self.recognize(&transcript.audio[..piece]);
        transcript.texts.extend(text);
        transcript.audio.drain(..piece);
        transcript.ahead += 1;
        true
    }

    /// The text of a whole recording: the pieces recognised ahead and the
    /// rest. Empty for recordings that are too short or contain no speech.
    pub fn finish(&self, mut transcript: Transcript) -> String {
        let Some(resampler) = transcript.resampler.take() else {
            return String::new();
        };
        if (transcript.taken as f32) < MIN_SECONDS * transcript.rate as f32 {
            return String::new();
        }
        if let Some(resampler) = resampler {
            resampler.finish(&mut transcript.audio);
        }
        let mut texts = transcript.texts;
        for piece in pieces(&transcript.audio) {
            texts.extend(self.recognize(piece));
        }
        tidy(&texts.join(" "))
    }

    /// The paragraphs of a long recording that are complete once its next
    /// piece is: where each starts, in seconds, and its text. None while
    /// there is no complete piece, maybe empty when a piece ends no
    /// paragraph. See [`Paragraphs`].
    pub fn paragraph(&self, paragraphs: &mut Paragraphs) -> Option<Vec<(f32, String)>> {
        let transcript = &mut paragraphs.transcript;
        let piece = transcript.complete_piece()?;
        let words = self.piece_words(&transcript.audio[..piece], paragraphs.done);
        transcript.audio.drain(..piece);
        paragraphs.done += piece;
        Some(paragraphs.stitcher.push(words, false))
    }

    /// The paragraphs left once the whole recording is pushed.
    pub fn last_paragraphs(&self, paragraphs: Paragraphs) -> Vec<(f32, String)> {
        let Paragraphs {
            mut transcript,
            done,
            mut stitcher,
        } = paragraphs;
        let Some(resampler) = transcript.resampler.take() else {
            return Vec::new();
        };
        if let Some(resampler) = resampler {
            resampler.finish(&mut transcript.audio);
        }
        let shortest = (MIN_SECONDS * MODEL_RATE as f32) as usize;
        let pieces = pieces(&transcript.audio);
        let mut at = done;
        let mut out = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            if piece.len() >= shortest {
                let words = self.piece_words(piece, at);
                out.extend(stitcher.push(words, i + 1 == pieces.len()));
            }
            at += piece.len();
        }
        out.extend(stitcher.finish());
        out
    }

    /// The words of a piece of 16 kHz audio that starts `at` samples into
    /// the recording, with their times from the start of the recording.
    fn piece_words(&self, audio: &[f32], at: usize) -> Vec<Word> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(MODEL_RATE as i32, audio);
        self.recognizer.decode(&stream);
        let offset = at as f32 / MODEL_RATE as f32;
        let mut words = stream.get_result().map_or_else(Vec::new, |r| {
            let timed = spoken_words(
                &r.tokens,
                r.timestamps.as_deref().unwrap_or_default(),
                r.durations.as_deref(),
            );
            // A model without times gives its text as one stretch of words.
            if timed.is_empty() && !r.text.trim().is_empty() {
                let end = audio.len() as f32 / MODEL_RATE as f32;
                return vec![Word {
                    text: r.text.trim().to_string(),
                    start: 0.0,
                    end,
                }];
            }
            timed
        });
        for word in &mut words {
            word.start += offset;
            word.end += offset;
        }
        words
    }

    fn recognize(&self, audio: &[f32]) -> Option<String> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(MODEL_RATE as i32, audio);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| r.text)
    }

    /// The words of a short recording (up to `PIECE_SECONDS`) and where
    /// they were heard, for following a recording while it runs (see
    /// [`Live`]). The words are as the model wrote them: a spoken line
    /// break ("neue Zeile") stays words here.
    pub fn words(&self, sample_rate: u32, samples: &[f32]) -> Vec<Word> {
        if sample_rate == 0 || (samples.len() as f32) < MIN_SECONDS * sample_rate as f32 {
            return Vec::new();
        }
        let Ok(audio) = resample(sample_rate, samples) else {
            return Vec::new();
        };
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(MODEL_RATE as i32, &audio);
        self.recognizer.decode(&stream);
        stream.get_result().map_or_else(Vec::new, |r| {
            words(&r.tokens, &r.timestamps.unwrap_or_default(), r.durations.as_deref())
        })
    }
}

/// Joins the model's tokens to words: a token starting with a space starts
/// a word, the others (the rest of a word, punctuation) go with the one
/// before. A word ends where its last token does; models without token
/// durations give each token one step.
fn words(tokens: &[String], starts: &[f32], durations: Option<&[f32]>) -> Vec<Word> {
    join(tokens, starts, durations, false)
}

/// The same, but a word ends where its last letter does, not the stop
/// after it. The model writes a stop late, in the pause that follows, and
/// the pause would be gone.
fn spoken_words(tokens: &[String], starts: &[f32], durations: Option<&[f32]>) -> Vec<Word> {
    join(tokens, starts, durations, true)
}

fn join(tokens: &[String], starts: &[f32], durations: Option<&[f32]>, letters_only: bool) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    // A token of only a space: the next token starts a word, too.
    let mut space = false;
    for (i, token) in tokens.iter().enumerate() {
        let Some(&start) = starts.get(i) else {
            break;
        };
        let end = start
            + durations
                .and_then(|d| d.get(i))
                .copied()
                .unwrap_or(STEP_SECONDS)
                .max(STEP_SECONDS);
        let text = token.trim();
        if text.is_empty() {
            space = letters_only && !token.is_empty();
            continue;
        }
        let stop = letters_only && !text.chars().any(char::is_alphanumeric);
        let new_word = (token.starts_with(' ') || space) && !stop;
        space = false;
        match out.last_mut() {
            Some(word) if !new_word => {
                word.text.push_str(text);
                if !stop {
                    word.end = word.end.max(end);
                }
            }
            _ => out.push(Word {
                text: text.to_string(),
                start,
                end,
            }),
        }
    }
    out
}

/// A recording on its way to text. While a long dictation runs, every
/// piece of it that is complete is recognised ([`Dictation::ahead`]), so
/// after the key only the last piece is left ([`Dictation::finish`]). The
/// pieces and their text are the same as for the whole recording at once.
pub struct Transcript {
    rate: u32,
    /// Converts to the model's rate as the recording comes in; `Some(None)`
    /// when the recording has that rate already, `None` when it cannot be
    /// converted.
    resampler: Option<Option<Stream>>,
    /// Samples taken in, at the recording's rate.
    taken: usize,
    /// The recording at the model's rate, from the end of the last piece
    /// recognised on.
    audio: Vec<f32>,
    texts: Vec<String>,
    ahead: usize,
}

impl Transcript {
    pub fn new(sample_rate: u32) -> Transcript {
        let resampler = match sample_rate {
            0 => None,
            MODEL_RATE => Some(None),
            rate => Stream::new(rate).ok().map(Some),
        };
        Transcript {
            rate: sample_rate,
            resampler,
            taken: 0,
            audio: Vec::new(),
            texts: Vec::new(),
            ahead: 0,
        }
    }

    /// Pieces recognised ahead so far.
    pub fn pieces(&self) -> usize {
        self.ahead
    }

    /// Samples taken in so far: the recording goes on from here.
    pub fn taken(&self) -> usize {
        self.taken
    }

    /// Takes in the next samples of the recording.
    pub fn push(&mut self, samples: &[f32]) {
        self.taken += samples.len();
        match &mut self.resampler {
            Some(Some(resampler)) => resampler.push(samples, &mut self.audio),
            Some(None) => self.audio.extend_from_slice(samples),
            None => {}
        }
    }

    /// The length of the oldest piece, once nothing that comes later can
    /// change where it ends.
    fn complete_piece(&self) -> Option<usize> {
        let shortest = (MIN_SECONDS * MODEL_RATE as f32) as usize;
        if self.audio.len() < PIECE_SECONDS * MODEL_RATE as usize + shortest {
            return None;
        }
        cut(&self.audio)
    }
}

/// A long recording (a voice memo) on its way to text, in paragraphs. The
/// model hears it in pieces of about 30 s that end at the quietest spot;
/// the words of all pieces are put together again and a paragraph starts
/// where a sentence ends and the speaker paused (see the `stitch` module), so a
/// paragraph knows where in the recording it starts. Pushed bit by bit,
/// only the newest piece and the paragraph under way wait in memory.
pub struct Paragraphs {
    transcript: Transcript,
    /// Samples at the model's rate that are words already.
    done: usize,
    stitcher: Stitcher,
}

impl Paragraphs {
    pub fn new(sample_rate: u32) -> Paragraphs {
        Paragraphs {
            transcript: Transcript::new(sample_rate),
            done: 0,
            stitcher: Stitcher::new(Rules::default()),
        }
    }

    /// Takes in the next samples of the recording.
    pub fn push(&mut self, samples: &[f32]) {
        self.transcript.push(samples);
    }
}

/// Where the first piece of a 16 kHz recording ends: at the quietest spot
/// in the end of its stretch, so a word is rarely split. None when the
/// recording fits into one piece.
fn cut(audio: &[f32]) -> Option<usize> {
    let limit = PIECE_SECONDS * MODEL_RATE as usize;
    if audio.len() <= limit {
        return None;
    }
    let search = SEARCH_SECONDS * MODEL_RATE as usize;
    // A tail shorter than the shortest recording worth decoding stays with
    // the piece before it.
    let shortest = (MIN_SECONDS * MODEL_RATE as f32) as usize;
    let from = limit - search;
    let quietest = audio[from..limit]
        .as_chunks::<WINDOW>()
        .0
        .iter()
        .enumerate()
        .map(|(i, w)| (i, w.iter().map(|s| s * s).sum::<f32>()))
        // The latest of equally quiet windows: digital silence is common.
        .min_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map_or(limit, |(i, _)| from + i * WINDOW + WINDOW / 2);
    Some(quietest.min(audio.len().saturating_sub(shortest)).max(1))
}

/// Cuts a 16 kHz recording into pieces of at most `PIECE_SECONDS` (see
/// [`cut`]). A recording that fits is one piece.
fn pieces(audio: &[f32]) -> Vec<&[f32]> {
    let mut out = Vec::new();
    let mut rest = audio;
    while let Some(at) = cut(rest) {
        let (piece, tail) = rest.split_at(at);
        out.push(piece);
        rest = tail;
    }
    out.push(rest);
    out
}

/// Converts a mono recording to 16 kHz as it comes in, with the same result
/// as converting it all at once with rubato's `process_all`: whole chunks
/// as they fill up, the last (partial) one when the recording ends.
struct Stream {
    resampler: Fft<f32>,
    /// Input not converted yet. At least one sample stays here, so the last
    /// chunk is a partial one, as in `process_all`.
    pending: Vec<f32>,
    out: Vec<f32>,
    taken: usize,
    /// Output samples still to drop (the resampler's delay), and kept so far.
    skip: usize,
    made: usize,
}

impl Stream {
    fn new(rate: u32) -> Result<Stream, String> {
        let resampler = Fft::<f32>::new(rate as usize, MODEL_RATE as usize, 1024, 1, FixedSync::Input)
            .map_err(|e| e.to_string())?;
        Ok(Stream {
            pending: Vec::new(),
            out: vec![0.0; resampler.output_frames_max()],
            taken: 0,
            skip: resampler.output_delay(),
            made: 0,
            resampler,
        })
    }

    fn push(&mut self, samples: &[f32], to: &mut Vec<f32>) {
        self.taken += samples.len();
        self.pending.extend_from_slice(samples);
        let mut used = 0;
        while self.pending.len() - used > self.resampler.input_frames_next() {
            match self.convert(used, None, to) {
                0 => break,
                n => used += n,
            }
        }
        self.pending.drain(..used);
    }

    /// Converts the rest and the resampler's delay: `to` then holds as many
    /// samples as `process_all` gives.
    fn finish(mut self, to: &mut Vec<f32>) {
        let expected = (self.resampler.resample_ratio() * self.taken as f64).ceil() as usize;
        let rest = self.pending.len();
        if rest > 0 {
            self.convert(0, Some(rest), to);
        }
        while self.made < expected {
            let before = (self.made, self.skip);
            self.convert(0, Some(0), to);
            if (self.made, self.skip) == before {
                break;
            }
        }
        let extra = self.made.saturating_sub(expected);
        to.truncate(to.len().saturating_sub(extra));
    }

    /// Converts one chunk from `pending[at..]` (`partial`: only that many
    /// samples, silence after them). Returns the samples used.
    fn convert(&mut self, at: usize, partial: Option<usize>, to: &mut Vec<f32>) -> usize {
        let indexing = Indexing {
            input_offset: at,
            output_offset: 0,
            partial_len: partial,
            active_channels_mask: None,
        };
        let frames = self.pending.len();
        let capacity = self.out.len();
        let (Ok(input), Ok(mut output)) = (
            InterleavedSlice::new(&self.pending[..], 1, frames),
            InterleavedSlice::new_mut(&mut self.out[..], 1, capacity),
        ) else {
            return 0;
        };
        let Ok((used, made)) = self.resampler.process_into_buffer(&input, &mut output, Some(&indexing)) else {
            return 0;
        };
        let dropped = made.min(self.skip);
        self.skip -= dropped;
        to.extend_from_slice(&self.out[dropped..made]);
        self.made += made - dropped;
        used
    }
}

/// Converts a whole mono recording to 16 kHz.
fn resample(rate: u32, samples: &[f32]) -> Result<Vec<f32>, String> {
    if rate == MODEL_RATE {
        return Ok(samples.to_vec());
    }
    let mut stream = Stream::new(rate)?;
    let mut out = Vec::new();
    stream.push(samples, &mut out);
    stream.finish(&mut out);
    Ok(out)
}

/// A mono recording at the rate the models (and speech servers) want, 16 kHz.
pub fn at_model_rate(rate: u32, samples: &[f32]) -> Vec<f32> {
    resample(rate, samples).unwrap_or_default()
}

/// Collapses whitespace runs and trims, and turns spoken line breaks into
/// real ones, so the text can be typed as is.
pub fn tidy(text: &str) -> String {
    commands(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Spoken line breaks, in both languages: what is said and what is typed.
/// The window names the first of each; the others are what people say
/// without thinking about it.
const COMMANDS: [(&str, &str); 11] = [
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

    /// Not quite noise, not quite a tone: something to resample.
    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| ((i as f32 * 0.37).sin() + (i as f32 * 0.011).cos()) * 0.4)
            .collect()
    }

    #[test]
    fn converting_as_it_comes_in_is_converting_at_once() {
        for rate in [44_100, 48_000, 22_050] {
            for len in [1, 500, 1023, 1024, 1025, 5000, rate as usize * 3 + 17] {
                let samples = signal(len);
                let mut whole = Fft::<f32>::new(rate as usize, MODEL_RATE as usize, 1024, 1, FixedSync::Input).unwrap();
                let input = InterleavedSlice::new(&samples[..], 1, len).unwrap();
                let expected = whole.process_all(&input, len, None).unwrap().take_data();
                for step in [1, 333, 1024, 4800, len] {
                    let mut stream = Stream::new(rate).unwrap();
                    let mut out = Vec::new();
                    for part in samples.chunks(step) {
                        stream.push(part, &mut out);
                    }
                    stream.finish(&mut out);
                    assert_eq!(out, expected, "{rate} Hz, {len} samples in steps of {step}");
                }
            }
        }
    }

    #[test]
    fn pieces_recognised_ahead_are_the_pieces_of_the_whole() {
        let rate = MODEL_RATE as usize;
        // Speech with short pauses here and there, 100 s.
        let audio: Vec<f32> = signal(100 * rate)
            .into_iter()
            .enumerate()
            .map(|(i, s)| if (i / 1000) % 37 == 0 { 0.0 } else { s })
            .collect();
        let whole: Vec<usize> = pieces(&audio).iter().map(|p| p.len()).collect();
        let mut transcript = Transcript::new(MODEL_RATE);
        let mut ahead = Vec::new();
        for part in audio.chunks(rate / 3) {
            transcript.push(part);
            while let Some(n) = transcript.complete_piece() {
                ahead.push(n);
                transcript.audio.drain(..n);
            }
        }
        assert_eq!(ahead.len(), whole.len() - 1);
        let rest: Vec<usize> = pieces(&transcript.audio).iter().map(|p| p.len()).collect();
        assert_eq!([ahead, rest].concat(), whole);
    }

    #[test]
    fn tokens_join_to_words() {
        let tokens: Vec<String> = [" Wir", " lan", "ge", ".", " K", "ann"].map(String::from).to_vec();
        let starts = [0.0, 0.4, 0.56, 0.8, 1.0, 1.16];
        let durations = [0.16, 0.16, 0.24, 0.08, 0.16, 0.0];
        let timed = words(&tokens, &starts, Some(&durations));
        let text: Vec<&str> = timed.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(text, ["Wir", "lange.", "Kann"]);
        assert_eq!((timed[1].start, timed[1].end), (0.4, 0.88));
        // No duration: one step.
        assert!((timed[2].end - 1.24).abs() < 1e-6);
        assert_eq!(words(&tokens, &starts, None)[0].end, STEP_SECONDS);
        // The stop comes late, in the pause after the word: not part of it.
        let spoken = spoken_words(&tokens, &starts, Some(&durations));
        assert_eq!((spoken[1].start, spoken[1].end), (0.4, 0.8));
        // A lone space and a stop with a space before it.
        let tokens: Vec<String> = [" Hallo", " ", "Welt", " ."].map(String::from).to_vec();
        let said = spoken_words(&tokens, &[0.0, 0.3, 0.4, 0.8], Some(&[0.3, 0.1, 0.3, 0.1]));
        let text: Vec<&str> = said.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(text, ["Hallo", "Welt."]);
        assert!((said[1].end - 0.7).abs() < 1e-6);
    }

    #[test]
    fn tidy_whitespace() {
        assert_eq!(tidy("  Hallo   Welt. \n"), "Hallo Welt.");
    }

    #[test]
    fn specs_are_consistent() {
        let specs = Language::ALL.map(Language::model).into_iter().chain([european()]);
        for spec in specs {
            assert_eq!(spec.files.len(), 4);
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
