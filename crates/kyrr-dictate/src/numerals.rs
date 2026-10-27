//! Numbers as digits: the speech models write what they hear,
//! "sechsundzwanzig", "twenty six". kyrr writes them the way they are
//! written, by fixed rules, so no value can come out wrong:
//!
//! - from 13 up always digits, 0 to 12 stay words ("zwei Kinder"), unless a
//!   unit follows ("2 Euro", "8 Uhr", "3 Prozent");
//! - times ("14:30 Uhr"), dates ("26. Mai", "3.10.2026"), money ("349,50
//!   Euro"), decimals ("3,5") and digit by digit ("0699 …") as digits;
//! - articles ("ein Haus", "eine Million"), "halb acht", "tausend Dank"
//!   stay as they are, and so does anything that does not read as one
//!   number ("zwanzig zwei").

use crate::Language;

/// One word and what follows it up to the next word.
#[derive(Clone, Debug)]
struct Token {
    word: String,
    after: String,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric()
}

fn tokens(text: &str) -> (String, Vec<Token>) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut lead = String::new();
    while i < chars.len() && !word_char(chars[i]) {
        lead.push(chars[i]);
        i += 1;
    }
    let mut out = Vec::new();
    while i < chars.len() {
        let mut word = String::new();
        loop {
            while i < chars.len() && word_char(chars[i]) {
                word.push(chars[i]);
                i += 1;
            }
            // "twenty-six", "o'clock"
            let inner = i + 1 < chars.len() && matches!(chars[i], '-' | '\'' | '’') && word_char(chars[i + 1]);
            if !inner {
                break;
            }
            word.push(chars[i]);
            i += 1;
        }
        let mut after = String::new();
        while i < chars.len() && !word_char(chars[i]) {
            after.push(chars[i]);
            i += 1;
        }
        out.push(Token { word, after });
    }
    (lead, out)
}

/// Lower case, ß as ss, ’ as '.
fn fold(w: &str) -> String {
    w.to_lowercase().replace('ß', "ss").replace('’', "'")
}

// ---- German ---------------------------------------------------------------

const DE_UNITS: [(&str, u64); 12] = [
    ("eins", 1),
    ("ein", 1),
    ("zwei", 2),
    ("zwo", 2),
    ("drei", 3),
    ("vier", 4),
    ("fünf", 5),
    ("sechs", 6),
    ("sieben", 7),
    ("acht", 8),
    ("neun", 9),
    ("null", 0),
];

const DE_TEENS: [(&str, u64); 10] = [
    ("zehn", 10),
    ("elf", 11),
    ("zwölf", 12),
    ("dreizehn", 13),
    ("vierzehn", 14),
    ("fünfzehn", 15),
    ("sechzehn", 16),
    ("siebzehn", 17),
    ("achtzehn", 18),
    ("neunzehn", 19),
];

const DE_TENS: [(&str, u64); 8] = [
    ("zwanzig", 20),
    ("dreissig", 30),
    ("vierzig", 40),
    ("fünfzig", 50),
    ("sechzig", 60),
    ("siebzig", 70),
    ("achtzig", 80),
    ("neunzig", 90),
];

fn find(list: &[(&str, u64)], w: &str) -> Option<u64> {
    list.iter().find(|(name, _)| *name == w).map(|(_, v)| *v)
}

fn de_below_100(w: &str) -> Option<u64> {
    if w == "ein" {
        return None;
    }
    if let Some(v) = find(&DE_UNITS, w)
        .or_else(|| find(&DE_TEENS, w))
        .or_else(|| find(&DE_TENS, w))
    {
        return Some(v);
    }
    let (unit, tens) = w.split_once("und")?;
    let unit = match unit {
        "eins" | "null" | "zwo" => return None,
        u => find(&DE_UNITS, u).filter(|&v| v > 0)?,
    };
    Some(unit + find(&DE_TENS, tens)?)
}

