//! One kyrr per user session. A second start hands its request to the
//! running one over a socket in the runtime folder and exits: plain `kyrr`
//! (from the app menu) shows the window, `kyrr --dictate` (bound to a key in
//! the desktop settings) starts or ends dictation, `kyrr --memo` a voice
//! memo. `kyrr --replace` (the
//! install command on the website, after an update) ends a running kyrr
//! first and takes its place. Omarchy's bar widget writes its switches to
//! the same socket.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use kyrr_common::Request;

pub enum Instance {
    /// This is the first instance. `show_requests` fires when another start
    /// wants the window shown, or the bar widget a switch flipped.
    First { show_requests: mpsc::Receiver<Request> },
    /// Another instance runs and got the request.
    Other,
}

const SHOW: &[u8] = b"show";
const DICTATE: &[u8] = b"dictate";
const DICTATE_START: &[u8] = b"dictate-start";
const DICTATE_STOP: &[u8] = b"dictate-stop";
const MEMO: &[u8] = b"memo";
const MEMO_START: &[u8] = b"memo-start";
const MEMO_STOP: &[u8] = b"memo-stop";
/// A start at sign-in while kyrr already runs (the session's autostart and
/// the compositor's both started it): nothing to do.
const BACKGROUND: &[u8] = b"background";
/// From Omarchy's bar widget: what the tray menu does elsewhere.
const WIDGET: [(&[u8], Request); 10] = [
    (b"toggle-filter", Request::ToggleFilter),
    (b"toggle-voice", Request::ToggleVoice),
    (b"toggle-dictation", Request::ToggleDictation),
    (b"toggle-memos", Request::ToggleMemos),
    (b"toggle-memo", Request::ToggleMemo),
    (b"open-memos", Request::OpenMemos),
    (b"install-update", Request::InstallUpdate),
    (b"quit", Request::Quit),
    (b"menu-open", Request::WidgetOpen(true)),
    (b"menu-closed", Request::WidgetOpen(false)),
];
/// The first words of the lines the widget's menu sends: everything kyrr
/// does, there.
const LINE_WORDS: [&str; 9] = [
    "set ", "toggle ", "mic ", "memo ", "update ", "test ", "setup ", "open ", "model ",
];
/// How long a running kyrr gets to hand back the microphone and end.
const REPLACE_WAIT: Duration = Duration::from_secs(20);

fn socket_path() -> PathBuf {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        // No folder of the user's own: the shared temp folder, one name per user.
        let uid = rustix::process::getuid().as_raw();
        return std::env::temp_dir().join(format!("kyrr-{uid}.sock"));
    };
    dir.join("kyrr.sock")
}

/// `args` are the program's arguments; `--dictate` asks the running kyrr to
/// toggle dictation, `--dictate-start` and `--dictate-stop` to start and end
/// it (for a key's press and release), `--memo` to start or end a voice
/// memo, `--memo-start` and `--memo-stop` to start and end one (the same); `--replace` ends the running kyrr
/// the way quitting does and takes its place.
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
    } else if has("--memo-start") {
        MEMO_START
    } else if has("--memo-stop") {
        MEMO_STOP
    } else if has("--memo") {
        MEMO
    } else if has("--background") {
        BACKGROUND
    } else {
        SHOW
    };
    if has("--replace") {
        end_running(&path);
    }
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

