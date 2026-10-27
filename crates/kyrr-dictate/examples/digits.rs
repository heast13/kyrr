//! Numbers as digits, as kyrr writes them after recognising a dictation:
//! reads "id<TAB>text" lines, writes them back converted. A measuring
//! helper; `cargo run -p kyrr-dictate --example digits -- de < in.tsv`.

use std::io::{BufRead, Write};

use kyrr_dictate::{numerals, Language};

fn main() {
    let languages = match std::env::args().nth(1).as_deref() {
        Some("en") => vec![Language::English],
        Some("both") => Language::ALL.to_vec(),
        _ => vec![Language::German],
    };
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("a line");
        let (id, text) = line.split_once('\t').unwrap_or(("", &line));
        writeln!(out, "{id}\t{}", numerals::digits(text, &languages)).expect("writing");
    }
}
