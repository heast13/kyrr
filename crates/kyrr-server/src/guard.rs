//! The checks on what the language model sends back, before kyrr types it.
//!
//! Fixing goes word by word (with word order on, words may also move, see
//! [`fix_ordered`]): a change stays only if it looks like a
//! mishearing of what was dictated (similar spelling or sound), or is
//! punctuation, case, a compound split or joined, a doubled word or a filler
//! dropped. Everything else goes back to the dictation. Rewording and
//! translating are all or nothing: length, numbers, no answer of its own, the
//! language. Measured on the home server in 2026-10 (see the research notes):
//! without these checks a model "fixes" right words and makes it worse.

/// A word or one sign, and whether a space came before it.
#[derive(Clone, Debug)]
struct Token {
    text: String,
    spaced: bool,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Words (with inner hyphens and apostrophes: "U-Boot", "geht's") and
/// single signs.
fn tokens(s: &str) -> Vec<Token> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut spaced = false;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            spaced = true;
            i += 1;
            continue;
        }
        let start = i;
        if word_char(c) {
            i += 1;
            loop {
                while i < chars.len() && word_char(chars[i]) {
                    i += 1;
                }
                let joined = i + 1 < chars.len() && matches!(chars[i], '-' | '\'' | '’') && word_char(chars[i + 1]);
                if !joined {
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
        out.push(Token {
            text: chars[start..i].iter().collect(),
            spaced: spaced && !out.is_empty(),
        });
        spaced = false;
    }
    out
}

fn join(tokens: &[Token]) -> String {
    let mut s = String::new();
    for (i, t) in tokens.iter().enumerate() {
        // Two words never touch: they would have been one token. Where the
        // model's text and the dictation meet, the spacing may be either's.
        let touching = i > 0 && is_word(&t.text) && tokens[i - 1].text.chars().last().is_some_and(word_char);
        if i > 0 && (t.spaced || touching) {
            s.push(' ');
        }
        s.push_str(&t.text);
    }
    s
}

fn is_word(t: &str) -> bool {
    t.chars().next().is_some_and(word_char)
}

/// Whether two texts have the same words, whatever their case and
/// punctuation.
pub fn same_words(a: &str, b: &str) -> bool {
    words(a) == words(b)
}

/// Lower case, ß as ss: what counts as the same word.
pub fn norm(w: &str) -> String {
    w.to_lowercase().replace('ß', "ss")
}

/// Edits between two strings, by characters.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut last = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = (row[j] + 1).min(here + 1).min(last + usize::from(ca != cb));
            last = here;
        }
    }
    row[b.len()]
}

/// 1 for the same, 0 for nothing alike.
fn similarity(a: &str, b: &str) -> f32 {
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    1.0 - levenshtein(a, b) as f32 / longest as f32
}

/// Kölner Phonetik: enough to tell "sounds alike" in German.
fn koelner(word: &str) -> String {
    let w: Vec<char> = norm(word)
        .replace('ä', "a")
        .replace('ö', "o")
        .replace('ü', "u")
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect();
    let mut codes = String::new();
    for (i, &c) in w.iter().enumerate() {
        let p = if i > 0 { w[i - 1] } else { ' ' };
        let n = w.get(i + 1).copied().unwrap_or(' ');
        let k = match c {
            'a' | 'e' | 'i' | 'j' | 'o' | 'u' | 'y' => "0",
            'b' => "1",
            'p' if n == 'h' => "3",
            'p' => "1",
            'd' | 't' if "csz".contains(n) => "8",
            'd' | 't' => "2",
            'f' | 'v' | 'w' => "3",
            'g' | 'k' | 'q' => "4",
            'c' if i == 0 && "ahkloqrux".contains(n) => "4",
            'c' if i == 0 => "8",
            'c' if "sz".contains(p) || !"ahkoqux".contains(n) => "8",
            'c' => "4",
            'x' if "ckq".contains(p) => "8",
            'x' => "48",
            'l' => "5",
            'm' | 'n' => "6",
            'r' => "7",
            's' | 'z' => "8",
            _ => "",
        };
        codes.push_str(k);
    }
    let mut out: Vec<char> = Vec::new();
    for ch in codes.chars() {
        if out.last() != Some(&ch) {
            out.push(ch);
        }
    }
    let Some((&first, rest)) = out.split_first() else {
        return String::new();
    };
    std::iter::once(first)
        .chain(rest.iter().copied().filter(|&c| c != '0'))
        .collect()
}

/// Could `b` be what was said where the recognizer wrote `a`?
fn alike(a: &str, b: &str) -> bool {
    let (x, y) = (norm(a), norm(b));
    if x == y {
        return true;
    }
    // Small words that only look alike are another word ("es" for "er",
    // "Erst" for "Es"): misheard, they sound the same.
    if x.chars().count() <= 4 && y.chars().count() <= 4 {
        let k = koelner(&x);
        return !k.is_empty() && k == koelner(&y);
    }
    // Mostly the same letters: misheard ("Beschwächungsraum").
    if similarity(&x, &y) >= 0.7 {
        return true;
    }
    // A compound with another word in it is a synonym, not a mishearing
    // ("Wasserbassin" for "Wasserbecken"): what differs must sound alike
    // on its own.
    let (xc, yc): (Vec<char>, Vec<char>) = (x.chars().collect(), y.chars().collect());
    let front = xc.iter().zip(&yc).take_while(|(p, q)| p == q).count();
    let back = xc[front..]
        .iter()
        .rev()
        .zip(yc[front..].iter().rev())
        .take_while(|(p, q)| p == q)
        .count();
    let (xm, ym): (String, String) = (
        xc[front..xc.len() - back].iter().collect(),
        yc[front..yc.len() - back].iter().collect(),
    );
    if front + back >= 4 && xm.chars().count() >= 3 && ym.chars().count() >= 3 {
        return close(&xm, &ym);
    }
    close(&x, &y)
}

/// Alike in letters or in sound.
fn close(x: &str, y: &str) -> bool {
    if similarity(x, y) >= 0.5 {
        return true;
    }
    let (ka, kb) = (koelner(x), koelner(y));
    !ka.is_empty() && similarity(&ka, &kb) >= 0.67
}

