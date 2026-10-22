//! Linux integration for kyrr: the virtual microphone "kyrr" in PipeWire,
//! the push-to-talk key, typing dictated text into the focused window and
//! the dictation wave on the screen.

#![cfg(target_os = "linux")]

pub mod binds;
pub mod display;
pub mod files;
pub mod gnome;
pub mod hotkey;
pub mod instance;
pub mod mic;
pub mod overlay;
pub mod pipewire_setup;
pub mod signals;
pub mod tray;
pub mod typing;
