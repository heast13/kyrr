//! Paragraphs for a long recording (a voice memo): the words of all pieces
//! in one stream, a new paragraph where a sentence ends and the speaker
//! paused. The pieces the model is given are cut for its sake (about 30 s,
//! at the quietest spot); where a paragraph starts is a matter of the text.

use crate::{tidy, Word};

/// When a paragraph starts. Seconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rules {
    /// A sentence end followed by a pause this long starts a paragraph,
    /// once the paragraph is `shortest` long.
    pub(crate) pause: f32,
    /// Paragraphs shorter than this are not split at a pause of `pause`.
    pub(crate) shortest: f32,
    /// A paragraph this long ends at the next sentence end, however short
    /// the pause.
    pub(crate) long: f32,
    /// A pause this long after a sentence end starts a paragraph however
    /// short it is; between `pause` and this the shortest paragraph shrinks.
    pub(crate) break_pause: f32,
    /// A pause this long at the end of a piece is a sentence end, stop or
    /// not.
    pub(crate) silence: f32,
    /// The least pause that makes a full stop a sentence end. Below it the
    /// speaker went on, and the stop may be the model's wish to close the
    /// piece (see [`Held::edge`]).
    pub(crate) gap: f32,
    /// A paragraph that reaches this length without a sentence end to cut
    /// at is cut at its longest pause.
    pub(crate) limit: f32,
    /// Words that follow without a pause this long, across the cut of a
    /// piece, are one sentence (see [`Stitcher::glue`]).
    pub(crate) glue: f32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            pause: 1.0,
            shortest: 8.0,
            long: 25.0,
            break_pause: 3.0,
            silence: 2.0,
            gap: 0.3,
            limit: 60.0,
            glue: 0.2,
        }
    }
}

/// A word and whether it ends a piece the model heard on its own.
struct Held {
    word: Word,
    edge: bool,
}

/// Collects the words of a recording, piece by piece, and hands out the
/// paragraphs as they are complete. Only the paragraph under way is kept.
pub(crate) struct Stitcher {
    rules: Rules,
    held: Vec<Held>,
}

impl Stitcher {
    pub(crate) fn new(rules: Rules) -> Stitcher {
        Stitcher {
            rules,
            held: Vec::new(),
        }
    }

    /// Takes the words of the next piece, seconds from the start of the
    /// recording, and returns the paragraphs that are complete now: where
    /// each starts and its text. `last` is the end of the recording: a
    /// piece before that ends where the model was made to stop, so the full
    /// stop it writes there says little.
    pub(crate) fn push(&mut self, words: Vec<Word>, last: bool) -> Vec<(f32, String)> {
        let mut done = Vec::new();
        let n = words.len();
        for (i, mut word) in words.into_iter().enumerate() {
            if self.breaks_before(&word) {
                if let Some(last) = self.held.last_mut() {
                    close(&mut last.word, &mut word);
                }
                done.extend(self.take(self.held.len()));
            } else {
                self.glue(&mut word);
            }
            self.held.push(Held {
                word,
                edge: !last && i + 1 == n,
            });
            if self.length_to(self.held.len() - 1) >= self.rules.limit {
                done.extend(self.cut_at_longest_pause());
            }
        }
        done
    }

    /// The paragraph under way, at the end of the recording.
    pub(crate) fn finish(&mut self) -> Option<(f32, String)> {
        self.take(self.held.len())
    }

    /// Whether `next` starts a new paragraph.
    fn breaks_before(&self, next: &Word) -> bool {
        let (Some(first), Some(last)) = (self.held.first(), self.held.last()) else {
            return false;
        };
        let r = &self.rules;
        let gap = (next.start - last.word.end).max(0.0);
        // The model sometimes leaves out the stop at the end of a piece
        // before a long pause, and puts one in the middle of a sentence.
        // At the end of a piece only a real pause tells.
        let sentence = if last.edge {
            gap >= r.pause && sentence_end(&last.word.text) || gap >= r.silence
        } else {
            gap >= r.gap && sentence_end(&last.word.text)
        };
        if !sentence {
            return false;
        }
        let length = last.word.end - first.word.start;
        // The longer the pause, the shorter a paragraph may be.
        let shortest = r.shortest * ((r.break_pause - gap) / (r.break_pause - r.pause)).clamp(0.0, 1.0);
        length >= r.long || gap >= r.pause && length >= shortest
    }