const FILLERS: [&str; 9] = ["äh", "ähm", "öhm", "hm", "hmm", "uh", "um", "uhm", "erm"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Equal,
    Replace,
    Delete,
    Insert,
}

/// The longest run that `a[alo..ahi]` and `b[blo..bhi]` share, the earliest
/// one on a tie, as Python's difflib finds it (without junk).
fn longest_match(a: &[String], b: &[String], alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
    let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
    let mut lengths = vec![0usize; bhi - blo + 1];
    for (i, x) in a.iter().enumerate().take(ahi).skip(alo) {
        let mut next = vec![0usize; bhi - blo + 1];
        for (j, y) in b.iter().enumerate().take(bhi).skip(blo) {
            if x == y {
                let k = if j > blo { lengths[j - blo] } else { 0 } + 1;
                next[j - blo + 1] = k;
                if k > bestsize {
                    (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                }
            }
        }
        lengths = next;
    }
    (besti, bestj, bestsize)
}

/// How `a` turns into `b`, as difflib's opcodes.
fn opcodes(a: &[String], b: &[String]) -> Vec<(Op, usize, usize, usize, usize)> {
    let mut queue = vec![(0, a.len(), 0, b.len())];
    let mut blocks = Vec::new();
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest_match(a, b, alo, ahi, blo, bhi);
        if k > 0 {
            blocks.push((i, j, k));
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    blocks.sort_unstable();
    blocks.push((a.len(), b.len(), 0));
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::new();
    for (ai, bj, size) in blocks {
        let op = match (i < ai, j < bj) {
            (true, true) => Some(Op::Replace),
            (true, false) => Some(Op::Delete),
            (false, true) => Some(Op::Insert),
            (false, false) => None,
        };
        if let Some(op) = op {
            ops.push((op, i, ai, j, bj));
        }
        if size > 0 {
            ops.push((Op::Equal, ai, ai + size, bj, bj + size));
        }
        (i, j) = (ai + size, bj + size);
    }
    ops
}

/// What came back from fixing, with every change undone that does not look
/// like a mishearing: the text and how many changes stayed.
pub fn fix(orig: &str, out: &str) -> (String, usize) {
    let (a, b) = (tokens(orig), tokens(out));
    if b.is_empty() {
        return (orig.to_string(), 0);
    }
    let na: Vec<String> = a.iter().map(|t| norm(&t.text)).collect();
    let nb: Vec<String> = b.iter().map(|t| norm(&t.text)).collect();
    let mut res: Vec<Token> = Vec::new();
    let mut kept = 0;
    let sentence_end = |t: &Token| matches!(t.text.as_str(), "." | "!" | "?");
    for (op, i1, i2, j1, j2) in opcodes(&na, &nb) {
        let (ta, tb) = (&a[i1..i2], &b[j1..j2]);
        if op == Op::Equal {
            // The model's case wins, but not a capital that only comes from
            // a sentence start of its own around a change that went back.
            for (k, t) in tb.iter().enumerate() {
                let j = j1 + k;
                let starts_b = j == 0 || sentence_end(&b[j - 1]);
                let starts_r = res.last().is_none_or(sentence_end);
                let lower = ta[k].text.chars().next().is_some_and(char::is_lowercase);
                let upper = t.text.chars().next().is_some_and(char::is_uppercase);
                if upper && lower && starts_b && !starts_r {
                    res.push(Token {
                        text: ta[k].text.clone(),
                        spaced: t.spaced,
                    });
                } else {
                    res.push(t.clone());
                }
            }
            continue;
        }
        let wa: Vec<&str> = ta.iter().map(|t| t.text.as_str()).filter(|t| is_word(t)).collect();
        let wb: Vec<&str> = tb.iter().map(|t| t.text.as_str()).filter(|t| is_word(t)).collect();
        let ok = if wa.is_empty() && wb.is_empty() {
            // Punctuation only.
            true
        } else {
            match op {
                // A doubled word or a filler may go, nothing else.
                Op::Delete => wa.iter().all(|w| {
                    let w = norm(w);
                    FILLERS.contains(&w.as_str()) || (i1 > 0 && w == na[i1 - 1]) || (i2 < na.len() && w == na[i2])
                }),
                // New words are never a fix.
                Op::Insert => false,
                _ => {
                    let ja: String = wa.iter().map(|w| norm(w)).collect();
                    let jb: String = wb.iter().map(|w| norm(w)).collect();
                    if ja == jb {
                        // A compound split or joined.
                        true
                    } else if wa.len() == wb.len() {
                        wa.iter().zip(&wb).all(|(x, y)| alike(x, y))
                    } else if (1..=3).contains(&wa.len()) && (1..=3).contains(&wb.len()) {
                        alike(&wa.join(" "), &wb.join(" ")) || similarity(&ja, &jb) >= 0.6
                    } else {
                        false
                    }
                }
            }
        };
        if ok {
            res.extend(tb.iter().cloned());
            kept += 1;
        } else {
            res.extend(ta.iter().cloned());
        }
    }
    if let Some(first) = res.first_mut() {
        first.spaced = false;
    }
    // A model that rewrote much of the text did not fix it: all back.
    let plain = |ts: &[Token]| {
        ts.iter()
            .filter(|t| is_word(&t.text))
            .map(|t| norm(&t.text))
            .collect::<Vec<_>>()
            .join(" ")
    };
    if 1.0 - similarity(&plain(&a), &plain(&res)) > 0.34 {
        return (orig.to_string(), 0);
    }
    (join(&res), kept)
}

/// German words that must stay where they are: they say who does what to
/// whom, or what is negated (articles, pronouns, negation and focus words,
/// "and/or/but"). Nouns and names (capitalized) stay too.
const STAY_DE: &str = "der die das den dem des ein eine einen einem einer eines \
er sie es ihn ihm ihr ihnen ich mich mir du dich dir wir uns euch man sich \
mein meine meinen meinem meiner dein deine deinen deinem deiner sein seine seinen seinem seiner unser unsere unseren \
nicht kein keine keinen keinem keiner nie niemals nur auch alle alles jede jeder jedes nichts niemand jemand und oder aber";

/// Capitalized only because they start a sentence: these may move.
const START_DE: &str =
    "gestern heute morgen dann zuerst danach nachher jetzt später früher manchmal immer oft bald schon noch";

/// English words that may move: auxiliaries and time adverbs. English
/// nouns cannot be told from verbs here, so nothing else moves.
const MOVE_EN: &str =
    "is are was were am be been has have had will would can could should must may might do does did shall \
always usually often sometimes already still yesterday today tomorrow now then soon later again";

/// The words that kept their place: the longest run of `order` that only
/// goes up, and among equally long ones the one that keeps most words that
/// `stays` calls fixed. Returns for each word whether it kept its place.
fn kept_in_place(order: &[usize], stays: &[bool]) -> Vec<bool> {
    let n = order.len();
    // Weight: a word counts 1000, 1 more if it must stay.
    let weight = |k: usize| 1000 + usize::from(stays[order[k]]);
    let mut best = vec![0usize; n];
    let mut from: Vec<Option<usize>> = vec![None; n];
    for k in 0..n {
        best[k] = weight(k);
        for j in 0..k {
            if order[j] < order[k] && best[j] + weight(k) > best[k] {
                best[k] = best[j] + weight(k);
                from[k] = Some(j);
            }
        }
    }
    let mut kept = vec![false; n];
    let mut at = (0..n).max_by_key(|&k| best[k]);
    while let Some(k) = at {
        kept[k] = true;
        at = from[k];
    }
    kept
}

/// Which dictated word each word of the answer is: the same or a misheard
/// one, one to one. The words that stayed in order are found first (the
/// same word counts more than a misheard one), then the same words that
/// moved, then misheard ones that moved. Also which dictated words are
/// used.
fn pair_words(na: &[String], nb: &[String]) -> (Vec<Option<usize>>, Vec<bool>) {
    let (n, m) = (na.len(), nb.len());
    let mut mb: Vec<Option<usize>> = vec![None; m];
    let mut ua = vec![false; n];
    let gain = |x: &str, y: &str| -> f32 {
        if x == y {
            1.0
        } else if alike(x, y) {
            0.45
        } else {
            0.0
        }
    };
    let mut best = vec![vec![0.0f32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let g = gain(&na[i], &nb[j]);
            let mut b = best[i + 1][j].max(best[i][j + 1]);
            if g > 0.0 {
                b = b.max(best[i + 1][j + 1] + g);
            }
            best[i][j] = b;
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        let g = gain(&na[i], &nb[j]);
        if g > 0.0 && (best[i][j] - (best[i + 1][j + 1] + g)).abs() < 1e-4 {
            mb[j] = Some(i);
            ua[i] = true;
            i += 1;
            j += 1;
        } else if (best[i][j] - best[i + 1][j]).abs() < 1e-4 {
            i += 1;
        } else {
            j += 1;
        }
    }
    // The same word, somewhere else: it moved.
    for j in 0..m {
        if mb[j].is_none() {
            let nearest = (0..n)
                .filter(|&i| !ua[i] && na[i] == nb[j])
                .min_by_key(|&i| i.abs_diff(j));
            if let Some(i) = nearest {
                mb[j] = Some(i);
                ua[i] = true;
            }
        }
    }
    // A misheard word that moved: the closest pairs first.
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for j in (0..m).filter(|&j| mb[j].is_none()) {
        for i in (0..n).filter(|&i| !ua[i]) {
            if alike(&na[i], &nb[j]) {
                pairs.push((similarity(&na[i], &nb[j]), i, j));
            }
        }
    }
    pairs.sort_by(|x, y| y.0.total_cmp(&x.0));
    for (_, i, j) in pairs {
        if mb[j].is_none() && !ua[i] {
            mb[j] = Some(i);
            ua[i] = true;
        }
    }
    (mb, ua)
}

/// Longest dictation line the move check works on: its table grows with
/// the square of the words.
const MOST_WORDS: usize = 300;

/// The answer with words moved, if that is all it did: every word of it is
/// a dictated word (or a misheard one, as in [`fix`]), none is missing (a
/// filler or a doubled word may go), at most two words (and a third of the
/// text) changed their place, and the words that moved are ones that may:
/// in German anything but articles, pronouns, negation, nouns and names; in
/// English only auxiliaries and time adverbs; in other languages nothing.
/// So who does what to whom, and what is negated, stays as said.
fn moved_words(orig: &str, out: &str) -> Option<String> {
    let (a, b) = (tokens(orig), tokens(out));
    let na: Vec<String> = a.iter().filter(|t| is_word(&t.text)).map(|t| norm(&t.text)).collect();
    let nb: Vec<String> = b.iter().filter(|t| is_word(&t.text)).map(|t| norm(&t.text)).collect();
    if na.is_empty() || nb.is_empty() || na.len() > MOST_WORDS || nb.len() > MOST_WORDS {
        return None;
    }
    // Only German and English, told apart by their small words (the other
    // languages share too many of them to be told from a short line).
    let ws = words(orig);
    let (de, en) = (small_words(&ws, "de"), small_words(&ws, "en"));
    let language = match (de.cmp(&en), de.max(en)) {
        (_, 0) => return None,
        (std::cmp::Ordering::Greater, _) => "de",
        (std::cmp::Ordering::Less, _) => "en",
        _ => return None,
    };
    let (mb, ua) = pair_words(&na, &nb);
    let order: Vec<usize> = mb.iter().copied().collect::<Option<Vec<_>>>()?;
    for (i, used) in ua.iter().enumerate() {
        let w = na[i].as_str();
        let doubled = (i > 0 && na[i - 1] == w) || na.get(i + 1).is_some_and(|n| n == w);
        if !used && !FILLERS.contains(&w) && !doubled {
            return None;
        }
    }
    // Which dictated words may not move.
    let mut stays: Vec<bool> = Vec::new();
    let mut sentence_start = true;
    for t in &a {
        if !is_word(&t.text) {
            sentence_start = matches!(t.text.as_str(), "." | "!" | "?");
            continue;
        }
        let word = norm(&t.text);
        let capital = t.text.chars().next().is_some_and(char::is_uppercase);
        stays.push(if language == "de" {
            STAY_DE.split(' ').any(|w| w == word)
                || t.text.chars().any(|c| c.is_ascii_digit())
                || (capital && !(sentence_start && START_DE.split(' ').any(|w| w == word)))
        } else {
            !MOVE_EN.split(' ').any(|w| w == word)
        });
        sentence_start = false;
    }
    let kept = kept_in_place(&order, &stays);
    let moved = kept.iter().filter(|&&k| !k).count();
    let allowed = ((0.34 * na.len() as f32).round() as usize).max(1);
    if moved == 0 || moved > 2 || moved > allowed {
        return None;
    }
    if (0..order.len()).any(|k| !kept[k] && stays[order[k]]) {
        return None;
    }
    Some(join(&b))
}

/// Like [`fix`], but the answer may also have words moved to where the
/// grammar puts them ("weil ich hab keine Zeit" → "weil ich keine Zeit
/// hab"). Only moved: a new or missing word, more than two words moved, a
/// noun, name, pronoun or negation moved (or any word but an auxiliary or
/// time adverb in English, any word in other languages), and the answer is
/// checked as in [`fix`], which undoes every move. Misheard words in it are
/// fixed as in [`fix`].
pub fn fix_ordered(orig: &str, out: &str) -> (String, usize) {
    match moved_words(orig, out) {
        Some(text) => (text, 1),
        None => fix(orig, out),
    }
}

/// The numbers in a text, sorted, written alike: "3,5" and "3.5", "10.000",
/// "10,000" and "10 000" are the same number in another language; "1,5"
/// and "15" are not.
fn numbers(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let digits_from = |i: usize| chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut number = String::new();
        loop {
            let n = digits_from(i);
            number.extend(&chars[i..i + n]);
            i += n;
            let Some(&sep) = chars.get(i) else { break };
            let next = if i + 1 < chars.len() { digits_from(i + 1) } else { 0 };
            // Three digits after a dot, comma or space: groups of
            // thousands, the sign does not count. Fewer or more after a dot
            // or comma: a decimal sign, the same in every language.
            let thousands = next == 3 && chars.get(i + 4).is_none_or(|c| !c.is_ascii_digit());
            match sep {
                '.' | ',' | ' ' | '\u{a0}' | '\u{202f}' if thousands => {}
                '.' | ',' if next > 0 => number.push('.'),
                _ => break,
            }
            i += 1;
        }
        out.push(number);
    }
    out.sort();
    out
}

const STOP: [(&str, &str); 17] = [
    ("de", "der die das und ist nicht ich du er sie wir ihr es ein eine zu mit auf für von den dem des im mich mir sich bei nach sind war hat haben auch noch wie aber oder wenn dass einen einem einer sein seine seiner jeder jede gegen über aus vom zum zur nur schon kann wird werden hier"),
    ("en", "the and is not you he she we they it a an to with on for of in was were has have also still how but or if that this i my me are be do would can at by"),
    ("fr", "le la les et est pas je tu il elle nous vous un une des du de en pour avec sur que qui on au ce ne dans sont mais ou"),
    ("es", "el la los las y es no yo tú él ella nosotros un una de en para con que por del se"),
    ("it", "il lo la gli le e è non io tu lui lei noi voi un una di in per con su che chi del della sono"),
    ("nl", "de het een en is niet ik jij je hij zij wij we jullie van in op met voor dat die er maar ook nog zijn was hebben heeft hoe wat waarom"),
    ("pt", "o a os as e é não eu tu ele ela nós um uma de em para com que por do da dos das se são"),
    ("pl", "i w nie to jest się na że z do jak ale czy ja ty on ona my wy jestem są był co tak która który kiedy gdzie"),
    ("cs", "a je to se na že v nebo ale jak jsem jsi já ty on ona my vy jsou byl co tak už kolik kde kdy proč"),
    ("sk", "a je to sa na že v alebo ale ako som si ja ty on ona my vy sú bol čo tak už koľko kde kedy prečo"),
    ("sv", "och är inte jag du han hon vi ni det en ett att med på för av som men om eller den har var vad mig dig hur varför"),
    ("da", "og er ikke jeg du han hun vi i det en et at med på for af som men om eller den har var hvad mig dig hvor hvorfor"),
    ("no", "og er ikke jeg du han hun vi dere det en et å med på for av som men om eller den har var hva meg deg hvor hvorfor"),
    ("fi", "ja on ei minä sinä hän me te se että mutta tai kuin olen olet ovat oli myös vain jo mitä missä milloin miksi"),
    ("hu", "a az és nem én te ő mi ti egy hogy de vagy is van volt meg már csak mert hány mit hol mikor miért"),
    ("ro", "și este nu eu tu el ea noi voi un o de în pe cu la că dar sau ce sunt cât unde când"),
    ("hr", "i je da se na u ne to ali ili ja ti on ona mi vi su bio što kako sam koliko gdje kada zašto li"),
];

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !word_char(c))
        .filter(|w| !w.is_empty())
        .map(norm)
        .collect()
}

