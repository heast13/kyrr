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
/// A start at sign-in while kyrr already runs (the session's autostart and
/// the compositor's both started it): nothing to do.
const BACKGROUND: &[u8] = b"background";

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
    claim_at(socket_path(), args)
}

fn claim_at(path: PathBuf, args: &[String]) -> Instance {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let request = if has("--dictate") {
        DICTATE
    } else if has("--dictate-start") {
        DICTATE_START
    } else if has("--dictate-stop") {
        DICTATE_STOP
    } else if has("--background") {
        BACKGROUND
    } else {
        SHOW
    };
    let ask = |request: &[u8]| {
        let mut stream = UnixStream::connect(&path).ok()?;
        stream.write_all(request).ok()
    };
    if ask(request).is_some() {
        return Instance::Other;
    }
    // Another kyrr started at the same moment (the session's autostart and
    // the compositor's): it holds the lock and opens its socket in a moment.
    if hold_lock(&path) == Some(false) {
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(50));
            if ask(request).is_some() {
                break;
            }
        }
        return Instance::Other;
    }
    // Nobody answers: a socket left by a kyrr that ended without cleaning up.
    let _ = std::fs::remove_file(&path);
    let (tx, rx) = mpsc::channel();
    if let Ok(listener) = UnixListener::bind(&path) {
        // Only this user may ask for dictation (the socket may sit in the
        // shared temp folder).
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        let _ = std::thread::Builder::new()
            .name("kyrr-instance".into())
            .spawn(move || serve(listener, tx));
    }
    Instance::First { show_requests: rx }
}

/// Takes the lock next to the socket for as long as kyrr runs: true when
/// this is the one kyrr, false when another holds it, `None` when locking
/// is not possible here (then the socket alone decides, as before).
fn hold_lock(socket: &std::path::Path) -> Option<bool> {
    use rustix::fs::{flock, FlockOperation};
    use std::os::unix::fs::OpenOptionsExt;
    let mut name = socket.file_name()?.to_os_string();
    name.push(".lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(socket.with_file_name(name))
        .ok()?;
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {
            // Released by the system when kyrr ends, however it ends.
            std::mem::forget(file);
            Some(true)
        }
        Err(rustix::io::Errno::WOULDBLOCK) => Some(false),
        Err(_) => None,
    }
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
            BACKGROUND => {}
            _ => {
                if show.send(()).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_hands_over() {
        let dir = std::env::temp_dir().join(format!("kyrr-instance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kyrr.sock");
        // A socket left behind by a kyrr that crashed.
        drop(UnixListener::bind(&path).unwrap());
        let Instance::First { show_requests } = claim_at(path.clone(), &[]) else {
            panic!("the first start runs");
        };
        assert!(matches!(claim_at(path.clone(), &[]), Instance::Other));
        show_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("asked to show");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
