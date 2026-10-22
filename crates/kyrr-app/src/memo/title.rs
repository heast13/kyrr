//! A memo's title: what it is about, as fits a file name. A short opening
//! sentence says that itself; otherwise the words that carry the most
//! meaning in the text do. No language model: word lists and counting.

use std::collections::HashMap;

/// Words that start many a memo without saying anything.
const FILLERS: [&str; 16] = [
    "äh", "ähm", "öhm", "hm", "hmm", "also", "ja", "ok", "okay", "so", "na", "gut", "um", "uh", "uhm", "well",
];

/// What starts talk about the speaker or the recording rather than a
/// subject: such a sentence does not make a title.
const CHATTER: [&str; 22] = [
    "ich", "i", "i'm", "i've", "wir", "we", "man", "es", "hallo", "hello", "hi", "hey", "mal", "kurz", "just",
    "remember", "denk", "vergiss", "don't", "this", "das", "dies",
];

/// Words that say little in German and English, in any case and place.
const COMMON: &str = "aber alle allem allen aller alles als also andere anderen auch auf aus bei bin bis bist da damit \
dann das dass dazu dein deine dem den denn der des die dies diese diesem diesen dieser dieses doch dort durch ein eine \
einem einen einer eines einfach eigentlich etwa euch für gar gegen genau gerade gestern gut habe haben hat hatte hier \
heute hin hinter ihm ihn ihr ihre immer ist ja jetzt kann kein keine mehr mein meine mich mir mit muss müssen nach nicht \
noch nun nur oder ohne schon sehr sein seine sich sind soll sollte sondern über unbedingt und uns unser unter vielleicht \
vom von vor war waren was weil wenn werden wie wieder wieso wir wird wirklich wollen wollte würde zum zur zwar zwei \
about after again all also and any are because been before being both but can could did does doing done each for from \
get gets going gonna good got had has have having here how into its just know let like make maybe more most much need \
needs not now okay once only other our out over really remember said should some still such than that their them then \
there these they thing things think this those through today tomorrow too very want wants was were what when where \
which while who will with would yeah yesterday your";

/// The most words a title takes, and the most characters.
const WORDS: usize = 6;
const CHARS: usize = 60;
/// The most keywords a title is made of.
const KEYWORDS: usize = 3;
/// The longest title a user gives, in bytes.
const BYTES: usize = 150;

/// The title for a memo of which `text` was heard (all of it, or its
/// start); None when nothing fit for one was said.
pub fn from_text(text: &str) -> Option<String> {
    // The first sentence with more than fillers in it; a line break or a
    // colon ends one as well.
    let first = text
        .split(['.', '!', '?', '\n', ':', ';'])
        .map(words)
        .find(|w| !w.is_empty());
    let first = first?;
    if first.len() <= WORDS && !chatter(&first) && carries_meaning(&first) {
        return finish(&first);
    }
    keywords(text).or_else(|| finish(&first[..first.len().min(WORDS)]))
}

/// The words of `sentence` without the fillers at its start and the
/// quotes and commas around them.
fn words(sentence: &str) -> Vec<&str> {
    sentence
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, ',' | '"' | '„' | '“' | '”')))
        .filter(|w| !w.is_empty())
        .skip_while(|w| FILLERS.contains(&w.to_lowercase().as_str()))
        .collect()
}

fn chatter(words: &[&str]) -> bool {
    CHATTER.contains(&words[0].to_lowercase().as_str())
}

/// Half of the words, at least, are ones that say something.
fn carries_meaning(words: &[&str]) -> bool {
    let said = words.iter().filter(|w| is_content(&w.to_lowercase())).count();
    said * 2 >= words.len()
}

fn is_content(word: &str) -> bool {
    word.chars().count() >= 4
        && !COMMON.split_whitespace().any(|c| c == word)
        && !word.chars().all(|c| c.is_ascii_digit())
}

/// The words of `text` that say most about it: German nouns (a capital
/// inside a sentence), names, words said again, long words; up to
/// [`KEYWORDS`], in the order they came.
fn keywords(text: &str) -> Option<String> {
    struct Seen {
        word: String,
        first: usize,
        count: usize,
        capital: bool,
    }
    let mut seen: HashMap<String, Seen> = HashMap::new();
    let mut at = 0;
    for sentence in text.split(['.', '!', '?', '\n']) {
        for (i, raw) in sentence
            .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
            .enumerate()
        {
            let raw = raw.trim_matches(['-', '\'']);
            let lower = raw.to_lowercase();
            if !is_content(&lower) || FILLERS.contains(&lower.as_str()) {
                continue;
            }
            let capital = i > 0 && raw.chars().next().is_some_and(char::is_uppercase);
            let entry = seen.entry(lower).or_insert_with(|| Seen {
                word: raw.to_string(),
                first: at,
                count: 0,
                capital: false,
            });
            entry.count += 1;
            // The form with a capital, where it was said once.
            if capital && !entry.capital {
                entry.capital = true;
                entry.word = raw.to_string();
            }
            at += 1;
        }
    }
    let score = |s: &Seen| {
        let long = s.word.chars().count();
        (if s.capital { 2.0 } else { 0.0 })
            + 1.5 * (s.count - 1) as f64
            + 0.1 * long.min(12) as f64
            + if long >= 6 { 0.5 } else { 0.0 }
            - 0.002 * s.first as f64
    };
    // A short plain word said once is chatter more often than not.
    let mut ranked: Vec<&Seen> = seen
        .values()
        .filter(|s| s.capital || s.count > 1 || s.word.chars().count() >= 6)
        .collect();
    ranked.sort_by(|a, b| score(b).total_cmp(&score(a)).then(a.first.cmp(&b.first)));
    let mut chosen: Vec<&Seen> = Vec::new();
    for s in ranked {
        // "Termin" and "Termine" are one word.
        let (a, b) = (s.word.to_lowercase(), |o: &&Seen| o.word.to_lowercase());
        if chosen.iter().any(|o| {
            let b = b(o);
            (a.starts_with(&b) || b.starts_with(&a)) && a.len().abs_diff(b.len()) <= 2
        }) {
            continue;
        }
        chosen.push(s);
        if chosen.len() == KEYWORDS {
            break;
        }
    }
    chosen.sort_by_key(|s| s.first);
    let mut title = String::new();
    for s in chosen {
        let next = if title.is_empty() {
            s.word.clone()
        } else {
            format!("{title}, {}", s.word)
        };
        if next.chars().count() > CHARS {
            break;
        }
        title = next;
    }
    capitalized(&title)
}