/// The language a text is most likely in, by its small words; None when
/// there are too few to tell.
pub fn language_of(s: &str) -> Option<&'static str> {
    let ws = words(s);
    if ws.is_empty() {
        return None;
    }
    let mut best = ("", 0);
    for (code, _) in STOP {
        let n = small_words(&ws, code);
        if n > best.1 {
            best = (code, n);
        }
    }
    (best.1 >= (ws.len() / 12).max(1)).then_some(best.0)
}

/// How many of `ws` are small words of the language with `code`.
fn small_words(ws: &[String], code: &str) -> usize {
    let list = STOP.iter().find(|(c, _)| *c == code).map_or("", |(_, l)| l);
    ws.iter().filter(|w| list.split(' ').any(|s| s == w.as_str())).count()
}

/// Whether `s` is clearly in another language than `wanted`: the small
/// words of another one win by two, or `wanted` has none of them. Short
/// sentences share small words ("die", "in") and are not judged by one.
pub(crate) fn other_language(s: &str, wanted: &str) -> bool {
    let Some(found) = language_of(s) else {
        return false;
    };
    if kin(found, wanted) {
        return false;
    }
    let ws = words(s);
    let (theirs, ours) = (small_words(&ws, found), small_words(&ws, wanted));
    ours == 0 || theirs >= ours + 2
}

