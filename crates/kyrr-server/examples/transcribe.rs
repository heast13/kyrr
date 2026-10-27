//! Sends WAV files to a speech server as kyrr does: what it heard and how
//! long it took. A measuring helper.
//!
//! cargo run --release -p kyrr-server --example transcribe -- URL MODEL LANG FILE.wav…

use std::time::{Duration, Instant};

use kyrr_server::Server;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let server = Server {
        url: kyrr_server::address(&args[0]).expect("an address"),
        key: std::env::var("ASR_KEY").unwrap_or_default(),
    };
    let language = Some(args[2].as_str()).filter(|l| !l.is_empty());
    for file in &args[3..] {
        let wav = std::fs::read(file).expect("the recording");
        let started = Instant::now();
        let heard = kyrr_server::transcribe(&server, &args[1], &wav, language, Duration::from_secs(600));
        println!("{file}\t{:.2}\t{heard:?}", started.elapsed().as_secs_f32());
    }
}