    /// The model treats the cut of a piece as the end of a sentence, or
    /// not, and starts the next piece with a capital letter either way. At
    /// a cut without a paragraph this puts the text right as far as the
    /// words and the pause tell:
    /// - no pause: the sentence goes on, so a stop goes, and a word that is
    ///   never capitalised in the middle of a sentence loses its capital;
    /// - a short pause and such a word with a capital after a word without
    ///   a stop: a new sentence, the stop is put in.
    fn glue(&mut self, next: &mut Word) {
        let Some(last) = self.held.last_mut().filter(|h| h.edge) else {
            return;
        };
        let (word, r) = (&mut last.word, &self.rules);
        let gap = next.start - word.end;
        let lower = next.text.to_lowercase();
        let small = SMALL_WORDS.contains(&lower.as_str());
        if gap < r.glue {
            if word.text.ends_with('.') && sentence_end(&word.text) {
                word.text.pop();
            }
            if small && !word.text.ends_with(|c: char| ".?!".contains(c)) {
                next.text = lower;
            }
        } else if gap >= r.gap && small && next.text != lower && word.text.ends_with(char::is_alphanumeric) {
            word.text.push('.');
        }
    }

    /// How long the paragraph is up to its word `i`.
    fn length_to(&self, i: usize) -> f32 {
        self.held[i].word.end - self.held[0].word.start
    }

    /// Cuts the paragraph under way where a sentence ends and the pause is
    /// longest, or at the longest pause if no sentence ends, leaving a good
    /// stretch on both sides.
    fn cut_at_longest_pause(&mut self) -> Option<(f32, String)> {
        const SIDE: f32 = 10.0;
        let start = self.held[0].word.start;
        let end = self.held[self.held.len() - 1].word.end;
        let at = (1..self.held.len())
            .filter(|&i| self.held[i - 1].word.end - start >= SIDE && end - self.held[i].word.start >= SIDE / 2.0)
            .max_by(|&a, &b| {
                let key = |i: usize| (sentence_end(&self.held[i - 1].word.text), self.gap_before(i));
                let (a, b) = (key(a), key(b));
                a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
            })?;
        let (before, after) = self.held.split_at_mut(at);
        close(&mut before[at - 1].word, &mut after[0].word);
        self.take(at)
    }

    fn gap_before(&self, i: usize) -> f32 {
        self.held[i].word.start - self.held[i - 1].word.end
    }

    /// The first `n` words as a paragraph.
    fn take(&mut self, n: usize) -> Option<(f32, String)> {
        let start = self.held.first()?.word.start;
        let words: Vec<&str> = self.held[..n].iter().map(|h| h.word.text.as_str()).collect();
        let text = tidy(&words.join(" "));
        self.held.drain(..n);
        (!text.is_empty()).then_some((start, text))
    }
}

/// Words that end in a full stop without ending a sentence.
const ABBREVIATIONS: &[&str] = &[
    "bzw", "ca", "dr", "evtl", "ggf", "hr", "fr", "inkl", "mio", "mrd", "nr", "prof", "st", "vgl", "z", "mr", "mrs",
    "ms", "vs", "no", "approx", "incl", "sr", "jr",
];

/// A paragraph ends with a stop, one that the model left out included, and
/// the next starts with a capital.
fn close(last: &mut Word, next: &mut Word) {
    if last.text.ends_with(char::is_alphanumeric) {
        last.text.push('.');
    }
    let mut letters = next.text.chars();
    if let Some(first) = letters.next().filter(|c| c.is_lowercase()) {
        next.text = first.to_uppercase().chain(letters).collect();
    }
}

/// Words that only start a sentence with a capital: lower case in the
/// middle of one. Not "Sie" and "Ihr" (you, formal) or "I".
const SMALL_WORDS: &[&str] = &[
    "und", "oder", "aber", "denn", "weil", "dass", "da", "dann", "also", "wenn", "als", "wie", "wo", "was", "die",
    "der", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "ich", "du", "er", "es", "wir", "mit",
    "für", "auf", "in", "im", "zu", "zum", "zur", "von", "vom", "bei", "bis", "nach", "über", "unter", "nicht", "noch",
    "auch", "nur", "schon", "ist", "sind", "war", "hat", "haben", "wird", "kann", "muss", "man", "and", "but", "so",
    "because", "that", "then", "when", "the", "a", "an", "he", "she", "it", "we", "they", "to", "of", "on", "for",
    "with", "is", "are", "was", "not", "also", "just",
];