/// Languages whose small words are too alike to tell apart by them.
pub fn kin(a: &str, b: &str) -> bool {
    const KIN: [[&str; 2]; 2] = [["da", "no"], ["cs", "sk"]];
    a == b || KIN.iter().any(|k| k.contains(&a) && k.contains(&b))
}

/// The person speaking or spoken to, by the words for it: I, you, we.
fn persons(s: &str) -> [bool; 3] {
    const PERSONS: [&str; 3] = [
        "ich mich mir mein meine meinem meinen meiner meines i me my mine myself",
        "du dich dir dein deine deinem deinen deiner deines you your yours yourself",
        "wir uns unser unsere unserem unseren unserer unseres we us our ours ourselves",
    ];
    let ws = words(s);
    PERSONS.map(|list| ws.iter().any(|w| list.split(' ').any(|p| p == w)))
}

/// The first five letters of every longer word.
fn stems(s: &str) -> std::collections::HashSet<String> {
    words(s)
        .into_iter()
        .filter(|w| w.chars().count() > 3)
        .map(|w| w.chars().take(5).collect())
        .collect()
}

/// A lead-in of the model's own ("Gerne, hier ist…:", "Here is the
/// revised text:").
fn lead_in(s: &str) -> bool {
    const STARTS: [&str; 21] = [
        "gerne",
        "gern",
        "natürlich",
        "klar",
        "hier ist",
        "hier sind",
        "hier der",
        "hier die",
        "korrigiert",
        "korrektur",
        "überarbeitet",
        "sure",
        "certainly",
        "of course",
        "here is",
        "here's",
        "here are",
        "corrected",
        "revised",
        "translation",
        "übersetzung",
    ];
    let lower = s.trim_start().to_lowercase();
    STARTS.iter().any(|start| {
        let Some(rest) = lower.strip_prefix(start) else {
            return false;
        };
        if rest.chars().next().is_some_and(word_char) {
            return false;
        }
        let line: String = rest.chars().take_while(|&c| c != '\n').take(61).collect();
        line.contains([':', '：'])
    })
}