/// Ends the kyrr that listens on `path` with SIGTERM (it gives the default
/// microphone back, see signals) and waits until it has gone. Whoever
/// listens is asked by the system, not by name; only a process called kyrr
/// is touched.
fn end_running(path: &std::path::Path) {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return;
    };
    // Looking must not make the dying kyrr show its window.
    let _ = stream.write_all(BACKGROUND);
    let Ok(peer) = rustix::net::sockopt::socket_peercred(&stream) else {
        return;
    };
    drop(stream);
    let pid = peer.pid;
    let is_kyrr =
        std::fs::read_to_string(format!("/proc/{}/comm", pid.as_raw_nonzero())).is_ok_and(|name| name.trim() == "kyrr");
    if !is_kyrr || pid == rustix::process::getpid() {
        return;
    }
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    // Gone when nothing accepts on the socket any more.
    let until = std::time::Instant::now() + REPLACE_WAIT;
    while std::time::Instant::now() < until {
        match UnixStream::connect(path) {
            Ok(mut stream) => {
                let _ = stream.write_all(BACKGROUND);
            }
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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

fn serve(listener: UnixListener, show: mpsc::Sender<Request>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            // Out of file handles, say: again in a moment, not in a spin.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut request = Vec::new();
        let _ = stream.take(1024).read_to_end(&mut request);
        // The widget ends its lines with a newline.
        while request.last().is_some_and(u8::is_ascii_whitespace) {
            request.pop();
        }
        match request.as_slice() {
            DICTATE => crate::hotkey::toggle(),
            DICTATE_START => crate::hotkey::remote_press(),
            DICTATE_STOP => crate::hotkey::remote_release(),
            MEMO => crate::hotkey::memo_press(),
            MEMO_START => crate::hotkey::memo_remote_start(),
            MEMO_STOP => crate::hotkey::memo_up(),
            BACKGROUND => {}
            other => {
                // Anything else is a mistake, not a reason to pop up.
                let request = if other == SHOW {
                    Some(Request::Show)
                } else {
                    WIDGET
                        .iter()
                        .find(|(word, _)| *word == other)
                        .map(|(_, request)| request.clone())
                        .or_else(|| page(other))
                        .or_else(|| line(other))
                };
                if request.is_some_and(|request| show.send(request).is_err()) {
                    break;
                }
            }
        }
    }
}

/// `page-1` to `page-5`, from the widget's Setup row and its gears.
fn page(word: &[u8]) -> Option<Request> {
    match word.strip_prefix(b"page-")? {
        [digit @ b'1'..=b'5'] => Some(Request::OpenPage(i32::from(digit - b'0'))),
        _ => None,
    }
}

/// A line from the widget's menu, such as `set voice.tone warm`.
fn line(text: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(text).ok()?;
    LINE_WORDS
        .iter()
        .any(|word| text.starts_with(word))
        .then(|| Request::Line(text.to_string()))
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
        assert_eq!(show_requests.recv_timeout(Duration::from_secs(2)), Ok(Request::Show));
        UnixStream::connect(&path).unwrap().write_all(b"toggle-voice").unwrap();
        assert_eq!(
            show_requests.recv_timeout(Duration::from_secs(2)),
            Ok(Request::ToggleVoice)
        );
        UnixStream::connect(&path).unwrap().write_all(b"page-3").unwrap();
        assert_eq!(
            show_requests.recv_timeout(Duration::from_secs(2)),
            Ok(Request::OpenPage(3))
        );
        assert_eq!(page(b"page-9"), None);
        UnixStream::connect(&path)
            .unwrap()
            .write_all(b"set voice.tone warm\n")
            .unwrap();
        assert_eq!(
            show_requests.recv_timeout(Duration::from_secs(2)),
            Ok(Request::Line("set voice.tone warm".into()))
        );
        UnixStream::connect(&path).unwrap().write_all(b"menu-open\n").unwrap();
        assert_eq!(
            show_requests.recv_timeout(Duration::from_secs(2)),
            Ok(Request::WidgetOpen(true))
        );
        assert_eq!(line(b"settle"), None);
        // A word nobody knows does nothing; the next one still arrives.
        UnixStream::connect(&path).unwrap().write_all(b"shwo").unwrap();
        UnixStream::connect(&path).unwrap().write_all(b"toggle-filter").unwrap();
        assert_eq!(
            show_requests.recv_timeout(Duration::from_secs(2)),
            Ok(Request::ToggleFilter)
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `--replace` ends only a process called kyrr: this test (the listener
    /// here) is not one and goes on answering.
    #[test]
    fn replace_leaves_other_programs_alone() {
        let dir = std::env::temp_dir().join(format!("kyrr-replace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kyrr.sock");
        let _listener = UnixListener::bind(&path).unwrap();
        end_running(&path);
        assert!(UnixStream::connect(&path).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
