//! The virtual keyboard protocol, the way `wtype` uses it: the keymap is
//! made up for the text, one key per distinct character, so the layout of the
//! user does not matter.

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::time::Instant;

use rustix::fs::{memfd_create, MemfdFlags};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_keyboard::KeymapFormat;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use super::KEY_DELAY;

/// Characters per keymap, so it stays small and far from the keycode limits.
const CHUNK: usize = 100;

/// Evdev keycode 0 is reserved, and XKB counts 8 above evdev.
const FIRST_KEY: usize = 1;
const XKB_OFFSET: usize = 8;

pub struct Keyboard {
    queue: EventQueue<State>,
    keyboard: ZwpVirtualKeyboardV1,
    started: Instant,
}

/// The objects have no events we care about.
struct State;

impl Keyboard {
    /// None when there is no Wayland compositor or it has no virtual keyboard.
    pub fn connect() -> Option<Keyboard> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection).ok()?;
        let handle = queue.handle();
        let seat: WlSeat = globals.bind(&handle, 1..=1, ()).ok()?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals.bind(&handle, 1..=1, ()).ok()?;
        let keyboard = manager.create_virtual_keyboard(&seat, &handle, ());
        queue.roundtrip(&mut State).ok()?;
        Some(Keyboard {
            queue,
            keyboard,
            started: Instant::now(),
        })
    }

    pub fn type_keysyms(&mut self, keysyms: &[u32]) -> bool {
        keysyms.chunks(CHUNK).all(|chunk| self.type_chunk(chunk).is_some())
    }

    fn type_chunk(&mut self, keysyms: &[u32]) -> Option<()> {
        let mut distinct = keysyms.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        self.upload_keymap(&distinct)?;
        for keysym in keysyms {
            let key = (distinct.binary_search(keysym).ok()? + FIRST_KEY) as u32;
            self.key(key, true)?;
            std::thread::sleep(KEY_DELAY);
            self.key(key, false)?;
            std::thread::sleep(KEY_DELAY);
        }
        Some(())
    }

    fn key(&mut self, key: u32, pressed: bool) -> Option<()> {
        let time = self.started.elapsed().as_millis() as u32;
        self.keyboard.key(time, key, u32::from(pressed));
        // The roundtrip also notices a compositor that went away.
        self.queue.roundtrip(&mut State).ok().map(drop)
    }

    fn upload_keymap(&mut self, distinct: &[u32]) -> Option<()> {
        let text = keymap(distinct);
        let mut file = File::from(memfd_create("kyrr-keymap", MemfdFlags::CLOEXEC).ok()?);
        file.write_all(text.as_bytes()).ok()?;
        // The compositor expects the terminating NUL to be part of the size.
        file.write_all(&[0]).ok()?;
        self.keyboard
            .keymap(KeymapFormat::XkbV1.into(), file.as_fd(), text.len() as u32 + 1);
        self.queue.roundtrip(&mut State).ok().map(drop)
    }
}

/// An XKB keymap with one key per keysym, in the order of `keysyms`.
fn keymap(keysyms: &[u32]) -> String {
    let mut keycodes = String::new();
    let mut symbols = String::new();
    for (index, &keysym) in keysyms.iter().enumerate() {
        let key = index + FIRST_KEY;
        keycodes += &format!("<K{key}> = {};\n", key + XKB_OFFSET);
        symbols += &format!("key <K{key}> {{ [ {} ] }};\n", name(keysym));
    }
    format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"kyrr\" {{ minimum = {XKB_OFFSET}; maximum = {}; {keycodes} }};\n\
         xkb_types \"kyrr\" {{ include \"complete\" }};\n\
         xkb_compatibility \"kyrr\" {{ include \"complete\" }};\n\
         xkb_symbols \"kyrr\" {{ {symbols} }};\n\
         }};\n",
        keysyms.len() + XKB_OFFSET,
    )
}

/// The keysym as the keymap parser reads it: `U00E4` is `adiaeresis`.
fn name(keysym: u32) -> String {
    match keysym {
        0xFF0D => "Return".into(),
        0xFF09 => "Tab".into(),
        0x0100_0000.. => format!("U{:04X}", keysym - 0x0100_0000),
        _ => format!("U{keysym:04X}"),
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! ignore_events {
    ($($interface:ty),*) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(WlSeat, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::{keymap, name};

    #[test]
    fn names_are_what_xkb_parses() {
        assert_eq!(name(0xFF0D), "Return");
        assert_eq!(name(0xE4), "U00E4");
        assert_eq!(name(0x0100_20AC), "U20AC");
    }

    #[test]
    fn keymap_has_a_key_per_keysym_above_the_reserved_code() {
        let text = keymap(&[0x61, 0xFF0D]);
        assert!(text.contains("minimum = 8; maximum = 10;"));
        assert!(text.contains("<K1> = 9;"));
        assert!(text.contains("<K2> = 10;"));
        assert!(text.contains("key <K1> { [ U0061 ] };"));
        assert!(text.contains("key <K2> { [ Return ] };"));
    }
}
