//! Following a recording while it runs, for the words on the screen.
//!
//! The models hear a whole recording at once; they cannot listen along.
//! So the newest stretch is recognised again and again, and [`Live`]
//! decides which of its words are sure enough to show. Every word comes
//! with its time in the recording, which keeps that simple:
//!
//! - A stretch starts a little before the last word shown, so the model
//!   knows what came before, and ends now. Words up to the last one shown
//!   are left out (they are on the screen already), so the stretch stays
//!   short however long the recording gets.
//! - A word is sure once a second and a bit followed it (the model hears a
//!   word better once it knows what comes after), or once two looks in a
//!   row heard it alike and the speaker has moved on from it. Where the last
//!   look heard something else, it waits for two looks to agree (or for much
//!   more after it): a word the model is unsure of comes out different from
//!   look to look, and the first guess is often wrong.
//! - A slow computer looks at shorter stretches and takes a word as sure a
//!   little sooner: its looks come far apart, so by the time a word
//!   shows, much more was said after it anyway.
//!
//! - After a look that found nothing new, the next one starts shortly before
//!   where that one ended, early enough to catch speech that had begun but
//!   was not recognised yet: a pause does not make the stretch grow, so the
//!   first words after it are not slowed down by listening to silence again.
//!
//! Shown words stay as they are, so the line on the screen only grows. What
//! gets typed is the whole recording, recognised once more at the end.

/// A recognised word and where it was heard, seconds from the start of the
/// audio given to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f32,
    pub end: f32,
}

/// Heard before the first new word, so the model knows what came before.
const BEFORE: f32 = 1.5;

/// The longest stretch looked at. A computer that cannot keep up skips
/// ahead rather than falling ever further behind.
const LONGEST: f32 = 8.0;

/// A word with this much of the recording after it is sure.
const AFTER: f32 = 1.2;

/// A word two looks agree on is sure with this much after it.
const AFTER_AGREED: f32 = 0.3;

/// A word the last look heard differently is sure with this much after it,
/// unless two looks agree on it before.
const AFTER_DISPUTED: f32 = 2.5;

/// On a slow computer (see [`Live::set_slow`]): heard before the first new
/// word, and the time after a word that makes it sure.
const BEFORE_SLOW: f32 = 0.8;
const AFTER_SLOW: f32 = 1.0;

/// Two looks agree on a word heard this close in time.
const SAME_PLACE: f32 = 0.3;

/// A word that starts this close to the start of a stretch may be cut.
const EDGE: f32 = 0.3;

/// After a look that found nothing new, the next one starts this long
/// before the end of that one (enough for a word that had just begun).
const QUIET_BEFORE: f32 = 1.5;

/// The words shown so far in a running recording.
#[derive(Default)]
pub struct Live {
    shown: Vec<String>,
    /// Where the last shown word starts and ends in the recording.
    last: (f32, f32),
    /// The words of the last look that are not shown yet.
    unsure: Vec<Word>,
    /// Where the last look that found nothing new ended.
    quiet: f32,
    /// The looks take long on this computer.
    slow: bool,
}

impl Live {
    /// The words to show, oldest first.
    pub fn shown(&self) -> &[String] {
        &self.shown
    }

    /// Whether the looks take long on this computer (the last one did).
    pub fn set_slow(&mut self, slow: bool) {
        self.slow = slow;
    }

    /// Where to start the next look, seconds into a recording `length`
    /// seconds long so far.
    pub fn from(&self, length: f32) -> f32 {
        let before = if self.slow { BEFORE_SLOW } else { BEFORE };
        (self.last.1 - before)
            .max(self.quiet - QUIET_BEFORE)
            .max(length - LONGEST)
            .max(0.0)
    }

