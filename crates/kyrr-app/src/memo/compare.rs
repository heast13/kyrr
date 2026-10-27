//! A memo's text before and after the language model, side by side in one
//! column: each changed paragraph shows what went (struck through) and what
//! came (in colour); a translation shows the paragraph as said above it.

use crate::memo::library::Paragraph;

/// Struck through: what went.
const GONE: &str = "#d9534f";
/// What came instead.
const CAME: &str = "#2e9e5b";
/// The paragraph as said, above its translation or a rewrite with little
/// left of it (then all of the new one is in colour).
const BEFORE: &str = "#8a8a8a";
/// Fewer words in common than this, and a paragraph is new rather than
/// changed (a translation): no marks in it.
const ALIKE: f32 = 0.3;

/// The comparison as markdown for the text view, one for each paragraph.
pub fn markdown(before: &[Paragraph], after: &[Paragraph]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, now) in after.iter().enumerate() {
        if now.text.trim().is_empty() {
            continue;
        }
        // The same place in the memo, else the same place in the list.
        let then = before
            .iter()
            .find(|p| p.seconds == now.seconds && p.seconds != u32::MAX)
            .or_else(|| {
                before
                    .get(i)
                    .filter(|p| p.seconds == u32::MAX && now.seconds == u32::MAX)
            })
            .map_or("", |p| p.text.as_str());
        out.push(paragraph(then, &now.text));
    }
    out
}

fn paragraph(then: &str, now: &str) -> String {
    if then.trim() == now.trim() {
        return escape(now);
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (then.split_whitespace().collect(), now.split_whitespace().collect());
    // Very long paragraphs are not compared word by word.
    let same = if a.len() * b.len() > 1_000_000 {
        Vec::new()
    } else {
        common(&a, &b)
    };
    let alike = same.len() as f32 / a.len().max(b.len()).max(1) as f32;
    if alike < ALIKE {
        if then.trim().is_empty() {
            return escape(now);
        }
        return format!(
            "<font color=\"{BEFORE}\">{}</font>\n<font color=\"{CAME}\">{}</font>",
            escape(then),
            escape(now)
        );
    }
    // Walk both along the words they share: what lies between is gone or new.
    let mut parts = Vec::new();
    let (mut i, mut j) = (0, 0);
    for &(si, sj) in same.iter().chain(std::iter::once(&(a.len(), b.len()))) {
        if i < si {
            parts.push(format!(
                "<font color=\"{GONE}\">~~{}~~</font>",
                escape(&a[i..si].join(" "))
            ));
        }
        if j < sj {
            parts.push(format!("<font color=\"{CAME}\">{}</font>", escape(&b[j..sj].join(" "))));
        }
        if si < a.len() {
            parts.push(escape(a[si]));
        }
        (i, j) = (si + 1, sj + 1);
    }
    parts.join(" ")
}

/// The words both have, in order, as pairs of places (longest common
/// subsequence).
fn common(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut lengths = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i][j] = if a[i] == b[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Text as it is, not as markdown: every sign that could mean something
/// there goes with a backslash.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(seconds: u32, text: &str) -> Paragraph {
        Paragraph {
            seconds,
            text: text.into(),
        }
    }

    #[test]
    fn changed_words_are_marked() {
        let md = markdown(
            &[p(0, "ich hab heute die Brainsbelege gewechselt")],
            &[p(0, "Ich habe heute die Bremsbeläge gewechselt.")],
        );
        assert_eq!(
            md[0],
            "<font color=\"#d9534f\">~~ich hab~~</font> <font color=\"#2e9e5b\">Ich habe</font> heute die \
             <font color=\"#d9534f\">~~Brainsbelege gewechselt~~</font> <font color=\"#2e9e5b\">Bremsbeläge gewechselt\\.</font>"
        );
        assert!(slint::StyledText::from_markdown(&md[0]).is_ok());
    }

    #[test]
    fn a_translation_shows_what_was_said_above_it() {
        let md = markdown(
            &[p(0, "Das Angebot kommt am Freitag."), p(30, "Danke!")],
            &[p(0, "The offer comes on Friday."), p(30, "Danke!")],
        );
        assert_eq!(
            md,
            [
                "<font color=\"#8a8a8a\">Das Angebot kommt am Freitag\\.</font>\n<font color=\"#2e9e5b\">The offer comes on Friday\\.</font>",
                "Danke\\!"
            ]
        );
        assert!(md.iter().all(|m| slint::StyledText::from_markdown(m).is_ok()));
    }

    #[test]
    fn markdown_signs_stay_signs() {
        let md = markdown(&[], &[p(0, "1. *Punkt* <b> [x](y) # a_b")]);
        assert!(slint::StyledText::from_markdown(&md[0]).is_ok());
        assert_eq!(md[0], "1\\. \\*Punkt\\* \\<b\\> \\[x\\]\\(y\\) \\# a\\_b");
    }
}