fn de_below_1000(w: &str) -> Option<u64> {
    let Some((front, rest)) = w.split_once("hundert") else {
        return de_below_100(w);
    };
    let hundreds = match front {
        "" | "ein" => 1,
        // "neunzehnhundert…": a year.
        f => de_below_100(f).filter(|&v| v >= 2)?,
    };
    let rest = if rest.is_empty() { 0 } else { de_below_100(rest)? };
    Some(hundreds * 100 + rest)
}

/// "dreihundertneunundvierzig", "zweitausendsechsundzwanzig".
fn de_cardinal(w: &str) -> Option<u64> {
    let Some((front, rest)) = w.split_once("tausend") else {
        return de_below_1000(w);
    };
    let thousands = match front {
        "" | "ein" => 1,
        f => de_below_1000(f).filter(|&v| v >= 2)?,
    };
    let rest = if rest.is_empty() { 0 } else { de_below_1000(rest)? };
    Some(thousands * 1000 + rest)
}

/// "sechsundzwanzigsten", "dritte", "ersten".
fn de_ordinal(w: &str) -> Option<u64> {
    let stem = ["en", "er", "es", "em", "e"]
        .iter()
        .find_map(|end| w.strip_suffix(end))
        .unwrap_or(w);
    if let Some(front) = stem.strip_suffix("st") {
        if let Some(v) = de_cardinal(front).filter(|&v| v >= 20) {
            return Some(v);
        }
    }
    for (end, cardinal) in [
        ("erst", "eins"),
        ("dritt", "drei"),
        ("siebt", "sieben"),
        ("acht", "acht"),
    ] {
        if let Some(front) = stem.strip_suffix(end) {
            let v = if front.is_empty() {
                de_cardinal(cardinal)
            } else {
                de_cardinal(&format!("{front}{cardinal}"))
            };
            if v.is_some() {
                return v;
            }
        }
    }
    let front = stem.strip_suffix('t')?;
    if front.ends_with("eins") || front.ends_with("drei") || front.ends_with("sieben") {
        return None;
    }
    de_cardinal(front).filter(|&v| v != 1 && v % 100 < 20)
}

const DE_MONTHS: [&str; 13] = [
    "januar",
    "jänner",
    "februar",
    "märz",
    "april",
    "mai",
    "juni",
    "juli",
    "august",
    "september",
    "oktober",
    "november",
    "dezember",
];

const DE_UNIT_WORDS: [&str; 30] = [
    "euro",
    "cent",
    "dollar",
    "franken",
    "rappen",
    "pfund",
    "prozent",
    "promille",
    "grad",
    "uhr",
    "meter",
    "kilometer",
    "zentimeter",
    "millimeter",
    "kilogramm",
    "kilo",
    "gramm",
    "tonnen",
    "liter",
    "milliliter",
    "watt",
    "kilowatt",
    "volt",
    "kilobyte",
    "megabyte",
    "gigabyte",
    "terabyte",
    "millionen",
    "milliarden",
    "km",
];

const DE_MONEY: [&str; 4] = ["euro", "dollar", "franken", "pfund"];

// ---- English --------------------------------------------------------------

const EN_UNITS: [(&str, u64); 10] = [
    ("zero", 0),
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
];

const EN_TEENS: [(&str, u64); 10] = [
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
];

const EN_TENS: [(&str, u64); 8] = [
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];

const EN_ORDINALS: [(&str, u64); 20] = [
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("fifth", 5),
    ("sixth", 6),
    ("seventh", 7),
    ("eighth", 8),
    ("ninth", 9),
    ("tenth", 10),
    ("eleventh", 11),
    ("twelfth", 12),
    ("thirteenth", 13),
    ("fourteenth", 14),
    ("fifteenth", 15),
    ("sixteenth", 16),
    ("seventeenth", 17),
    ("eighteenth", 18),
    ("nineteenth", 19),
    ("hundredth", 100),
];

