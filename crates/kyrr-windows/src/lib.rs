//! Windows integration for kyrr: installing the audio filter and attaching
//! it to a microphone (elevated), the filter's settings and status, the
//! push-to-talk key, typing text, and app plumbing (single instance,
//! autostart, elevation).

#![cfg(windows)]

pub mod autostart;
pub mod devices;
pub mod elevate;
pub mod filter;
pub mod hotkey;
pub mod instance;
mod privilege;
mod reg;
pub mod service;
pub mod setup;
pub mod typing;

/// Runs an elevated setup command and returns the process exit code:
/// `install <dll>`, `uninstall`, `attach <guid>`, `detach <guid>`.
/// Attach and detach restart the audio service so the change applies.
pub fn run_setup_command(args: &[String]) -> i32 {
    let result = match args {
        [cmd, dll] if cmd == "install" => setup::install(std::path::Path::new(dll)),
        [cmd] if cmd == "uninstall" => setup::uninstall().and_then(|()| service::restart_audio()),
        [cmd, guid] if cmd == "attach" => setup::attach(guid).and_then(|()| service::restart_audio()),
        [cmd, guid] if cmd == "detach" => setup::detach(guid).and_then(|()| service::restart_audio()),
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