/// Whether a word closes a sentence: it ends in . ? ! or …, leaving out
/// abbreviations ("z.B.", "Dr."), ordinals ("am 3.") and initials.
fn sentence_end(word: &str) -> bool {
    let word = word.trim_end_matches(['"', '\'', ')', '»', '«', '“', '”', '’', '‘']);
    if word.ends_with(['?', '!', '…']) {
        return true;
    }
    let Some(body) = word.strip_suffix('.') else {
        return false;
    };
    let ordinal = !body.is_empty() && body.chars().all(|c| c.is_ascii_digit());
    let initial = body.chars().count() == 1 && body.chars().all(char::is_uppercase);
    body.ends_with('.')
        || !(body.is_empty()
            || ordinal
            || initial
            || body.contains('.')
            || ABBREVIATIONS.contains(&body.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words at their times: (text, start, end).
    fn said(list: &[(&str, f32, f32)]) -> Vec<Word> {
        list.iter()
            .map(|&(text, start, end)| Word {
                text: text.to_string(),
                start,
                end,
            })
            .collect()
    }

    /// A stretch of speech: `n` words of 0.4 s with 0.1 s between, the last
    /// ending in a full stop, from `start`.
    fn sentence(start: f32, n: usize) -> Vec<Word> {
        (0..n)
            .map(|i| Word {
                text: if i + 1 == n { "Ende.".into() } else { "wort".into() },
                start: start + i as f32 * 0.5,
                end: start + i as f32 * 0.5 + 0.4,
            })
            .collect()
    }

    fn stitch(parts: &[Vec<Word>]) -> Vec<(f32, String)> {
        let mut stitcher = Stitcher::new(Rules::default());
        let mut out = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            out.extend(stitcher.push(part.clone(), i + 1 == parts.len()));
        }
        out.extend(stitcher.finish());
        out
    }

    #[test]
    fn sentence_ends() {
        for yes in [
            "Ende.",
            "Wirklich?",
            "Nein!",
            "gut…",
            "sagte er.\"",
            "(fertig.)",
            "ja...",
        ] {
            assert!(sentence_end(yes), "{yes}");
        }
        for no in [
            "z.B.", "Dr.", "3.", "J.", "bzw.", "Wort", "Wort,", "d.h.", "U.S.", "ca.", "",
        ] {
            assert!(!sentence_end(no), "{no}");
        }
    }

    #[test]
    fn a_pause_after_a_sentence_end_starts_a_paragraph() {
        // 10 s of speech, 1.6 s of pause, 10 s of speech.
        let mut words = sentence(0.0, 20);
        words.extend(sentence(11.5, 20));
        let out = stitch(&[words]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].0, 11.5);
    }

    #[test]
    fn a_short_pause_or_a_short_paragraph_does_not() {
        // A pause of 0.6 s is no break; 1.5 s after a 4 s paragraph neither.
        let mut a = sentence(0.0, 20);
        a.extend(sentence(10.6, 20));
        assert_eq!(stitch(&[a]).len(), 1);
        let mut b = sentence(0.0, 8);
        b.extend(sentence(5.5, 8));
        assert_eq!(stitch(&[b]).len(), 1);
    }

    #[test]
    fn a_pause_without_a_sentence_end_does_not() {
        let mut words = sentence(0.0, 20);
        words[19].text = "Ende,".into();
        words.extend(sentence(12.0, 20));
        assert_eq!(stitch(&[words]).len(), 1);
    }

    #[test]
    fn a_long_pause_starts_a_paragraph_even_after_a_short_one() {
        let mut words = sentence(0.0, 4);
        words.extend(sentence(5.0, 4));
        assert_eq!(stitch(&[words]).len(), 2);
    }

    #[test]
    fn a_long_paragraph_ends_at_the_next_sentence_end() {
        // Sentences of 5 s with a short pause: the paragraph ends at the first
        // sentence end after 25 s.
        let mut words = Vec::new();
        for i in 0..8 {
            words.extend(sentence(i as f32 * 5.5, 10));
        }
        let out = stitch(&[words]);
        assert_eq!(out.len(), 2, "{out:?}");
        // The fifth sentence ends at 26.9 s, the first after 25 s.
        assert_eq!(out[1].0, 27.5);
    }

    #[test]
    fn a_stop_at_the_cut_of_a_piece_needs_a_real_pause() {
        // The model closes its first piece with a full stop in the middle
        // of a sentence; the next piece goes on at once.
        let first = sentence(0.0, 50);
        let second = sentence(25.2, 10);
        assert_eq!(stitch(&[first.clone(), second]).len(), 1);
        // The same with a real pause after it.
        let second = sentence(27.0, 10);
        assert_eq!(stitch(&[first, second]).len(), 2);
    }

    #[test]
    fn a_stop_at_the_cut_that_the_speaker_did_not_make_goes() {
        let first = said(&[("hat", 24.0, 24.3), ("gemeldet.", 24.3, 26.0)]);
        let second = said(&[("Und", 26.1, 26.3), ("Heizung", 26.3, 26.8), ("kaputt.", 26.8, 27.4)]);
        let out = stitch(&[first.clone(), second]);
        assert_eq!(out[0].1, "hat gemeldet und Heizung kaputt.");
        // A pause between them: the stop and the capital stay.
        let second = said(&[("Und", 26.6, 26.8), ("Heizung", 26.8, 27.3), ("kaputt.", 27.3, 27.9)]);
        let out = stitch(&[first, second]);
        assert_eq!(out[0].1, "hat gemeldet. Und Heizung kaputt.");
    }

    #[test]
    fn a_cut_without_a_stop_is_mended_as_far_as_the_pause_tells() {
        let first = said(&[("zum", 25.0, 25.3), ("Auto", 25.3, 25.8)]);
        // A new sentence after a short pause: the stop is put in.
        let out = stitch(&[first.clone(), said(&[("Der", 26.2, 26.4), ("Mechaniker", 26.4, 27.0)])]);
        assert_eq!(out[0].1, "zum Auto. Der Mechaniker");
        // No pause: it goes on.
        let out = stitch(&[first.clone(), said(&[("Der", 25.9, 26.1), ("Mechaniker", 26.1, 26.7)])]);
        assert_eq!(out[0].1, "zum Auto der Mechaniker");
        // A noun with a capital is left alone.
        let out = stitch(&[first, said(&[("Mechaniker", 26.2, 26.8)])]);
        assert_eq!(out[0].1, "zum Auto Mechaniker");
    }

    #[test]
    fn a_long_pause_at_the_end_of_a_piece_is_a_sentence_end_without_a_stop() {
        let first = said(&[("hat", 16.0, 16.3), ("gemeldet", 16.3, 21.0)]);
        let second = said(&[("und", 23.2, 23.4), ("Heizung", 23.4, 23.9)]);
        let out = stitch(&[first, second]);
        assert_eq!(out[0].1, "hat gemeldet.");
        assert_eq!(out[1].1, "Und Heizung");
    }

    #[test]
    fn the_shorter_the_pause_the_longer_the_paragraph_has_to_be() {
        // 5 s of speech, then 2.5 s and 1.2 s of pause.
        let mut words = sentence(0.0, 10);
        words.extend(sentence(7.4, 10));
        assert_eq!(stitch(&[words]).len(), 2);
        let mut words = sentence(0.0, 10);
        words.extend(sentence(6.1, 10));
        assert_eq!(stitch(&[words]).len(), 1);
    }

    #[test]
    fn without_any_sentence_end_it_is_cut_at_the_longest_pause() {
        // 90 s of words without a stop, and a pause of a second at 41 s.
        let mut words: Vec<Word> = (0..180)
            .map(|i| Word {
                text: "wort".into(),
                start: i as f32 * 0.5 + if i >= 82 { 1.0 } else { 0.0 },
                end: i as f32 * 0.5 + 0.4 + if i >= 82 { 1.0 } else { 0.0 },
            })
            .collect();
        words[179].text = "Ende.".into();
        let out = stitch(&[words]);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[1].0, 42.0);
    }

    #[test]
    fn a_run_of_fast_sentences_is_cut_at_a_sentence_end() {
        // Sentences of 3.5 s with 0.2 s between: no pause to speak of.
        let mut words = Vec::new();
        for i in 0..20 {
            words.extend(sentence(i as f32 * 3.6, 7));
        }
        let out = stitch(&[words]);
        assert!(out.len() >= 2, "{out:?}");
        assert!(out.iter().all(|(_, t)| t.ends_with("Ende.")), "{out:?}");
    }

    #[test]
    fn nothing_said_is_nothing() {
        assert!(stitch(&[Vec::new(), Vec::new()]).is_empty());
        assert!(stitch(&[said(&[("Hallo.", 1.0, 1.4)])]).len() == 1);
    }
}
