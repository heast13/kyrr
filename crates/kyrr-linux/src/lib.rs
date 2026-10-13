//! Linux integration for kyrr: the virtual microphone "kyrr" in PipeWire,
//! the push-to-talk key and typing dictated text into the focused window.

#![cfg(target_os = "linux")]

pub mod background;
pub mod hotkey;
pub mod instance;
pub mod mic;
pub mod signals;
pub mod tray;
pub mod typing;