/// How rewording or translating went wrong, for the log and the pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    LeadIn,
    Length,
    Numbers,
    Language,
    Content,
    Lines,
    /// A question came back as a statement: answered, most likely.
    Answered,
    /// "Bist du…" came back as "Bin ich…": who speaks to whom changed.
    Person,
    /// Fixing changed words, and none of the changes looked like a
    /// mishearing.
    Unlike,
}

/// Whether reworded text may be typed instead of the dictation, which is in
/// `language` (a code, None when unknown).
pub fn reworded(orig: &str, out: &str, language: Option<&str>) -> Result<(), Refusal> {
    rewritten(orig, out, language, (0.6, 1.5), 0.4)
}

/// Whether text put into business language may be typed instead: it may
/// change more words and grow a little more than plain rewording ("viel zu
/// lang" becomes "unzureichend").
pub fn formal(orig: &str, out: &str, language: Option<&str>) -> Result<(), Refusal> {
    rewritten(orig, out, language, (0.5, 1.8), 0.15)
}

fn rewritten(orig: &str, out: &str, language: Option<&str>, length: (f32, f32), kept: f32) -> Result<(), Refusal> {
    whole(orig, out, length)?;
    if language.is_some_and(|wanted| other_language(out, wanted)) {
        return Err(Refusal::Language);
    }
    let (before, after) = (persons(orig), persons(out));
    let lost = (0..3).any(|i| before[i] && !after[i]);
    let gained = (0..3).any(|i| !before[i] && after[i]);
    if lost && gained {
        return Err(Refusal::Person);
    }
    let (before, after) = (stems(orig), stems(out));
    if before.len() > 3 && (before.intersection(&after).count() as f32) < kept * before.len() as f32 {
        return Err(Refusal::Content);
    }
    // Three longer words or fewer: one of them must stay, by its start
    // ("Danke fürs Zuhören", "Vielen Dank für Ihre Aufmerksamkeit").
    let starts = |s: &str| -> std::collections::HashSet<String> {
        stems(s).into_iter().map(|w| w.chars().take(4).collect()).collect()
    };
    if (1..=3).contains(&before.len()) && starts(orig).is_disjoint(&starts(out)) {
        return Err(Refusal::Content);
    }
    Ok(())
}

/// Whether a translation into `target` (a code) may be typed instead of the
/// dictation.
pub fn translated(orig: &str, out: &str, target: &str) -> Result<(), Refusal> {
    whole(orig, out, (0.5, 2.0))?;
    // Another language must have more small words than the one asked for;
    // in a short sentence a tie is no proof either ("on" and "i" are
    // English, French, Finnish and Polish alike).
    let ws = words(out);
    let long = ws.len() > 6;
    if language_of(out).is_some_and(|found| {
        let (theirs, ours) = (small_words(&ws, found), small_words(&ws, target));
        !kin(found, target) && (theirs > ours || (long && theirs == ours))
    }) {
        return Err(Refusal::Language);
    }
    Ok(())
}

fn whole(orig: &str, out: &str, (lo, hi): (f32, f32)) -> Result<(), Refusal> {
    if out.trim().is_empty() {
        return Err(Refusal::Empty);
    }
    if lead_in(out) {
        return Err(Refusal::LeadIn);
    }
    let (o, n) = (orig.chars().count(), out.chars().count());
    let ratio = n as f32 / o.max(1) as f32;
    let (lo, hi) = if o < 40 { (lo * 0.6, hi * 1.6) } else { (lo, hi) };
    if !(lo..=hi).contains(&ratio) {
        return Err(Refusal::Length);
    }
    let after = numbers(out);
    if numbers(orig).iter().any(|n| !after.contains(n)) {
        return Err(Refusal::Numbers);
    }
    if !orig.contains('\n') && out.matches('\n').count() > 0 {
        return Err(Refusal::Lines);
    }
    if orig.contains('?') && !out.contains(['?', '？']) {
        return Err(Refusal::Answered);
    }
    Ok(())
}

