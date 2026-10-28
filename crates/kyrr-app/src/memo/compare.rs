//! A memo's text before and after the language model, in two columns: the
//! text before on the left with what went in red, the new one on the right
//! with what came in green. A translation, or a rewrite with little left of
//! the text before, stands unmarked beside it.

use crate::memo::library::Paragraph;

/// What went, on the left.
const GONE: &str = "#d9534f";
/// What came instead, on the right.
const CAME: &str = "#2e9e5b";
/// Fewer words in common than this, and a paragraph is new rather than
/// changed (a translation): no marks in it.
const ALIKE: f32 = 0.3;

/// The comparison as markdown for the two columns, a pair (before, after)
/// for each paragraph.
pub fn markdown(before: &[Paragraph], after: &[Paragraph]) -> Vec<(String, String)> {
    pairs(before, after, escape)
}

/// The same as rich text (`<font color>`), for a menu that is no window of
/// kyrr's.
#[cfg(target_os = "linux")]
pub fn rich(before: &[Paragraph], after: &[Paragraph]) -> Vec<(String, String)> {
    pairs(before, after, html)
}

fn pairs(before: &[Paragraph], after: &[Paragraph], escape: fn(&str) -> String) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // In order: the next paragraph with the same place in the memo (two
    // can share one after a paragraph was split; untimed ones all do).
    let mut next = 0;
    for now in after {
        if now.text.trim().is_empty() {
            continue;
        }
        let then = match before[next.min(before.len())..]
            .iter()
            .position(|p| p.seconds == now.seconds)
        {
            Some(k) => {
                next += k + 1;
                before[next - 1].text.as_str()
            }
            None => "",
        };
        out.push(paragraph(then, &now.text, escape));
    }
    out
}

fn paragraph(then: &str, now: &str, escape: fn(&str) -> String) -> (String, String) {
    if then.trim() == now.trim() || then.trim().is_empty() {
        return (escape(then), escape(now));
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
        return (escape(then), escape(now));
    }
    // Walk both along the words they share: what lies between is gone or new.
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    for &(si, sj) in same.iter().chain(std::iter::once(&(a.len(), b.len()))) {
        if i < si {
            left.push(format!("<font color=\"{GONE}\">{}</font>", escape(&a[i..si].join(" "))));
        }
        if j < sj {
            right.push(format!("<font color=\"{CAME}\">{}</font>", escape(&b[j..sj].join(" "))));
        }
        if si < a.len() {
            left.push(escape(a[si]));
            right.push(escape(b[sj]));
        }
        (i, j) = (si + 1, sj + 1);
    }
    (left.join(" "), right.join(" "))
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

/// Text as it is, not as rich text.
#[cfg(target_os = "linux")]
fn html(s: &str) -> String {
    s.trim().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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
            md[0].0,
            "<font color=\"#d9534f\">ich hab</font> heute die \
             <font color=\"#d9534f\">Brainsbelege gewechselt</font>"
        );
        assert_eq!(
            md[0].1,
            "<font color=\"#2e9e5b\">Ich habe</font> heute die \
             <font color=\"#2e9e5b\">Bremsbeläge gewechselt\\.</font>"
        );
        assert!(slint::StyledText::from_markdown(&md[0].0).is_ok());
        assert!(slint::StyledText::from_markdown(&md[0].1).is_ok());
    }

    #[test]
    fn a_translation_stands_unmarked_beside_what_was_said() {
        let md = markdown(
            &[p(0, "Das Angebot kommt am Freitag."), p(30, "Danke!")],
            &[p(0, "The offer comes on Friday."), p(30, "Danke!")],
        );
        assert_eq!(
            md,
            [
                (
                    "Das Angebot kommt am Freitag\\.".to_string(),
                    "The offer comes on Friday\\.".to_string()
                ),
                ("Danke\\!".into(), "Danke\\!".into())
            ]
        );
    }

    #[test]
    fn paragraphs_with_the_same_time_pair_in_order() {
        let md = markdown(
            &[p(30, "Alpha beta."), p(30, "Gamma."), p(u32::MAX, "Delta.")],
            &[p(30, "Alpha beta."), p(30, "Gamma."), p(u32::MAX, "Delta.")],
        );
        let left: Vec<&str> = md.iter().map(|(before, _)| before.as_str()).collect();
        assert_eq!(left, ["Alpha beta\\.", "Gamma\\.", "Delta\\."]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rich_text_keeps_its_marks_and_escapes_its_signs() {
        let md = rich(&[p(0, "a <b> & c")], &[p(0, "a <i> & c.")]);
        assert_eq!(
            md[0],
            (
                "a <font color=\"#d9534f\">&lt;b&gt;</font> &amp; <font color=\"#d9534f\">c</font>".to_string(),
                "a <font color=\"#2e9e5b\">&lt;i&gt;</font> &amp; <font color=\"#2e9e5b\">c.</font>".to_string()
            )
        );
    }

    #[test]
    fn markdown_signs_stay_signs() {
        let md = markdown(&[], &[p(0, "1. *Punkt* <b> [x](y) # a_b")]);
        assert!(slint::StyledText::from_markdown(&md[0].1).is_ok());
        assert_eq!(
            md[0],
            (
                String::new(),
                "1\\. \\*Punkt\\* \\<b\\> \\[x\\]\\(y\\) \\# a\\_b".to_string()
            )
        );
    }
}
