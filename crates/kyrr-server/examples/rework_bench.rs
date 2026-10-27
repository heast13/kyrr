//! Measures one rework preset against a real server: how often the first
//! answer passes the check, how often the second try (with what was wrong)
//! does, and how often only fixing is left.
//!
//! cargo run --release -p kyrr-server --example rework_bench -- URL MODEL [business|polish] [dictations.jsonl] [results.jsonl]
//!
//! As kyrr does it while dictating, with its time limits, steps such as
//! "fix", "polish+en", "business+fr" or "en": `live:STEPS` in place of the
//! preset. `prime:STEPS` measures how much reading ahead while the user
//! speaks saves: each dictation once after another preset, once primed.
//! With MEMO set, live mode gives each item the time of a memo paragraph.
//! `replay:fix` needs no server: it checks answers measured before (lines
//! with "hyp" or "text" and the model's "raw" answer) with today's check.
//!
//! A key goes in LLM_KEY. Each line of the dictations is
//! {"id": …, "lang": "de"|"en", "text": …}, numbers already as digits (as
//! kyrr hands them on).

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use kyrr_server::rework::{self, Outcome, Preset};
use kyrr_server::Server;
use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = kyrr_server::address(args.first().expect("URL")).expect("an address");
    let model = args.get(1).expect("MODEL").clone();
    let preset = match args.get(2).map(String::as_str) {
        Some("polish") => Preset::Polish,
        _ => Preset::Business,
    };
    let input = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/examples/dictations.jsonl").into());
    let output = args.get(4).cloned().unwrap_or_else(|| "rework_bench.jsonl".into());
    let server = Server {
        url,
        key: std::env::var("LLM_KEY").unwrap_or_default(),
    };
    if let Some(mode) = args.get(2).and_then(|m| m.split_once(':')) {
        let input = args
            .get(3)
            .cloned()
            .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/examples/dictations.jsonl").into());
        let output = args.get(4).cloned().unwrap_or_else(|| "rework_bench.jsonl".into());
        if mode.0 == "replay" {
            replay(&read_items(&input), &output);
            return;
        }
        let steps = steps(mode.1);
        match mode.0 {
            "prime" => primed(&server, &model, &steps, &read_items(&input), &output),
            _ => live(&server, &model, &steps, &read_items(&input), &output),
        }
        return;
    }
    let items: Vec<Value> = std::fs::read_to_string(&input)
        .expect("the dictations")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a json line"))
        .collect();
    let mut out = std::fs::File::create(&output).expect("the results file");
    let mut stages: BTreeMap<&str, usize> = BTreeMap::new();
    let mut first_why: BTreeMap<String, usize> = BTreeMap::new();
    let mut second_why: BTreeMap<String, usize> = BTreeMap::new();
    let mut times = Vec::new();
    let mut over = 0;
    let stop = AtomicBool::new(false);
    let timeout = Duration::from_secs(60);
    for (n, item) in items.iter().enumerate() {
        let id = item["id"].as_str().unwrap_or_default();
        let text = item["text"].as_str().unwrap_or_default();
        let language = item["lang"].as_str();
        let started = Instant::now();
        let mut record = json!({"id": id, "text": text});
        let first = rework::attempt(&server, &model, preset, text, language, None, timeout);
        let stage = match first {
            Err(e) => {
                record["error"] = json!(format!("{e:?}"));
                "server error"
            }
            Ok((answer, Ok(_))) => {
                record["first"] = json!(answer);
                "first try"
            }
            Ok((answer, Err(refusal))) => {
                record["first"] = json!(answer);
                record["first_refused"] = json!(format!("{refusal:?}"));
                *first_why.entry(format!("{refusal:?}")).or_default() += 1;
                match rework::attempt(
                    &server,
                    &model,
                    preset,
                    text,
                    language,
                    Some((answer.as_str(), &refusal)),
                    timeout,
                ) {
                    Ok((again, Ok(_))) => {
                        record["second"] = json!(again);
                        "second try"
                    }
                    Ok((again, Err(refusal))) => {
                        record["second"] = json!(again);
                        record["second_refused"] = json!(format!("{refusal:?}"));
                        *second_why.entry(format!("{refusal:?}")).or_default() += 1;
                        let deadline = Instant::now() + timeout;
                        match rework::rework(&server, &model, Preset::Fix, text, language, deadline, &stop) {
                            Outcome::Reworked(fixed) => {
                                record["fixed"] = json!(fixed);
                                "fixed instead"
                            }
                            other => {
                                record["fix"] = json!(format!("{other:?}"));
                                "as said"
                            }
                        }
                    }
                    Err(e) => {
                        record["error"] = json!(format!("{e:?}"));
                        "server error"
                    }
                }
            }
        };
        let took = started.elapsed();
        let limit = rework::time_limit(preset, text);
        if took > limit {
            over += 1;
        }
        record["stage"] = json!(stage);
        record["seconds"] = json!((took.as_secs_f32() * 100.0).round() / 100.0);
        record["limit"] = json!((limit.as_secs_f32() * 10.0).round() / 10.0);
        *stages.entry(stage).or_default() += 1;
        times.push(took.as_secs_f32());
        writeln!(out, "{record}").expect("writing the results");
        if stage != "first try" {
            println!(
                "{n:>3} {id}: {stage} {}",
                record.get("first_refused").unwrap_or(&json!(""))
            );
        }
    }
    times.sort_by(f32::total_cmp);
    println!("\n{} dictations, preset {preset:?}, model {model}", items.len());
    for (stage, count) in &stages {
        println!("  {stage:<14} {count:>4}");
    }
    println!("  first refusals:  {first_why:?}");
    println!("  second refusals: {second_why:?}");
    if !times.is_empty() {
        println!(
            "  seconds: median {:.2}, max {:.2}; over the live time limit: {over}",
            times[times.len() / 2],
            times[times.len() - 1]
        );
    }
    println!("  details: {output}");
}