    /// Takes the words heard in the recording from `from` to `to` (seconds;
    /// the words' times count from `from`). True when words were added.
    pub fn heard(&mut self, from: f32, to: f32, words: Vec<Word>) -> bool {
        let (last_start, done) = self.last;
        let last_key = self.shown.last().map(|w| key(w));
        let new: Vec<Word> = words
            .into_iter()
            .map(|w| Word {
                start: w.start + from,
                end: w.end + from,
                text: w.text,
            })
            // Cut at the start of the stretch, or shown already.
            .filter(|w| from == 0.0 || w.start >= from + EDGE)
            .filter(|w| (w.start + w.end) / 2.0 > done)
            .skip_while(|w| last_key.as_deref() == Some(&key(&w.text)) && (w.start - last_start).abs() <= SAME_PLACE)
            .collect();
        let after = if self.slow { AFTER_SLOW } else { AFTER };
        let mut added = 0;
        for word in &new {
            let here = |u: &&Word| u.start < word.end && word.start < u.end;
            let agreed = self
                .unsure
                .iter()
                .any(|u| key(&u.text) == key(&word.text) && (u.start - word.start).abs() <= SAME_PLACE);
            let disputed = !agreed && self.unsure.iter().filter(here).any(|u| key(&u.text) != key(&word.text));
            let sure = if agreed {
                word.end <= to - AFTER_AGREED
            } else if disputed {
                word.end <= to - AFTER_DISPUTED
            } else {
                word.end <= to - after
            };
            if !sure {
                break;
            }
            self.shown.push(word.text.clone());
            self.last = (word.start, word.end);
            added += 1;
        }
        self.unsure = new[added..].to_vec();
        if new.is_empty() {
            self.quiet = to;
        }
        added > 0
    }
}