/// What every answer goes through first: thinking, code fences, quotes and
/// markers around it.
pub fn clean(out: &str) -> String {
    // Control characters would be typed as keys (a backspace, a tab).
    let mut s: String = out
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    // gpt-oss hands its channels on as they are: the answer is the final one.
    if let Some(at) = s.rfind("<|channel|>final<|message|>") {
        s = s[at + "<|channel|>final<|message|>".len()..].to_string();
    }
    for end in ["<|return|>", "<|end|>"] {
        if let Some(at) = s.find(end) {
            s.truncate(at);
        }
    }
    for (open, close) in [("<think>", "</think>"), ("<|channel|>", "<|message|>")] {
        while let Some(start) = s.find(open) {
            match s[start..].find(close) {
                Some(end) => s.replace_range(start..start + end + close.len(), ""),
                // Never closed: all of it was thinking.
                None => s.truncate(start),
            }
        }
    }
    let mut s = s.trim().to_string();
    for _ in 0..2 {
        if s.starts_with("```") {
            let line = s.find('\n').unwrap_or(s.len());
            s = s[line..].to_string();
        }
        for open in ["<dictation>", "<diktat>", "<text>", "\"", "„", "«"] {
            if let Some(rest) = s.trim_start().strip_prefix(open) {
                s = rest.to_string();
            }
        }
        for close in ["```", "</dictation>", "</diktat>", "</text>", "\"", "“", "»"] {
            if let Some(rest) = s.trim_end().strip_suffix(close) {
                s = rest.to_string();
            }
        }
        s = s.trim().to_string();
    }
    s
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn words_may_move_when_nothing_else_does() {
        for (said, model) in [
            (
                "Ich komme später, weil ich hab noch einen Termin beim Arzt.",
                "Ich komme später, weil ich noch einen Termin beim Arzt hab.",
            ),
            (
                "Dann wir haben uns getroffen vor dem Bahnhof.",
                "Dann haben wir uns vor dem Bahnhof getroffen.",
            ),
            (
                "Gestern ich war im Kino mit meiner Schwester.",
                "Gestern war ich im Kino mit meiner Schwester.",
            ),
            (
                "Ich glaube, das er hat das Pakett schon abgeholt.",
                "Ich glaube, dass er das Paket schon abgeholt hat.",
            ),
            (
                "Hast du schon gegessen etwas zum Mitagessen?",
                "Hast du schon etwas zum Mittagessen gegessen?",
            ),
            (
                "Can you tell me where is the nearest pharmacy?",
                "Can you tell me where the nearest pharmacy is?",
            ),
            (
                "Kannst du mir sagen, wie viel Uhr es ist jetzt?",
                "Kannst du mir sagen, wie viel Uhr es jetzt ist?",
            ),
        ] {
            assert_eq!(fix_ordered(said, model).0, model, "{said}");
            // The plain check undoes every move.
            assert_ne!(fix(said, model).0, model, "{said}");
        }
    }

    #[test]
    fn a_move_that_changes_the_meaning_goes_back() {
        for (said, model) in [
            ("Der Hund beißt den Mann.", "Den Mann beißt der Hund."),
            (
                "Der Chef hat den Kunden angerufen.",
                "Den Chef hat der Kunde angerufen.",
            ),
            ("Ich habe ihn gesehen, nicht sie.", "Ich habe sie gesehen, nicht ihn."),
            ("Nur ich habe das gesehen.", "Ich habe nur das gesehen."),
            ("Ich habe nicht alles gelesen.", "Alles habe ich nicht gelesen."),
            ("The dog bites the man.", "The man bites the dog."),
            ("The dog bites the boy.", "The boy bites the dog."),
            ("John gave Mary the book.", "Mary gave John the book."),
            (
                "Maria hat Peter den Brief geschrieben.",
                "Peter hat Maria den Brief geschrieben.",
            ),
            ("Peter sieht Maria heute Abend.", "Maria sieht Peter heute Abend."),
            // Another language than German and English: nothing moves.
            (
                "Je pense que nous devons le faire demain matin.",
                "Je pense que nous devons demain matin le faire.",
            ),
        ] {
            assert_eq!(fix_ordered(said, model).0, fix(said, model).0, "{said}");
            assert_ne!(fix_ordered(said, model).0, model, "{said}");
        }
    }

    #[test]
    fn a_very_long_line_is_not_checked_for_moves() {
        let said = "Das ist ein langer Satz mit vielen kleinen Wörtern. ".repeat(60);
        let model = said.replacen("Das ist", "Ist das", 1);
        let started = std::time::Instant::now();
        assert_eq!(fix_ordered(&said, &model).0, fix(&said, &model).0);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn moving_is_no_license_for_other_changes() {
        // A synonym, an added word, a missing word, many moves: as with the plain check.
        for (said, model) in [
            (
                "Wir haben das Auto gekauft letzte Woche.",
                "Wir haben letzte Woche den Wagen gekauft.",
            ),
            (
                "Ich glaube, dass er hat das Paket abgeholt.",
                "Ich glaube, dass er das Paket heute abgeholt hat.",
            ),
            (
                "Ich glaube, dass er hat das Paket schon abgeholt.",
                "Ich glaube, dass er das Paket abgeholt hat.",
            ),
            (
                "Ich glaube, dass er hat das Paket abgeholt gestern Abend im Büro.",
                "Ich glaube, dass er gestern Abend im Büro das Paket abgeholt hat.",
            ),
        ] {
            assert_eq!(fix_ordered(said, model).0, fix(said, model).0, "{said}");
        }
    }

    #[test]
    fn mishearings_are_still_fixed_with_word_order_on() {
        let (text, kept) = fix_ordered(
            "Die Brainsbelege sind im Beschwächungsraum",
            "Die Bremsbeläge sind im Besprechungsraum.",
        );
        assert_eq!(text, "Die Bremsbeläge sind im Besprechungsraum.");
        assert_eq!(kept, 2);
        let q = "Was ist die Hauptstadt von Australien?";
        assert_eq!(fix_ordered(q, "Die Hauptstadt von Australien ist Canberra.").0, q);
    }

    #[test]
    fn small_words_of_two_languages_are_no_proof_against_a_translation() {
        // "on" and "i" are small words of several languages.
        assert_eq!(
            translated(
                "Wir treffen uns morgen im Büro.",
                "On se retrouve demain au bureau.",
                "fr"
            ),
            Ok(())
        );
        assert_eq!(
            translated("Ich komme gleich zurück.", "I'm coming right back.", "en"),
            Ok(())
        );
        assert_eq!(
            translated("Entschuldigung für die Verspätung.", "Please excuse my delay.", "en"),
            Ok(())
        );
        // Short, and "on" is Finnish as well as English: a tie.
        assert_eq!(
            translated(
                "Kannst du mir sagen, wie spät es ist?",
                "Voitko kertoa minulle, paljonko kello on?",
                "fi"
            ),
            Ok(())
        );
        // A plainly other language still is none.
        assert_eq!(
            translated(
                "Ich bin heute nicht im Büro.",
                "Je ne suis pas dans le bureau aujourd'hui.",
                "en"
            ),
            Err(Refusal::Language)
        );
    }

    #[test]
    fn other_words_are_no_mishearing() {
        for (said, model) in [
            ("Das Wasserbassin war leer.", "Das Wasserbecken war leer."),
            ("Die widerhaarigen Kinder kamen.", "Die widerspenstigen Kinder kamen."),
            ("Der Motor ist angelaufen und lief.", "Der Motor ist angelegt und lief."),
            ("Ich glaube es kommt morgen.", "Ich glaube er kommt morgen."),
            ("Erst sah es noch hin.", "Es sah es noch hin."),
        ] {
            assert_eq!(fix(said, model).0, said, "{model}");
        }
        for (said, model) in [
            ("Im foggländischen Dorf.", "Im Vogtländischen Dorf."),
            ("Er schmiekte sich an.", "Er schmiegte sich an."),
            ("Ich weiß das du kommst.", "Ich weiß, dass du kommst."),
            ("Sie hat es im Licht gesehen.", "Sie hat es nicht gesehen."),
        ] {
            assert_eq!(fix(said, model).0, model, "{model}");
        }
    }

    #[test]
    fn mishearings_are_fixed() {
        let (text, kept) = fix(
            "Die Brainsbelege sind im Beschwächungsraum",
            "Die Bremsbeläge sind im Besprechungsraum.",
        );
        assert_eq!(text, "Die Bremsbeläge sind im Besprechungsraum.");
        assert_eq!(kept, 2);
        let (text, _) = fix("Wir fahren nach süd Tirol", "Wir fahren nach Südtirol.");
        assert_eq!(text, "Wir fahren nach Südtirol.");
    }

    #[test]
    fn other_words_go_back() {
        // A synonym is no mishearing; a new word is never a fix.
        let (text, kept) = fix(
            "Ich gehe zur Bank und hole Geld",
            "Ich gehe zur Sparkasse und hole schnell Geld.",
        );
        assert_eq!(text, "Ich gehe zur Bank und hole Geld.");
        assert_eq!(kept, 1);
    }

    #[test]
    fn an_answer_is_not_a_fix() {
        let q = "Was ist die Hauptstadt von Australien?";
        let (text, kept) = fix(q, "Die Hauptstadt von Australien ist Canberra.");
        assert_eq!(text, q);
        assert_eq!(kept, 0);
    }

    #[test]
    fn doubled_words_and_fillers_may_go() {
        let (text, _) = fix(
            "Das das Treffen ist äh morgen um drei im großen Raum",
            "Das Treffen ist morgen um drei im großen Raum.",
        );
        assert_eq!(text, "Das Treffen ist morgen um drei im großen Raum.");
        let (text, _) = fix(
            "Das Treffen ist nicht morgen um drei im großen Raum",
            "Das Treffen ist morgen um drei im großen Raum.",
        );
        assert_eq!(text, "Das Treffen ist nicht morgen um drei im großen Raum.");
    }

    #[test]
    fn hyphens_and_apostrophes_stay_in_the_word() {
        let t: Vec<String> = tokens("Das U-Boot geht's, gut.").into_iter().map(|t| t.text).collect();
        assert_eq!(t, ["Das", "U-Boot", "geht's", ",", "gut", "."]);
        assert_eq!(join(&tokens("Ja , gut ( so ) .")), "Ja , gut ( so ) .");
    }

    #[test]
    fn opcodes_like_difflib() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let ops = opcodes(&s(&["a", "b", "c", "d"]), &s(&["a", "x", "c", "d", "e"]));
        assert_eq!(
            ops,
            [
                (Op::Equal, 0, 1, 0, 1),
                (Op::Replace, 1, 2, 1, 2),
                (Op::Equal, 2, 4, 2, 4),
                (Op::Insert, 4, 4, 4, 5)
            ]
        );
    }

    #[test]
    fn koelner_phonetik() {
        assert_eq!(koelner("Müller-Lüdenscheidt"), "65752682");
        assert_eq!(koelner("Wikipedia"), "3412");
        assert_eq!(koelner("Breschnew"), "17863");
    }

    #[test]
    fn rewording_keeps_numbers_and_language() {
        let orig = "Also ich wollte nur kurz sagen dass wir den Termin am 3. Mai machen weil der Chef keine Zeit hat";
        assert_eq!(
            reworded(
                orig,
                "Ich wollte kurz sagen, dass wir den Termin am 3. Mai machen, weil der Chef keine Zeit hat.",
                Some("de")
            ),
            Ok(())
        );
        assert_eq!(
            reworded(
                orig,
                "Ich wollte kurz sagen, dass wir den Termin im Mai machen, weil der Chef keine Zeit hat.",
                Some("de")
            ),
            Err(Refusal::Numbers)
        );
        assert_eq!(
            reworded(
                orig,
                "I just wanted to say that we have the meeting on the 3rd of May because the boss has no time.",
                Some("de")
            ),
            Err(Refusal::Language)
        );
        assert_eq!(
            reworded(
                orig,
                "Gerne, hier ist der Text: Termin am 3. Mai, der Chef hat keine Zeit, also machen wir das so.",
                Some("de")
            ),
            Err(Refusal::LeadIn)
        );
    }

    #[test]
    fn translations_land_in_the_target_language() {
        let orig = "Kannst du mir sagen, wie spät es ist?";
        assert_eq!(translated(orig, "Can you tell me what time it is?", "en"), Ok(()));
        assert_eq!(
            translated(orig, "Kannst du mir sagen, wie spät es ist?", "en"),
            Err(Refusal::Language)
        );
        assert_eq!(translated(orig, "  ", "en"), Err(Refusal::Empty));
        let long = "Wir treffen uns am Montag um 9 Uhr im Büro, bring bitte die Unterlagen mit.";
        assert_eq!(
            translated(long, "We meet on Monday at nine.", "en"),
            Err(Refusal::Length)
        );
        assert_eq!(
            translated(
                long,
                "We meet on Monday at nine in the office, please bring the documents along.",
                "en"
            ),
            Err(Refusal::Numbers)
        );
    }

    #[test]
    fn business_wording_from_the_measurements_passes() {
        let de = Some("de");
        // Real answers of Gemma 4 26B that were refused before.
        for (said, answer) in [
            ("Die Entscheidung fällt in zwei Wochen.", "Die Entscheidung wird in zwei Wochen fallen."),
            ("Kurze Erinnerung an mich selbst.", "Kurze Erinnerung an mich selbst."),
            ("Danke fürs Zuhören.", "Vielen Dank für Ihre Aufmerksamkeit."),
            (
                "Die Ladezeit ist viel zu lang, vor allem auf dem Handy.",
                "Die Ladezeit ist unzureichend, insbesondere auf mobilen Endgeräten.",
            ),
            (
                "Was ist da eigentlich los mit der Buchhaltung, warum dauert das so lange?",
                "Könnten Sie bitte erläutern, weshalb die Bearbeitung durch die Buchhaltung so viel Zeit in Anspruch nimmt?",
            ),
        ] {
            assert_eq!(formal(said, answer, de), Ok(()), "{answer}");
        }
        assert_eq!(
            formal(
                "sorry for the late reply, I was sick all week, I'll look into it first thing on Monday",
                "Please excuse my delayed response; I was on sick leave all week. I will look into this first thing on Monday.",
                Some("en")
            ),
            Ok(())
        );
        // Still refused: another language, a number gone or written out
        // (the second try asks for digits), another sentence altogether.
        assert_eq!(
            formal("Ruf mich in einer Stunde an.", "Call me in an hour.", de),
            Err(Refusal::Language)
        );
        assert_eq!(
            formal(
                "Wir brauchen 2 Leute für 5 Tage.",
                "Wir brauchen 5 Leute für zwei Tage.",
                de
            ),
            Err(Refusal::Numbers)
        );
        assert_eq!(
            formal(
                "Das Meeting ist morgen.",
                "Ich habe gestern im Garten Tomaten geerntet.",
                de
            ),
            Err(Refusal::Content)
        );
        assert_eq!(super::language_of("Lunch in Berlin?"), Some("en"));
        assert_eq!(
            formal("Dann zum Auto", "Regarding the vehicle.", de),
            Err(Refusal::Language)
        );
        assert_eq!(
            formal(
                "Das kostet 349 Euro im Monat.",
                "Das kostet monatlich einen Betrag.",
                de
            ),
            Err(Refusal::Numbers)
        );
    }

    #[test]
    fn numbers_are_the_same_in_every_language() {
        assert_eq!(
            numbers("3,5 Liter für 10.000 Euro"),
            numbers("3.5 litres for 10,000 euros")
        );
        assert_eq!(numbers("10.000 Euro"), numbers("10 000 euros"));
        assert_eq!(numbers("um 14:30 Uhr"), vec!["14", "30"]);
        assert_ne!(numbers("am 3. 10 Uhr"), numbers("am 310 Uhr"));
        assert_ne!(numbers("1,5 Meter"), numbers("15 Meter"));
        assert_eq!(numbers("1,5 Meter"), vec!["1.5"]);
        assert_eq!(
            translated("Das kostet 349,50 Euro.", "Cela coûte 349,50 euros.", "fr"),
            Ok(())
        );
        assert_eq!(
            translated("Das kostet 349,50 Euro.", "That costs 349.50 euros.", "en"),
            Ok(())
        );
    }

    #[test]
    fn who_speaks_to_whom_stays() {
        assert_eq!(
            reworded(
                "Bist du eine künstliche Intelligenz?",
                "Bin ich eine künstliche Intelligenz?",
                Some("de")
            ),
            Err(Refusal::Person)
        );
        assert_eq!(
            reworded(
                "Danke dir, das hat mir sehr geholfen.",
                "Danke, das hat mir sehr geholfen.",
                Some("de")
            ),
            Ok(())
        );
    }

    #[test]
    fn near_languages_count_as_one() {
        assert_eq!(
            translated(
                "Kannst du mir sagen, wie spät es ist?",
                "Kan du sige mig, hvad klokken er?",
                "no"
            ),
            Ok(())
        );
        assert_eq!(
            language_of("Kan du säga mig vad klockan är och om det är sent?"),
            Some("sv")
        );
        assert_eq!(
            language_of("Kun je me zeggen hoe laat het is en of het al donker is?"),
            Some("nl")
        );
        assert_eq!(
            language_of("Možeš li mi reći koliko je sati i da li je kasno?"),
            Some("hr")
        );
    }

    #[test]
    fn a_hyphen_left_hanging_keeps_its_place() {
        let (text, _) = fix(
            "Er hat die Gemüse- oder Blumenbeete gemülcht",
            "Er hat die Gemüse- oder Blumenbeete gemulcht.",
        );
        assert_eq!(text, "Er hat die Gemüse- oder Blumenbeete gemulcht.");
    }

    #[test]
    fn control_characters_are_not_typed() {
        assert_eq!(clean("Das ist\u{8}\tgut."), "Das ist  gut.");
        assert_eq!(clean("Erste Zeile\nzweite"), "Erste Zeile\nzweite");
    }

    #[test]
    fn thinking_and_wrapping_go() {
        assert_eq!(clean("<think>hmm</think>\n„Hallo Welt.“"), "Hallo Welt.");
        assert_eq!(clean("```text\nHallo.\n```"), "Hallo.");
        assert_eq!(clean("<dictation>\nHallo.\n</dictation>"), "Hallo.");
        assert_eq!(clean("<think>endless"), "");
        assert_eq!(
            clean("<|channel|>analysis<|message|>The user dictated.<|end|><|start|>assistant<|channel|>final<|message|>Hallo Welt.<|return|>"),
            "Hallo Welt."
        );
    }

    #[test]
    fn languages() {
        assert_eq!(language_of("Das ist ein Test und es geht"), Some("de"));
        assert_eq!(language_of("This is a test and it works"), Some("en"));
        assert_eq!(language_of("Xyz"), None);
    }
}