fn read_items(input: &str) -> Vec<Value> {
    std::fs::read_to_string(input)
        .expect("the dictations")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a json line"))
        .collect()
}

/// "polish+en" as the steps kyrr takes.
fn steps(spec: &str) -> Vec<Preset> {
    spec.split('+')
        .map(|step| match step {
            "fix" => Preset::Fix,
            "polish" => Preset::Polish,
            "business" => Preset::Business,
            code => Preset::Translate(
                rework::TARGETS
                    .iter()
                    .find(|(c, _)| *c == code)
                    .map(|(c, _)| *c)
                    .expect("a preset or a language code"),
            ),
        })
        .collect()
}

fn kind(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Reworked(_) => "reworked".into(),
        Outcome::FixedInstead(_) => "fixed instead".into(),
        Outcome::Original(why) => format!("{why:?}"),
    }
}

/// Every dictation as kyrr reworks it while dictating, with its time limit.
fn live(server: &Server, model: &str, steps: &[Preset], items: &[Value], output: &str) {
    let mut out = std::fs::File::create(output).expect("the results file");
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut times = Vec::new();
    let stop = AtomicBool::new(false);
    for item in items {
        let text = item["text"].as_str().unwrap_or_default();
        let mut limit = rework::time_limit_all(steps, text);
        // A memo's paragraph: as kyrr gives it, nobody waits at the keyboard.
        if std::env::var_os("MEMO").is_some() {
            limit = (limit * 4).max(Duration::from_secs(30));
        }
        let started = Instant::now();
        let outcomes = rework::rework_all(
            server,
            model,
            steps,
            text,
            item["lang"].as_str(),
            started + limit,
            &stop,
        );
        let took = started.elapsed().as_secs_f32();
        times.push(took);
        let said: Vec<String> = outcomes.iter().map(kind).collect();
        let typed = outcomes
            .iter()
            .rev()
            .find_map(|o| match o {
                Outcome::Reworked(t) | Outcome::FixedInstead(t) => Some(t.as_str()),
                Outcome::Original(_) => None,
            })
            .unwrap_or(text);
        *kinds.entry(said.join(" / ")).or_default() += 1;
        let record = json!({"id": item["id"], "text": text, "typed": typed, "outcomes": said,
            "seconds": (took * 100.0).round() / 100.0, "limit": (limit.as_secs_f32() * 10.0).round() / 10.0});
        writeln!(out, "{record}").expect("writing the results");
    }
    times.sort_by(f32::total_cmp);
    println!("\n{} dictations, live {steps:?}, model {model}", items.len());
    for (kind, count) in &kinds {
        println!("  {count:>4}  {kind}");
    }
    if !times.is_empty() {
        println!(
            "  seconds: median {:.2}, 90 % {:.2}, max {:.2}",
            times[times.len() / 2],
            times[times.len() * 9 / 10],
            times[times.len() - 1]
        );
    }
    println!("  details: {output}");
}

/// The time to the answer with and without reading ahead.
fn primed(server: &Server, model: &str, steps: &[Preset], items: &[Value], output: &str) {
    let mut out = std::fs::File::create(output).expect("the results file");
    let stop = AtomicBool::new(false);
    let (mut cold, mut warm) = (Vec::new(), Vec::new());
    let other = if steps[0] == Preset::Fix {
        Preset::Translate("fr")
    } else {
        Preset::Fix
    };
    let far = Duration::from_secs(60);
    for item in items {
        let text = item["text"].as_str().unwrap_or_default();
        let language = item["lang"].as_str();
        // Something else read last: the instructions are new to the server.
        rework::prime(server, model, other, far);
        let started = Instant::now();
        rework::rework_all(server, model, steps, text, language, started + far, &stop);
        let c = started.elapsed().as_secs_f32();
        rework::prime(server, model, steps[0], far);
        let started = Instant::now();
        rework::rework_all(server, model, steps, text, language, started + far, &stop);
        let w = started.elapsed().as_secs_f32();
        writeln!(out, "{}", json!({"id": item["id"], "cold": c, "warm": w})).expect("writing the results");
        cold.push(c);
        warm.push(w);
    }
    for v in [&mut cold, &mut warm] {
        v.sort_by(f32::total_cmp);
    }
    let median = |v: &[f32]| v.get(v.len() / 2).copied().unwrap_or_default();
    println!(
        "\n{} dictations, {steps:?}, model {model}: median {:.2} s without, {:.2} s with reading ahead",
        items.len(),
        median(&cold),
        median(&warm)
    );
}

/// Answers of a fix run measured before, through today's check.
fn replay(items: &[Value], output: &str) {
    let mut out = std::fs::File::create(output).expect("the results file");
    let mut changed = 0;
    for item in items {
        let text = item["hyp"].as_str().or(item["text"].as_str()).unwrap_or_default();
        let raw = item["raw"].as_str().unwrap_or_default();
        let (typed, _) = kyrr_server::guard::fix(text, &kyrr_server::guard::clean(raw));
        changed += usize::from(typed != text);
        let mut record = item.clone();
        record["typed"] = json!(typed);
        writeln!(out, "{record}").expect("writing the results");
    }
    println!(
        "{} answers checked again, {changed} changed; details: {output}",
        items.len()
    );
}
