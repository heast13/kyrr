//! Windows integration for kyrr: installing the audio filter and attaching
//! it to a microphone (elevated), the filter's settings, status and live
//! monitor, the
//! push-to-talk key, typing text, and app plumbing (single instance,
//! autostart, elevation, screen size).

#![cfg(windows)]

pub mod autostart;
pub mod devices;
pub mod elevate;
pub mod filter;
pub mod hotkey;
pub mod instance;
pub mod monitor;
mod privilege;
mod reg;
pub mod screen;
pub mod service;
pub mod setup;
pub mod typing;
pub mod window;

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
