//! Windows integration for kyrr: installing the audio filter and attaching
//! it to a microphone (elevated), the filter's settings, status and live
//! monitor, the push-to-talk key, typing text, the dictation wave on the
//! screen, and app plumbing (single instance, autostart, elevation, screen
//! size).

#![cfg(windows)]

pub mod autostart;
pub mod devices;
pub mod elevate;
pub mod filter;
pub mod hotkey;
pub mod instance;
pub mod monitor;
pub mod overlay;
mod privilege;
mod reg;
pub mod screen;
pub mod service;
pub mod setup;
pub mod typing;
pub mod window;

/// Exit code of `move <guid>` when that microphone is not plugged in (any
/// more): nothing was changed and the audio service was not restarted.
pub const EXIT_GONE: i32 = 3;

/// "Windows 11 24H2 (build 26100)", for the feedback mail. Windows 11 still
/// calls itself Windows 10 in the registry; its builds start at 22000.
pub fn version_name() -> String {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let Some(key) = reg::Key::open(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
        return "Windows".into();
    };
    let build = key.get_string("CurrentBuild").unwrap_or_default();
    let name = match build.parse::<u32>() {
        Ok(b) if b >= 22000 => "Windows 11",
        Ok(_) => "Windows 10",
        Err(_) => "Windows",
    };
    let mut text = name.to_string();
    if let Some(release) = key.get_string("DisplayVersion").filter(|v| !v.is_empty()) {
        text = format!("{text} {release}");
    }
    if !build.is_empty() {
        text = format!("{text} (build {build})");
    }
    text
}

/// Runs an elevated setup command and returns the process exit code:
/// `install` (the DLL next to this exe), `uninstall`, `move <guid>`,
/// `detach <guid>`. All but install restart the audio service so the
/// change applies; the restart happens even when the change failed.
/// On Windows on ARM, install registers nothing and takes away what an
/// earlier kyrr put there, so dictation works and the microphone stays
/// usable.
pub fn run_setup_command(args: &[String]) -> i32 {
    let restart = |r: Result<(), String>| {
        let restarted = service::restart_audio();
        r.and(restarted)
    };
    let result = match args {
        [cmd] if cmd == "install" && !setup::filter_supported() => {
            if setup::is_installed() || !setup::attached().is_empty() {
                restart(setup::uninstall())
            } else {
                Ok(())
            }
        }
        [cmd] if cmd == "install" => setup::installed_dll().and_then(|dll| setup::install(&dll)),
        [cmd] if cmd == "uninstall" => restart(setup::uninstall()),
        // Unplugged while Windows asked for permission: kyrr stays where it is.
        [cmd, guid] if cmd == "move" && setup::is_endpoint_guid(guid) && !setup::endpoint_usable(guid) => {
            return EXIT_GONE
        }
        [cmd, guid] if cmd == "move" => restart(setup::move_to(guid)),
        [cmd, guid] if cmd == "detach" => restart(setup::detach(guid)),
        _ => Err(format!("unknown setup command: {args:?}")),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("kyrr setup: {e}");
            1
        }
    }
}
