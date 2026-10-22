//! Keys as kyrr's own window sees them (winit's physical keys, which tell the
//! right Ctrl from the left, as Slint's key events do not): for the key fields
//! in the settings that wait for a key to be pressed, and for the memo key
//! when the Windows hook does not see it.

use std::sync::atomic::{AtomicBool, Ordering};

use slint::winit_030::winit::event::{ElementState, WindowEvent};
use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};

/// What a key field heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    /// A key kyrr offers, as a virtual-key code.
    Key(u32),
    /// A key a program needs for typing (or any other kyrr does not offer).
    Refused,
    /// Escape, or the window lost the focus.
    Cancelled,
}

/// A key field waits for a key.
static CAPTURING: AtomicBool = AtomicBool::new(false);

pub fn set_capturing(on: bool) {
    CAPTURING.store(on, Ordering::Relaxed);
}

/// The function keys in order: F1 is 0x70.
const F_KEYS: [KeyCode; 24] = [
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::F13,
    KeyCode::F14,
    KeyCode::F15,
    KeyCode::F16,
    KeyCode::F17,
    KeyCode::F18,
    KeyCode::F19,
    KeyCode::F20,
    KeyCode::F21,
    KeyCode::F22,
    KeyCode::F23,
    KeyCode::F24,
];

/// The key's virtual-key code if kyrr offers it.
fn virtual_key(code: KeyCode) -> Option<u32> {
    use kyrr_common::keys;
    if let Some(i) = F_KEYS.iter().position(|&k| k == code) {
        return Some(0x70 + i as u32);
    }
    match code {
        KeyCode::Pause => Some(keys::PAUSE),
        KeyCode::ScrollLock => Some(keys::SCROLL_LOCK),
        KeyCode::ControlRight => Some(keys::RIGHT_CTRL),
        KeyCode::ContextMenu => Some(keys::MENU),
        _ => None,
    }
}

/// Listens to a window's keys. While a key field waits (`set_capturing`), the
/// next key goes to `captured` and nothing else sees it (a key kyrr offers
/// counts when it comes up again, so its auto-repeat does not start the very
/// thing it was just chosen for); Escape or losing the focus gives up.
/// Otherwise the keys kyrr offers go to `platform::window_key`: Windows does
/// not run its key hook for keys that reach kyrr's own windows. Only the
/// main window has a `captured`.
pub fn install(window: &slint::Window, captured: Option<Box<dyn Fn(Captured)>>) {
    // The key heard, waiting for its release.
    let mut heard: Option<(KeyCode, u32)> = None;
    window.on_winit_window_event(move |_, event| {
        let capturing = captured.is_some() && CAPTURING.load(Ordering::Relaxed);
        if !capturing || matches!(event, WindowEvent::Focused(false)) {
            heard = None;
        }
        let say = |what| {
            if let Some(captured) = &captured {
                captured(what);
            }
        };
        match event {
            WindowEvent::Focused(false) if capturing => say(Captured::Cancelled),
            // Not the synthetic presses and releases winit makes up when the
            // window gains or loses focus with a key down.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let pressed = event.state == ElementState::Pressed;
                if !capturing {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        if virtual_key(code).is_some_and(|vk| crate::platform::window_key(vk, pressed)) {
                            return EventResult::PreventDefault;
                        }
                    }
                    return EventResult::Propagate;
                }
                let PhysicalKey::Code(code) = event.physical_key else {
                    if pressed && !event.repeat {
                        say(Captured::Refused);
                    }
                    return EventResult::PreventDefault;
                };
                if pressed && !event.repeat {
                    match virtual_key(code) {
                        Some(vk) => heard = Some((code, vk)),
                        None if code == KeyCode::Escape => {
                            heard = None;
                            say(Captured::Cancelled);
                        }
                        None => {
                            heard = None;
                            say(Captured::Refused);
                        }
                    }
                } else if !pressed {
                    if let Some((_, vk)) = heard.take_if(|(c, _)| *c == code) {
                        say(Captured::Key(vk));
                    }
                }
                return EventResult::PreventDefault;
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_kyrr_offers_has_a_physical_key() {
        let offered = (0..0x100u32).filter(|&vk| kyrr_common::keys::allowed(vk)).count();
        let physical = (0..0x100u32)
            .filter(|&vk| {
                F_KEYS
                    .iter()
                    .chain(&[
                        KeyCode::Pause,
                        KeyCode::ScrollLock,
                        KeyCode::ControlRight,
                        KeyCode::ContextMenu,
                    ])
                    .any(|&k| virtual_key(k) == Some(vk))
            })
            .count();
        assert_eq!(offered, physical);
    }

    #[test]
    fn the_two_tables_agree() {
        assert_eq!(virtual_key(KeyCode::F9), Some(0x78));
        assert_eq!(virtual_key(KeyCode::ControlRight), Some(0xA3));
        assert_eq!(virtual_key(KeyCode::ControlLeft), None);
        assert_eq!(virtual_key(KeyCode::KeyA), None);
    }
}
