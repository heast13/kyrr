//! One kyrr per user session. A second start hands its request to the
//! running one over a socket in the runtime folder and exits: plain `kyrr`
//! (from the app menu) shows the window, `kyrr --dictate` (bound to a key in
//! the desktop settings) starts or ends dictation.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

pub enum Instance {
    /// This is the first instance. `show_requests` fires when another start
    /// wants the window shown.
    First { show_requests: mpsc::Receiver<()> },
    /// Another instance runs and got the request.
    Other,
}

const SHOW: &[u8] = b"show";
const DICTATE: &[u8] = b"dictate";
const DICTATE_START: &[u8] = b"dictate-start";
const DICTATE_STOP: &[u8] = b"dictate-stop";

fn socket_path() -> PathBuf {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        // No folder of the user's own: the shared temp folder, one name per user.
        let uid = rustix::process::getuid().as_raw();
        return std::env::temp_dir().join(format!("kyrr-{uid}.sock"));
    };
    // In a Flatpak the runtime folder is each start's own; this one below it
    // is shared by all starts of the app.
    match std::env::var("FLATPAK_ID") {
        Ok(id) => dir.join("app").join(id).join("kyrr.sock"),
        Err(_) => dir.join("kyrr.sock"),
    }
}

/// `args` are the program's arguments; `--dictate` asks the running kyrr to
/// toggle dictation, `--dictate-start` and `--dictate-stop` to start and end
/// it (for a key's press and release).
pub fn claim(args: &[String]) -> Instance {
    let path = socket_path();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let request = if has("--dictate") {
        DICTATE
    } else if has("--dictate-start") {
        DICTATE_START
    } else if has("--dictate-stop") {
        DICTATE_STOP
    } else {
        SHOW
    };
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(request);
        return Instance::Other;
    }
    // Nobody answers: a socket left by a kyrr that ended without cleaning up.
    let _ = std::fs::remove_file(&path);
    let (tx, rx) = mpsc::channel();
    if let Ok(listener) = UnixListener::bind(&path) {
        let _ = std::thread::Builder::new()
            .name("kyrr-instance".into())
            .spawn(move || serve(listener, tx));
    }
    Instance::First { show_requests: rx }
}

fn serve(listener: UnixListener, show: mpsc::Sender<()>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut request = Vec::new();
        let _ = stream.take(16).read_to_end(&mut request);
        match request.as_slice() {
            DICTATE => crate::hotkey::toggle(),
            DICTATE_START => crate::hotkey::remote_press(),
            DICTATE_STOP => crate::hotkey::remote_release(),
            _ => {
                if show.send(()).is_err() {
                    break;
                }
            }
        }
    }
}