/// One English word below 100: "six", "twenty", "twenty-six", "sixth",
/// "twenty-sixth"; and whether it was an ordinal.
fn en_word(w: &str) -> Option<(u64, bool)> {
    if let Some(v) = find(&EN_UNITS, w)
        .or_else(|| find(&EN_TEENS, w))
        .or_else(|| find(&EN_TENS, w))
    {
        return Some((v, false));
    }
    if let Some(v) = find(&EN_ORDINALS, w) {
        return Some((v, true));
    }
    if let Some(tens) = w.strip_suffix("ieth") {
        return find(&EN_TENS, &format!("{tens}y")).map(|v| (v, true));
    }
    let (tens, unit) = w.split_once('-')?;
    let tens = find(&EN_TENS, tens)?;
    if let Some(u) = find(&EN_UNITS, unit).filter(|&u| u > 0) {
        return Some((tens + u, false));
    }
    find(&EN_ORDINALS, unit).filter(|&u| u < 10).map(|u| (tens + u, true))
}

/// The English number that starts at `at`, as long as it goes: "three
/// hundred and forty nine", "twenty six"; its value, how many words, and
/// whether it ends as an ordinal.
fn en_cardinal(words: &[String], gaps: &[bool], at: usize) -> Option<(u64, usize, bool)> {
    let word = |i: usize| words.get(i).map(String::as_str);
    let joined = |i: usize| i > at && gaps.get(i - 1).copied().unwrap_or(false);
    let mut i = at;
    let mut total = 0;
    let mut group: u64 = 0;
    let mut ordinal = false;
    let mut any = false;
    // What the last word allows next.
    let mut last: Option<&str> = None;
    while let Some(w) = word(i) {
        if i > at && !joined(i) {
            break;
        }
        let step = match (w, last) {
            ("hundred" | "hundredth", Some("unit" | "teen")) if group > 0 && group < 100 => {
                group *= 100;
                ordinal = w == "hundredth";
                "hundred"
            }
            ("thousand", Some("unit" | "teen" | "tens" | "hundred" | "tens-unit")) if group > 0 => {
                total += group * 1000;
                group = 0;
                "thousand"
            }
            ("and", Some("hundred" | "thousand")) => "and",
            _ => match en_word(w) {
                Some((v, ord)) => {
                    let kind = if v < 10 {
                        "unit"
                    } else if v < 20 || v == 100 {
                        "teen"
                    } else if v.is_multiple_of(10) {
                        "tens"
                    } else {
                        "tens-unit"
                    };
                    let fits = match last {
                        None | Some("hundred" | "thousand" | "and") => group.is_multiple_of(100),
                        // "twenty six"
                        Some("tens") => kind == "unit" && v > 0,
                        _ => false,
                    };
                    if !fits || v == 100 && last.is_some() {
                        break;
                    }
                    group += v;
                    ordinal = ord;
                    any = true;
                    kind
                }
                None => break,
            },
        };
        last = Some(step);
        i += 1;
        if ordinal {
            break;
        }
    }
    // "and" at the end is no part of it.
    while i > at && matches!(word(i - 1), Some("and")) {
        i -= 1;
    }
    (any && i > at).then_some((total + group, i - at, ordinal))
}

const EN_MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const EN_UNIT_WORDS: [&str; 30] = [
    "percent",
    "dollar",
    "dollars",
    "euro",
    "euros",
    "cent",
    "cents",
    "pound",
    "pounds",
    "degrees",
    "o'clock",
    "miles",
    "mile",
    "kilometers",
    "kilometres",
    "meters",
    "metres",
    "feet",
    "inches",
    "kilograms",
    "kilos",
    "grams",
    "liters",
    "litres",
    "gallons",
    "million",
    "billion",
    "gigabytes",
    "megabytes",
    "km",
];

const EN_MONEY: [&str; 6] = ["dollar", "dollars", "euro", "euros", "pound", "pounds"];

// ---- Writing --------------------------------------------------------------