/// `words` as a title, cut to [`WORDS`] and [`CHARS`].
fn finish(words: &[&str]) -> Option<String> {
    let mut title = String::new();
    for word in words.iter().take(WORDS) {
        let next = if title.is_empty() {
            word.to_string()
        } else {
            format!("{title} {word}")
        };
        if next.chars().count() > CHARS {
            break;
        }
        title = next;
    }
    capitalized(&title)
}

fn capitalized(title: &str) -> Option<String> {
    let title = clean(title);
    let mut chars = title.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// `title` with what a file name cannot hold (on Windows or Linux) taken out:
/// no slashes, colons, quotes or control characters, no dot or space at the
/// end. Empty when nothing is left.
pub fn clean(title: &str) -> String {
    let kept: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let mut title = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    // A file name holds 255 bytes on Linux; the date and ".opus" come along.
    let mut end = title.len().min(BYTES);
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    title.truncate(end);
    while title.ends_with(['.', ' ']) {
        title.pop();
    }
    let title = title.trim_start_matches('.').trim_start().to_string();
    // Names Windows keeps for devices.
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "lpt1", "lpt2", "lpt3",
    ];
    if reserved.contains(&title.to_lowercase().as_str()) {
        return format!("{title} 1");
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_opening_says_what_it_is() {
        assert_eq!(
            from_text("Äh, also Einkaufsliste für Samstag: Milch, Eier und Kaffee. Und Kohle.").as_deref(),
            Some("Einkaufsliste für Samstag")
        );
        assert_eq!(
            from_text("Notizen vom Meeting mit dem Marketing. Wir haben über das Budget gesprochen.").as_deref(),
            Some("Notizen vom Meeting mit dem Marketing")
        );
        assert_eq!(from_text("Anruf beim Arzt?").as_deref(), Some("Anruf beim Arzt"));
        assert_eq!(
            from_text("Ähm. Ja, also Zahnarzt anrufen.").as_deref(),
            Some("Zahnarzt anrufen")
        );
    }

    #[test]
    fn a_long_or_chatty_opening_gives_keywords() {
        assert_eq!(
            from_text(
                "Äh, also ich muss morgen unbedingt beim Zahnarzt anrufen wegen dem Termin. Der hat letzte Woche \
                 abgesagt und ich brauche dringend einen neuen Termin vor dem Urlaub. Und dann muss ich noch die \
                 Rechnung von der Zahnreinigung bei der Versicherung einreichen."
            )
            .as_deref(),
            Some("Termin, Zahnreinigung, Versicherung")
        );
        assert_eq!(
            from_text(
                "So, notes from the call with the client. They want the new Dashboard shipped by the end of \
                 March, and the budget for the Dashboard was increased. I need to send the contract to Sarah."
            )
            .as_deref(),
            Some("Dashboard, March, Sarah")
        );
    }

    #[test]
    fn nothing_said_is_no_title() {
        assert_eq!(from_text("").as_deref(), None);
        assert_eq!(from_text("Äh. Ja.").as_deref(), None);
        assert_eq!(from_text("ähm").as_deref(), None);
        // Only small words: the opening stays, cut short.
        assert_eq!(
            from_text("Ich weiß nicht, ob das so geht").as_deref(),
            Some("Ich weiß nicht ob das so")
        );
        let long = from_text("Donaudampfschifffahrtsgesellschaftskapitänsmütze und Rinderkennzeichnungsfleischetikettierungsüberwachungsaufgabenübertragungsgesetz").unwrap();
        assert!(long.chars().count() <= CHARS, "{long}");
    }

    #[test]
    fn titles_fit_a_file_name() {
        assert_eq!(clean("a/b\\c: d*e?f\"g<h>i|j"), "a b c d e f g h i j");
        assert_eq!(clean("  Ende...  "), "Ende");
        let long = clean(&"Überweisung ".repeat(30));
        assert!(long.len() <= BYTES && !long.ends_with(' '), "{long}");
        assert_eq!(clean("..versteckt"), "versteckt");
        assert_eq!(clean("CON"), "CON 1");
        assert_eq!(clean("Zeile\neins\tzwei"), "Zeile eins zwei");
        assert_eq!(clean(" . "), "");
    }
}