/// A word apart from case and punctuation.
fn key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words at a steady pace, one every 0.4 s from `at`.
    fn timed(text: &str, at: f32) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| Word {
                text: w.to_string(),
                start: at + 0.4 * i as f32,
                end: at + 0.4 * i as f32 + 0.3,
            })
            .collect()
    }

    fn shown(live: &Live) -> String {
        live.shown().join(" ")
    }

    #[test]
    fn words_with_a_second_and_a_bit_after_them_show() {
        let mut live = Live::default();
        // "haben" ends at 0.7 s, "lange" at 1.1 s.
        assert!(live.heard(0.0, 2.2, timed("Wir haben lange über die Ar", 0.0)));
        assert_eq!(shown(&live), "Wir haben");
    }

    #[test]
    fn the_last_words_show_once_two_looks_agree() {
        let mut live = Live::default();
        live.heard(0.0, 2.2, timed("Wir haben lange über", 0.0));
        assert_eq!(shown(&live), "Wir haben");
        // "über" (to 1.5 s) heard alike, and the speaker is past it.
        live.heard(0.0, 2.5, timed("Wir haben lange über die", 0.0));
        assert_eq!(shown(&live), "Wir haben lange über");
    }

    #[test]
    fn a_word_still_being_said_does_not_show() {
        let mut live = Live::default();
        live.heard(0.0, 1.0, timed("Wir haben Unter", 0.0));
        assert_eq!(shown(&live), "");
        // "Unter" (to 1.1 s) heard alike, but the speaker may still be in it.
        live.heard(0.0, 1.35, timed("Wir haben Unter", 0.0));
        assert_eq!(shown(&live), "Wir haben");
    }

    #[test]
    fn a_changed_mind_waits_for_more() {
        let mut live = Live::default();
        live.heard(0.0, 1.6, timed("Hallo Hanna wie", 0.0));
        assert_eq!(shown(&live), "Hallo");
        live.heard(0.0, 1.65, timed("Hallo Anna, wie", 0.0));
        assert_eq!(shown(&live), "Hallo");
        live.heard(0.0, 1.95, timed("Hallo Anna, wie geht", 0.0));
        assert_eq!(shown(&live), "Hallo Anna, wie");
    }

    #[test]
    fn a_word_heard_differently_waits_for_two_looks_to_agree() {
        let mut live = Live::default();
        live.heard(0.0, 1.2, timed("Wir haben Kalk", 0.0));
        assert_eq!(shown(&live), "");
        // "Kalb" (to 1.1 s) has a second and a bit after it, but the last
        // look heard "Kalk" there.
        live.heard(0.0, 2.5, timed("Wir haben Kalb fleisch", 0.0));
        assert_eq!(shown(&live), "Wir haben");
        // Heard alike once more: sure.
        live.heard(0.0, 2.9, timed("Wir haben Kalb fleisch aus", 0.0));
        assert_eq!(shown(&live), "Wir haben Kalb fleisch");
    }

    #[test]
    fn a_slow_computer_looks_at_shorter_stretches() {
        let mut live = Live::default();
        live.set_slow(true);
        // "haben" (to 0.7 s) has a second after it.
        assert!(live.heard(0.0, 1.75, timed("Wir haben lange", 0.0)));
        assert_eq!(shown(&live), "Wir haben");
        live.heard(0.0, 4.0, timed("Wir haben lange über die Arbeit", 0.0));
        assert_eq!(shown(&live), "Wir haben lange über die Arbeit");
        // "Arbeit" ends at 2.3 s: the next look starts 0.8 s before.
        assert!((live.from(4.5) - 1.5).abs() < 1e-5);
    }

    #[test]
    fn the_next_look_starts_a_little_before_the_last_word_shown() {
        let mut live = Live::default();
        assert_eq!(live.from(5.0), 0.0);
        live.heard(
            0.0,
            4.2,
            timed("eins zwei drei vier fünf sechs sieben acht neun zehn", 0.0),
        );
        // "sieben" (2.4 .. 2.7 s) is the last one with at least 1.2 s after it.
        assert_eq!(shown(&live), "eins zwei drei vier fünf sechs sieben");
        assert!((live.from(4.9) - 1.2).abs() < 1e-5);
        // A long recording: never more than the longest stretch.
        assert_eq!(live.from(20.0), 12.0);
    }

    #[test]
    fn words_shown_already_are_left_out() {
        let mut live = Live::default();
        live.heard(0.0, 2.6, timed("eins zwei drei vier fünf", 0.0));
        assert_eq!(shown(&live), "eins zwei drei");
        // Heard again a little later and longer this time: still shown.
        let mut words = timed("eins zwei drei vier fünf sechs sieben", 0.0);
        words[3].start = 1.3;
        words[3].end = 1.8;
        live.heard(0.0, 3.4, words);
        assert_eq!(shown(&live), "eins zwei drei vier fünf");
        // Nothing new: nothing added.
        assert!(!live.heard(1.0, 3.7, Vec::new()));
    }

    #[test]
    fn a_pause_does_not_make_the_stretch_grow() {
        let mut live = Live::default();
        live.heard(0.0, 2.9, timed("eins zwei drei vier", 0.0));
        // Nothing but the words shown already: the speaker is quiet.
        let from = live.from(4.0);
        live.heard(from, 4.0, Vec::new());
        assert!((live.from(4.0) - 2.5).abs() < 1e-5);
        // Silence goes on: the stretch is as short as the time since.
        assert!((live.from(9.0) - 2.5).abs() < 1e-5);
        // Speech again: the words are new and sure after a moment.
        live.heard(2.5, 9.0, timed("sechs sieben", 1.5));
        assert_eq!(shown(&live), "eins zwei drei vier sechs sieben");
    }

    #[test]
    fn a_stretch_that_skipped_ahead_drops_its_cut_first_word() {
        let mut live = Live::default();
        let from = live.from(12.0);
        assert_eq!(from, 4.0);
        live.heard(from, 12.0, timed("gen über die Arbeit gesprochen kannst", 0.1));
        assert_eq!(shown(&live), "über die Arbeit gesprochen kannst");
    }
}