fn grouped(v: u64, sep: char) -> String {
    let s = v.to_string();
    if v < 10_000 {
        return s;
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

fn en_ordinal_suffix(v: u64) -> &'static str {
    match (v % 10, v % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// What a word is as a number, in one language.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Num {
    Cardinal(u64),
    Ordinal(u64),
}

struct Text {
    words: Vec<String>,
    /// As said, with their capitals ("May" the month, "may" the verb).
    raw: Vec<String>,
    /// Whether word `i` and word `i + 1` are apart by a single space only.
    gaps: Vec<bool>,
}

impl Text {
    fn word(&self, i: usize) -> Option<&str> {
        self.words.get(i).map(String::as_str)
    }

    fn joined(&self, i: usize) -> bool {
        self.gaps.get(i).copied().unwrap_or(false)
    }

    fn de(&self, i: usize) -> Option<Num> {
        let w = self.word(i)?;
        if let Ok(v) = w.parse::<u64>() {
            return Some(Num::Cardinal(v));
        }
        de_cardinal(w)
            .map(Num::Cardinal)
            .or_else(|| de_ordinal(w).map(Num::Ordinal))
    }

    fn de_cardinal(&self, i: usize) -> Option<u64> {
        match self.de(i)? {
            Num::Cardinal(v) => Some(v),
            Num::Ordinal(_) => None,
        }
    }

    /// One German digit, as said digit by digit.
    fn de_digit(&self, i: usize) -> Option<u64> {
        let w = self.word(i)?;
        find(&DE_UNITS, w).filter(|_| w != "ein")
    }

    fn en(&self, i: usize) -> Option<(u64, usize, bool)> {
        if let Some(v) = self.word(i).and_then(|w| w.parse::<u64>().ok()) {
            return Some((v, 1, false));
        }
        en_cardinal(&self.words, &self.gaps, i)
    }

    fn en_digit(&self, i: usize) -> Option<u64> {
        find(&EN_UNITS, self.word(i)?)
    }

    /// An English month's name; "may" only as "May".
    fn en_month(&self, i: usize) -> bool {
        self.word(i).is_some_and(|w| EN_MONTHS.contains(&w))
            && (self.word(i) != Some("may") || self.raw[i].starts_with('M'))
    }

    fn any_number(&self, i: usize) -> bool {
        self.de(i).is_some() || self.en(i).is_some()
    }
}

/// Numbers that are said digit by digit: four and more, or a 0 and two
/// more ("null eins sieben sechs", a phone number).
fn digit_run(text: &Text, at: usize, digit: impl Fn(usize) -> Option<u64>) -> Option<(String, usize)> {
    let mut n = 0;
    let mut out = String::new();
    while let Some(d) = digit(at + n) {
        out.push_str(&d.to_string());
        n += 1;
        if !text.joined(at + n - 1) {
            break;
        }
    }
    (n >= 4 || (n >= 3 && out.starts_with('0'))).then_some((out, n))
}

/// The German rules at word `i`: what to write and how many words it takes.
fn german(text: &Text, i: usize) -> Option<(String, usize)> {
    if let Some(found) = digit_run(text, i, |j| text.de_digit(j)) {
        return Some(found);
    }
    let num = text.de(i)?;
    let next = |k: usize| if text.joined(i + k - 1) { text.word(i + k) } else { None };
    match num {
        Num::Ordinal(day) => {
            // "dritten zehnten zweitausendsechsundzwanzig"
            if let Some(Num::Ordinal(month)) = next(1).and_then(|_| text.de(i + 1)) {
                if (1..=31).contains(&day) && (1..=12).contains(&month) {
                    if let Some(year) = next(2)
                        .and_then(|_| text.de_cardinal(i + 2))
                        .filter(|y| (1000..3000).contains(y))
                    {
                        return Some((format!("{day}.{month}.{year}"), 3));
                    }
                    // "am dritten zehnten.", but not "im dritten achten Stock"
                    if next(2).is_none() {
                        return Some((format!("{day}.{month}."), 2));
                    }
                }
            }
            let month = next(1).is_some_and(|w| DE_MONTHS.contains(&w));
            ((month && (1..=31).contains(&day)) || day >= 13).then(|| (format!("{day}."), 1))
        }
        Num::Cardinal(v) => {
            // "vierzehn Uhr dreißig", but not "fünf Uhr zehn Minuten"
            let counted = matches!(next(3), Some("minuten" | "minute" | "sekunden"));
            if next(1) == Some("uhr") && v <= 24 && !counted {
                if let Some(m) = next(2)
                    .and_then(|_| text.de_cardinal(i + 2))
                    .filter(|m| (1..60).contains(m))
                {
                    return Some((format!("{v}:{m:02} Uhr"), 3));
                }
            }
            // "dreihundertneunundvierzig Euro (und) fünfzig (Cent)"
            if next(1).is_some_and(|w| DE_MONEY.contains(&w)) {
                let and = usize::from(next(2) == Some("und"));
                if let Some(cents) = next(2 + and)
                    .and_then(|_| text.de_cardinal(i + 2 + and))
                    .filter(|c| (1..100).contains(c))
                {
                    let cent_word = matches!(next(3 + and), Some("cent" | "rappen"));
                    // "zwölf Euro fünfzig." is money, "zwei Euro und zwanzig
                    // Minuten" is not.
                    let ends = next(3 + and).is_none() && and == 0;
                    if cent_word || (cents >= 10 && ends) {
                        let money = text.words[i + 1].clone();
                        return Some((
                            format!("{},{cents:02} {}", grouped(v, '.'), capital(&money)),
                            3 + and + usize::from(cent_word),
                        ));
                    }
                }
            }
            // "drei Komma fünf"
            if next(1) == Some("komma") {
                if let Some((decimals, n)) = decimals(text, i + 2, |j| text.de_digit(j), |j| text.de_cardinal(j)) {
                    return Some((format!("{},{decimals}", grouped(v, '.')), 2 + n));
                }
            }
            let word = text.word(i)?;
            if word.parse::<u64>().is_ok() {
                return None;
            }
            // "zwanzig zwei" is no number: leave it be.
            if next(1).is_some() && text.de(i + 1).is_some() && !matches!(word, "ein" | "eine") {
                return None;
            }
            let unit = next(1).is_some_and(|w| DE_UNIT_WORDS.contains(&w));
            let bare = matches!(word, "hundert" | "tausend");
            (unit || (v >= 13 && !bare)).then(|| (grouped(v, '.'), 1))
        }
    }
}

/// Digits after the decimal sign: one by one, or one number below 100.
fn decimals(
    text: &Text,
    at: usize,
    digit: impl Fn(usize) -> Option<u64>,
    number: impl Fn(usize) -> Option<u64>,
) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut n = 0;
    while let Some(d) = digit(at + n) {
        out.push_str(&d.to_string());
        n += 1;
        if !text.joined(at + n - 1) {
            break;
        }
    }
    if n > 0 {
        return Some((out, n));
    }
    number(at).filter(|&v| v < 100).map(|v| (v.to_string(), 1))
}

fn capital(w: &str) -> String {
    let mut c = w.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The English rules at word `i`.
fn english(text: &Text, i: usize) -> Option<(String, usize)> {
    if let Some(found) = digit_run(text, i, |j| text.en_digit(j)) {
        return Some(found);
    }
    let (v, n, ordinal) = text.en(i)?;
    let next = |k: usize| if text.joined(i + k - 1) { text.word(i + k) } else { None };
    let digits_only = n == 1 && text.word(i).is_some_and(|w| w.parse::<u64>().is_ok());
    if ordinal {
        let month_before = i > 0 && text.joined(i - 1) && text.en_month(i - 1);
        let month_after = next(n) == Some("of") && next(n + 1).is_some() && text.en_month(i + n + 1);
        return ((month_before || month_after) || v >= 13).then(|| (format!("{v}{}", en_ordinal_suffix(v)), n));
    }
    // "nineteen eighty-nine", "twenty twenty-four"
    if n == 1 && matches!(v, 18..=20) {
        let spelled = next(1).is_some_and(|w| w.parse::<u64>().is_err());
        if let Some((rest, 1, false)) = next(1).filter(|_| spelled).and_then(|_| text.en(i + 1)) {
            if (10..100).contains(&rest) {
                return Some((format!("{}", v * 100 + rest), 2));
            }
        }
    }
    // "at three thirty", "three thirty p.m."
    if (1..=12).contains(&v) && n == 1 {
        if let Some((m, 1, false)) = next(1).and_then(|_| text.en(i + 1)) {
            let at = i > 0 && text.word(i - 1) == Some("at");
            let pm = matches!(next(2), Some("am" | "pm"))
                || (matches!(next(2), Some("a" | "p")) && text.word(i + 3) == Some("m"));
            let unit = next(2).is_some_and(|w| EN_UNIT_WORDS.contains(&w));
            if (10..60).contains(&m) && (at || pm) && !unit {
                return Some((format!("{v}:{m:02}"), 2));
            }
        }
    }
    // "three hundred dollars and fifty cents"
    if next(n).is_some_and(|w| EN_MONEY.contains(&w)) && next(n + 1) == Some("and") {
        if let Some((cents, 1, false)) = next(n + 2).and_then(|_| text.en(i + n + 2)) {
            if (1..100).contains(&cents) && matches!(next(n + 3), Some("cent" | "cents")) {
                let money = text.words[i + n].clone();
                return Some((format!("{}.{cents:02} {money}", grouped(v, ',')), n + 4));
            }
        }
    }
    // "three point one four"
    if next(n) == Some("point") {
        if let Some((decimals, k)) = decimals(text, i + n + 1, |j| text.en_digit(j), |_| None) {
            return Some((format!("{}.{decimals}", grouped(v, ',')), n + 1 + k));
        }
    }
    if digits_only {
        return None;
    }
    if next(n).is_some() && text.any_number(i + n) && en_cardinal(&text.words, &text.gaps, i + n).is_some() {
        return None;
    }
    let unit = next(n).is_some_and(|w| EN_UNIT_WORDS.contains(&w));
    let bare = n == 1 && matches!(text.word(i), Some("hundred" | "thousand"));
    (unit || (v >= 13 && !bare)).then(|| (grouped(v, ','), n))
}

/// The text with its numbers as digits, by the rules of `languages` (both
/// for dictation in many languages: German first).
pub fn digits(text: &str, languages: &[Language]) -> String {
    let (lead, tokens) = tokens(text);
    if tokens.is_empty() {
        return text.to_string();
    }
    let parsed = Text {
        words: tokens.iter().map(|t| fold(&t.word)).collect(),
        raw: tokens.iter().map(|t| t.word.clone()).collect(),
        gaps: tokens.iter().map(|t| t.after == " ").collect(),
    };
    let mut out = lead;
    let mut i = 0;
    while i < tokens.len() {
        // Right after another number ("four fifty", "drei zwanzig Euro"):
        // what was said is no number of the rules, it stays.
        let after_number = i > 0
            && parsed.joined(i - 1)
            && !matches!(parsed.word(i - 1), Some("ein" | "eine" | "one" | "a"))
            && parsed.any_number(i - 1);
        let found = languages.iter().filter(|_| !after_number).find_map(|l| match l {
            Language::German => german(&parsed, i),
            Language::English => english(&parsed, i),
        });
        match found {
            Some((written, n)) => {
                let after = &tokens[i + n - 1].after;
                out.push_str(&written);
                // "am 13." at the end of a sentence: one dot.
                if written.ends_with('.') && after.starts_with('.') {
                    out.push_str(&after[1..]);
                } else {
                    out.push_str(after);
                }
                i += n;
            }
            None => {
                // A whole number that did not fit the rules stays whole:
                // "zwanzig zwei" as said.
                out.push_str(&tokens[i].word);
                out.push_str(&tokens[i].after);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn de(s: &str) -> String {
        digits(s, &[Language::German])
    }

    fn en(s: &str) -> String {
        digits(s, &[Language::English])
    }

    #[test]
    fn german_words() {
        assert_eq!(de_cardinal("sechsundzwanzig"), Some(26));
        assert_eq!(de_cardinal("dreihundertneunundvierzig"), Some(349));
        assert_eq!(de_cardinal("zweitausendsechsundzwanzig"), Some(2026));
        assert_eq!(de_cardinal("neunzehnhundertneunundachtzig"), Some(1989));
        assert_eq!(de_cardinal("hunderteins"), Some(101));
        assert_eq!(de_cardinal("einunddreissig"), Some(31));
        assert_eq!(de_cardinal("zwölftausendfünfhundert"), Some(12500));
        assert_eq!(de_cardinal("einer"), None);
        assert_eq!(de_cardinal("achtung"), None);
        assert_eq!(de_ordinal("sechsundzwanzigsten"), Some(26));
        assert_eq!(de_ordinal("dritten"), Some(3));
        assert_eq!(de_ordinal("ersten"), Some(1));
        assert_eq!(de_ordinal("zehnten"), Some(10));
        assert_eq!(de_ordinal("siebte"), Some(7));
        assert_eq!(de_ordinal("achten"), Some(8));
        assert_eq!(de_ordinal("hundertsten"), Some(100));
        assert_eq!(de_ordinal("einundzwanzigste"), Some(21));
        assert_eq!(de_ordinal("ster"), None);
        assert_eq!(de_ordinal("seit"), None);
    }

    #[test]
    fn german_sentences() {
        assert_eq!(
            de("Wir treffen uns am sechsundzwanzigsten Mai um halb acht beim Bahnhof."),
            "Wir treffen uns am 26. Mai um halb acht beim Bahnhof."
        );
        assert_eq!(
            de("Das kostet dreihundertneunundvierzig Euro und fünfzig Cent."),
            "Das kostet 349,50 Euro."
        );
        assert_eq!(de("Das kostet zwölf Euro fünfzig."), "Das kostet 12,50 Euro.");
        assert_eq!(
            de("Ich hab zwei Kinder und die sind sechs und neun Jahre alt."),
            "Ich hab zwei Kinder und die sind sechs und neun Jahre alt."
        );
        assert_eq!(
            de("Bitte bis zum dritten zehnten zweitausendsechsundzwanzig überweisen."),
            "Bitte bis zum 3.10.2026 überweisen."
        );
        assert_eq!(
            de("Die Besprechung dauert ungefähr fünfundvierzig Minuten."),
            "Die Besprechung dauert ungefähr 45 Minuten."
        );
        assert_eq!(de("Wir brauchen sechsundzwanzig Stühle."), "Wir brauchen 26 Stühle.");
        assert_eq!(
            de("Wir fangen am Freitag um vierzehn Uhr dreißig an."),
            "Wir fangen am Freitag um 14:30 Uhr an."
        );
        assert_eq!(
            de("Meine Nummer ist null sechs neun neun eins zwei drei vier fünf."),
            "Meine Nummer ist 069912345."
        );
        assert_eq!(
            de("Er ist einer der besten und hat ein Haus mit einem Garten."),
            "Er ist einer der besten und hat ein Haus mit einem Garten."
        );
        assert_eq!(de("Um acht Uhr geht es los."), "Um 8 Uhr geht es los.");
        assert_eq!(de("Das sind drei Prozent mehr."), "Das sind 3 Prozent mehr.");
        assert_eq!(de("Es waren drei Komma fünf Liter."), "Es waren 3,5 Liter.");
        assert_eq!(de("Das war eine Million Euro."), "Das war eine Million Euro.");
        assert_eq!(de("Das waren zwei Millionen."), "Das waren 2 Millionen.");
        assert_eq!(de("Tausend Dank und hundert Grüße."), "Tausend Dank und hundert Grüße.");
        assert_eq!(de("Es kostet hundert Euro."), "Es kostet 100 Euro.");
        assert_eq!(de("Das war zwanzig zwei."), "Das war zwanzig zwei.");
        assert_eq!(
            de("Er wurde neunzehnhundertneunundachtzig geboren."),
            "Er wurde 1989 geboren."
        );
        assert_eq!(
            de("Das waren zwölftausendfünfhundert Leute."),
            "Das waren 12.500 Leute."
        );
        assert_eq!(de("Treffen am dreizehnten."), "Treffen am 13.");
        assert_eq!(de("Treffen am ersten Mai."), "Treffen am 1. Mai.");
        assert_eq!(
            de("Der dritte Versuch und das achte Mal."),
            "Der dritte Versuch und das achte Mal."
        );
        assert_eq!(de("Achte auf die Elf."), "Achte auf die Elf.");
        assert_eq!(de("Sechsundzwanzig Leute kamen."), "26 Leute kamen.");
        assert_eq!(de("Es sind 26 Leute."), "Es sind 26 Leute.");
        assert_eq!(de("Um zwölf Uhr fünfzehn."), "Um 12:15 Uhr.");
        assert_eq!(de("Es ist fünf vor zwölf."), "Es ist fünf vor zwölf.");
        assert_eq!(de("Auf Seite zweihundertdrei steht es."), "Auf Seite 203 steht es.");
        assert_eq!(
            de("Zuerst die Nacht, dann das Fest."),
            "Zuerst die Nacht, dann das Fest."
        );
        assert_eq!(
            de("Wir haben zwei Euro und zwanzig Minuten gewartet."),
            "Wir haben 2 Euro und 20 Minuten gewartet."
        );
        assert_eq!(
            de("Wir wohnen im dritten achten Stock."),
            "Wir wohnen im dritten achten Stock."
        );
        assert_eq!(de("Bis zum dritten zehnten."), "Bis zum 3.10.");
        assert_eq!(de("Um fünf Uhr zehn Minuten warten."), "Um 5 Uhr zehn Minuten warten.");
    }

    #[test]
    fn english_sentences() {
        assert_eq!(
            en("The meeting is at three thirty and we need twenty six chairs."),
            "The meeting is at 3:30 and we need 26 chairs."
        );
        assert_eq!(
            en("It costs three hundred and forty nine dollars."),
            "It costs 349 dollars."
        );
        assert_eq!(en("We need twenty-six chairs."), "We need 26 chairs.");
        assert_eq!(en("I have two kids and one dog."), "I have two kids and one dog.");
        assert_eq!(en("She was born in nineteen eighty-nine."), "She was born in 1989.");
        assert_eq!(en("See you on May twelfth."), "See you on May 12th.");
        assert_eq!(en("It is the twenty-first time."), "It is the 21st time.");
        assert_eq!(en("That is one of them."), "That is one of them.");
        assert_eq!(en("It was three point one four."), "It was 3.14.");
        assert_eq!(en("Call zero one seven six."), "Call 0176.");
        assert_eq!(en("Wait a second."), "Wait a second.");
        assert_eq!(en("Five percent more."), "5 percent more.");
        assert_eq!(
            en("It costs twelve dollars and fifty cents."),
            "It costs 12.50 dollars."
        );
        assert_eq!(en("There were fifteen thousand people."), "There were 15,000 people.");
        assert_eq!(en("We start at eight o'clock."), "We start at 8 o'clock.");
        assert_eq!(
            en("About a hundred people and one hundred cars."),
            "About a hundred people and 100 cars."
        );
        assert_eq!(en("Two three four."), "Two three four.");
        assert_eq!(de("Das kostet drei zwanzig Euro."), "Das kostet drei zwanzig Euro.");
        assert_eq!(en("We may first talk to the team."), "We may first talk to the team.");
        assert_eq!(en("I need twenty 30 chairs."), "I need 20 30 chairs.");
        assert_eq!(
            en("It is priced at four fifty dollars."),
            "It is priced at four fifty dollars."
        );
    }

    #[test]
    fn both_languages() {
        let both = [Language::German, Language::English];
        assert_eq!(
            digits("Wir brauchen sechsundzwanzig Stühle and twenty six chairs.", &both),
            "Wir brauchen 26 Stühle and 26 chairs."
        );
    }

    #[test]
    fn nothing_to_do() {
        assert_eq!(de(""), "");
        assert_eq!(de("…"), "…");
        assert_eq!(de("Hallo Welt"), "Hallo Welt");
        assert_eq!(de("Zeile eins\nZeile zwei"), "Zeile eins\nZeile zwei");
    }
}
