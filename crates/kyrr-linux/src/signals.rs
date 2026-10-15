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

/// Ends kyrr once it has cleaned up, without the exit handlers of the C++
/// libraries inside (the speech models). They have nothing left to save,
/// and at sign-out the AppImage's mount can be gone before them: running
/// code not read from it yet then crashes kyrr (SIGBUS) on its way out.
pub fn exit_now(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: _exit ends the process at once; nothing runs after it.
    unsafe { libc::_exit(code) }
}
