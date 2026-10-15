//! Without PipeWire there is no noise filter. Most desktops have it; where
//! one does not, kyrr installs it with the system's package manager, after
//! the system's own password prompt (pkexec), switches the session over to
//! it and starts itself again.
//!
//! The packages are PipeWire's PulseAudio replacement and its session
//! manager, which bring PipeWire itself; installing them takes PulseAudio
//! off (apt and zypper by their conflicts, dnf with `--allowerasing`,
//! pacman with `--ask 4`, its "remove the conflicting package" answer).
//! In a Flatpak the system is out of reach: kyrr only says what to install.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Whether a PipeWire server runs in this session: its socket is there.
pub fn running() -> bool {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return false;
    };
    let name = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    let path = if Path::new(&name).is_absolute() {
        PathBuf::from(name)
    } else {
        dir.join(name)
    };
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

/// How PipeWire gets onto this system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The command as root, word by word.
    root: Vec<String>,
    /// The same for a terminal, with sudo, for the user to read.
    pub shown: String,
}

/// The way to install PipeWire here, `None` in a Flatpak, on systems kyrr
/// does not know or whose system files are read-only (Silverblue and the
/// like).
pub fn plan() -> Option<Plan> {
    if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/run/ostree-booted").exists() {
        return None;
    }
    // Full paths: pkexec looks bare names up on the caller's PATH.
    let find = |program: &str| {
        ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
            .iter()
            .map(|d| Path::new(d).join(program))
            .find(|p| p.is_file())
            .map(|p| p.display().to_string())
    };
    let words = |w: &[&str]| w.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    let (root, shown) = if let Some(apt) = find("apt-get") {
        let script =
            format!("{apt} update; DEBIAN_FRONTEND=noninteractive {apt} install -y pipewire-pulse wireplumber");
        (
            vec!["/bin/sh".to_string(), "-c".to_string(), script],
            "sudo apt install pipewire-pulse wireplumber",
        )
    } else if let Some(dnf) = find("dnf") {
        let mut root = vec![dnf];
        root.extend(words(&[
            "install",
            "-y",
            "--allowerasing",
            "pipewire-pulseaudio",
            "wireplumber",
        ]));
        (root, "sudo dnf install --allowerasing pipewire-pulseaudio wireplumber")
    } else if let Some(pacman) = find("pacman") {
        let mut root = vec![pacman];
        root.extend(words(&[
            "-S",
            "--needed",
            "--noconfirm",
            "--ask",
            "4",
            "pipewire-pulse",
            "wireplumber",
        ]));
        (root, "sudo pacman -S pipewire-pulse wireplumber")
    } else {
        let mut root = vec![find("zypper")?];
        root.extend(words(&[
            "--non-interactive",
            "install",
            "--force-resolution",
            "pipewire-pulseaudio",
            "wireplumber",
        ]));
        (root, "sudo zypper install pipewire-pulseaudio wireplumber")
    };
    Some(Plan {
        root,
        shown: shown.to_string(),
    })
}

/// Whether kyrr can ask for the password itself (polkit's pkexec).
pub fn can_ask() -> bool {
    Path::new("/usr/bin/pkexec").is_file()
}

/// How installing went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// PipeWire runs now.
    Running,
    /// Installed, but it only starts with the next sign-in.
    AfterSignIn,
    /// The password prompt was cancelled.
    Declined,
    /// It did not work; the command is for a terminal.
    Failed,
}

/// Installs PipeWire and starts it for this session. Blocks for as long as
/// the download takes.
pub fn install(plan: &Plan) -> Installed {
    let status = Command::new("/usr/bin/pkexec").args(&plan.root).status();
    match status.map(|s| s.code()) {
        Ok(Some(0)) => {}
        // pkexec: 126 when the prompt was dismissed. 127 (not allowed, or no
        // password prompt in this session) needs the terminal.
        Ok(Some(126)) => return Installed::Declined,
        Ok(_) | Err(_) => return Installed::Failed,
    }
    // PulseAudio may still hold the session; PipeWire takes over from it.
    let user = |args: &[&str]| {
        let _ = Command::new("systemctl").arg("--user").args(args).status();
    };
    user(&["daemon-reload"]);
    user(&["disable", "--now", "pulseaudio.socket", "pulseaudio.service"]);
    user(&[
        "enable",
        "--now",
        "pipewire.socket",
        "pipewire-pulse.socket",
        "wireplumber.service",
    ]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if running() {
            return Installed::Running;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Installed::AfterSignIn
}

/// Starts kyrr again once this one has ended, with `args`. The AppImage
/// is started again, not the program inside its mount, which goes away.
pub fn restart_after_exit(args: &[String]) {
    let program = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    let Some(program) = program else {
        return;
    };
    let script = "while kill -0 \"$0\" 2>/dev/null; do sleep 0.2; done; exec \"$@\"";
    // In a scope of its own where systemd runs the session: kyrr started at
    // sign-in is a service, whose leftovers systemd ends with it.
    let user_systemd = std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| Path::new(&d).join("systemd").is_dir());
    let mut command = if user_systemd && Path::new("/usr/bin/systemd-run").is_file() {
        let mut c = Command::new("/usr/bin/systemd-run");
        c.args(["--user", "--scope", "--quiet", "/bin/sh"]);
        c
    } else {
        Command::new("/bin/sh")
    };
    let result = command
        .args(["-c", script, &std::process::id().to_string()])
        .arg(program)
        .args(args)
        // Not inside the AppImage's mount, which has to go.
        .current_dir("/")
        .spawn();
    if let Err(e) = result {
        eprintln!("kyrr: starting again: {e}");
    }
}

/// Takes the AppImage's stand-in for a missing libpipewire off the library
/// path, so programs kyrr starts (a browser, gsettings) never load it. kyrr
/// itself has it loaded by now. Call first thing, before any thread runs.
pub fn forget_stand_in() {
    let Some(dir) = std::env::var_os("KYRR_PIPEWIRE_STAND_IN") else {
        return;
    };
    std::env::remove_var("KYRR_PIPEWIRE_STAND_IN");
    let Some(path) = std::env::var_os("LD_LIBRARY_PATH") else {
        return;
    };
    let rest: Vec<PathBuf> = std::env::split_paths(&path).filter(|p| p.as_os_str() != dir).collect();
    match std::env::join_paths(rest) {
        Ok(rest) if !rest.is_empty() => std::env::set_var("LD_LIBRARY_PATH", rest),
        _ => std::env::remove_var("LD_LIBRARY_PATH"),
    }
}
