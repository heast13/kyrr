//! A memo's title: the first words said in it, as fits a file name.

/// Words that start many a memo without saying anything.
const FILLERS: [&str; 16] = [
    "äh", "ähm", "öhm", "hm", "hmm", "also", "ja", "ok", "okay", "so", "na", "gut", "um", "uh", "uhm", "well",
];

/// The most words a title takes, and the most characters.
const WORDS: usize = 6;
const CHARS: usize = 60;
/// The longest title a user gives, in bytes.
const BYTES: usize = 150;

/// The title for a memo whose start was heard as `text`; None when nothing
/// fit for one was said.
pub fn from_text(text: &str) -> Option<String> {
    // The first sentence with more than fillers in it; a line break or a
    // colon ends one as well.
    text.split(['.', '!', '?', '\n', ':', ';']).find_map(sentence)
}

fn sentence(text: &str) -> Option<String> {
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, ',' | '"' | '„' | '“' | '”')))
        .filter(|w| !w.is_empty())
        .skip_while(|w| FILLERS.contains(&w.to_lowercase().as_str()))
        .take(WORDS)
        .collect();
    let mut title = String::new();
    for word in words {
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
    let title = clean(&title);
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
    fn the_first_words_say_what_it_is() {
        assert_eq!(
            from_text("Äh, also Einkaufsliste für Samstag: Milch, Eier und Kaffee. Und Kohle.").as_deref(),
            Some("Einkaufsliste für Samstag")
        );
        assert_eq!(
            from_text("ok so meeting notes for Monday").as_deref(),
            Some("Meeting notes for Monday")
        );
        assert_eq!(from_text("Anruf beim Arzt?").as_deref(), Some("Anruf beim Arzt"));
        assert_eq!(from_text("").as_deref(), None);
        assert_eq!(from_text("Äh. Ja.").as_deref(), None);
        assert_eq!(
            from_text("Ähm. Ja, also Zahnarzt anrufen.").as_deref(),
            Some("Zahnarzt anrufen")
        );
        assert_eq!(from_text("ähm").as_deref(), None);
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
