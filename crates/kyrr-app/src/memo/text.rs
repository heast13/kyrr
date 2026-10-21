//! Writing down what was said in a memo, with the dictation model: its
//! title from the start, and on a click the whole text, a paragraph at a
//! time and beside dictation, which goes first.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kyrr_dictate::{Dictation, Paragraphs};

use super::library::{self, Paragraph};
use super::opusfile::{self, Reader, GRANULE_RATE};
use super::title;

/// Sound read from the file at a time.
const READ_SECONDS: usize = 2;
/// A paragraph's audio waits this long, at most, before it is text; the
/// progress counts it as not done yet.
const BEHIND_SECONDS: f64 = 30.0;

/// The title for a memo that starts with these samples.
pub fn title(model: &Dictation, rate: u32, samples: &[f32]) -> Option<String> {
    title::from_text(&model.transcribe(rate, samples))
}

/// How a memo's text comes along.
pub enum Done {
    /// Written next to the memo.
    Written,
    Cancelled,
}

/// Writes the text of the memo at `path` next to it. `progress` hears how
/// far it is (0 to 1) and about how long it still takes. While `pause` is
/// set (a dictation runs) it waits.
pub fn write(
    model: &Dictation,
    path: &Path,
    cancel: &AtomicBool,
    pause: &AtomicBool,
    progress: &mut dyn FnMut(f32, Option<Duration>),
) -> Result<Done, String> {
    let total = opusfile::seconds(path).unwrap_or(0.0).max(1.0);
    let mut reader = Reader::open(path)?;
    let mut paragraphs = Paragraphs::new(GRANULE_RATE);
    let mut found = Vec::new();
    let mut samples = Vec::new();
    let mut read = 0usize;
    let mut working = Duration::ZERO;
    let mut ended = false;
    // Waits while a dictation runs or is typed; false once cancelled.
    let go_on = || {
        while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
        !cancel.load(Ordering::Relaxed)
    };
    while !ended {
        if !go_on() {
            return Ok(Done::Cancelled);
        }
        let started = Instant::now();
        samples.clear();
        while samples.len() < READ_SECONDS * GRANULE_RATE as usize {
            if !reader.next(&mut samples) {
                ended = true;
                break;
            }
        }
        read += samples.len();
        paragraphs.push(&samples);
        let mut heard = false;
        let mut busy = started.elapsed();
        loop {
            if !go_on() {
                return Ok(Done::Cancelled);
            }
            let started = Instant::now();
            let paragraph = model.paragraph(&mut paragraphs);
            busy += started.elapsed();
            let Some(paragraph) = paragraph else { break };
            found.push(paragraph);
            heard = true;
        }
        working += busy;
        if heard {
            let done = ((read as f64 / GRANULE_RATE as f64 - BEHIND_SECONDS) / total).clamp(0.0, 0.99);
            let left = (done > 0.05).then(|| working.mul_f64((1.0 - done) / done));
            progress(done as f32, left);
        }
    }
    if !go_on() {
        return Ok(Done::Cancelled);
    }
    found.extend(model.last_paragraphs(paragraphs));
    let paragraphs: Vec<Paragraph> = found
        .into_iter()
        .map(|(seconds, text)| Paragraph {
            seconds: seconds as u32,
            text,
        })
        .collect();
    // Deleted meanwhile: nothing to put beside it.
    if !path.exists() {
        return Ok(Done::Cancelled);
    }
    let written = library::write_text(path, &paragraphs).map_err(|e| e.to_string())?;
    // Deleted or renamed while it was written: no text without its memo.
    if !path.exists() {
        let _ = std::fs::remove_file(written);
        return Ok(Done::Cancelled);
    }
    progress(1.0, None);
    Ok(Done::Written)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::memo::opusfile::Writer;

    /// A 16-bit mono WAV file's rate and samples.
    fn wav(path: &Path) -> (u32, Vec<f32>) {
        let data = std::fs::read(path).expect("wav reads");
        let rate = u32::from_le_bytes(data[24..28].try_into().unwrap());
        let mut at = 12;
        while &data[at..at + 4] != b"data" {
            at += 8 + u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        }
        let samples = data[at + 8..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0)
            .collect();
        (rate, samples)
    }

    /// The whole way of a memo's text with the German model, from an Opus
    /// file to the `.txt` beside it. Needs the model in `models/` and
    /// `scripts/fetch-assets.py --test`; skipped otherwise.
    #[test]
    fn a_long_memo_becomes_paragraphs_with_times() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (models, clip) = (root.join("models"), root.join("assets/test-audio/de.wav"));
        let spec = kyrr_dictate::Language::German.model();
        if !spec.is_installed(&models) || !clip.is_file() {
            eprintln!("German model or test audio missing, skipped");
            return;
        }
        let model = Dictation::load(&models, spec, 4).expect("model loads");
        let (rate, said) = wav(&clip);
        // The sentence 24 times with a breath between: over a minute.
        let mut memo = Vec::new();
        for _ in 0..24 {
            memo.extend_from_slice(&said);
            memo.extend(std::iter::repeat_n(0.0, rate as usize / 2));
        }
        let step = rate as f64 / 16_000.0;
        let at_16k: Vec<f32> = (0..(memo.len() as f64 / step) as usize - 1)
            .map(|i| {
                let (at, frac) = ((i as f64 * step) as usize, (i as f64 * step).fract() as f32);
                memo[at] + (memo[at + 1] - memo[at]) * frac
            })
            .collect();
        let dir = std::env::temp_dir().join(format!("kyrr-memo-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-10-21 14-05 Aufnahme.opus");
        let mut writer = Writer::create(&path, 16_000).unwrap();
        writer.push(&at_16k).unwrap();
        writer.finish().unwrap();

        assert_eq!(
            title(&model, rate, &said).as_deref(),
            Some("Alles hat ein Ende nur die")
        );
        let mut steps = Vec::new();
        let (cancel, pause) = (AtomicBool::new(false), AtomicBool::new(false));
        let done = write(&model, &path, &cancel, &pause, &mut |p, _| steps.push(p)).unwrap();
        assert!(matches!(done, Done::Written));
        assert_eq!(steps.last(), Some(&1.0));
        let paragraphs = library::read_text(&path).expect("the text is there");
        let _ = std::fs::remove_dir_all(&dir);
        for p in &paragraphs {
            println!("[{}] {}", library::clock(p.seconds), p.text);
        }
        assert!(paragraphs.len() >= 2, "{} paragraphs", paragraphs.len());
        assert_eq!(paragraphs[0].seconds, 0);
        assert!(paragraphs.windows(2).all(|w| w[0].seconds < w[1].seconds));
        let all = library::plain(&paragraphs);
        let heard = all.matches("Wurst").count();
        assert!(heard >= 22, "{heard} of 24 sentences:\n{all}");
    }
}
