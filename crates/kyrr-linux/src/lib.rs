//! Linux integration for kyrr: the virtual microphone "kyrr" in PipeWire,
//! the push-to-talk key and typing dictated text into the focused window.

#![cfg(target_os = "linux")]

pub mod hotkey;
pub mod mic;
pub mod typing;
