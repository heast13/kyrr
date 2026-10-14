//! Ending kyrr from outside (signing out, `kill`) takes the same way as
//! quitting from the tray, so the default microphone is given back first.

use async_signal::{Signal, Signals};
use futures_lite::{future, StreamExt};

/// Calls `quit` on its own thread at the first SIGTERM, SIGINT or SIGHUP.
pub fn on_terminate(quit: impl FnOnce() + Send + 'static) {
    let Ok(mut signals) = Signals::new([Signal::Term, Signal::Int, Signal::Hup]) else {
        return;
    };
    let _ = std::thread::Builder::new().name("kyrr-signals".into()).spawn(move || {
        if future::block_on(signals.next()).is_some() {
            quit();
        }
    });
}
