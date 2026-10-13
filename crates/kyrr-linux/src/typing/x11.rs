//! X11 with the XTest extension. Characters the keyboard layout has no key
//! for are put on spare keycodes for the duration of the typing, the way
//! `xdotool` does it.

use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::KEY_DELAY;

const SHIFT_L: u32 = 0xFFE1;
const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;

/// Apps read the keymap again when told it changed; they need a moment
/// before the keys arrive, and again before it is put back.
const REMAP_SETTLE: Duration = Duration::from_millis(50);

pub struct Keyboard {
    connection: RustConnection,
    root: Window,
}

/// How to produce a keysym: a keycode, with Shift held or not.
#[derive(Clone, Copy)]
struct Key {
    code: u8,
    shift: bool,
}

impl Keyboard {
    /// None when there is no X server or it lacks XTest.
    pub fn connect() -> Option<Keyboard> {
        let (connection, screen) = RustConnection::connect(None).ok()?;
        let root = connection.setup().roots.get(screen)?.root;
        connection.xtest_get_version(2, 2).ok()?.reply().ok()?;
        Some(Keyboard { connection, root })
    }

    pub fn type_keysyms(&mut self, keysyms: &[u32]) -> bool {
        self.type_all(keysyms).is_ok()
    }

    fn type_all(&self, keysyms: &[u32]) -> Result<(), Box<dyn Error>> {
        let setup = self.connection.setup();
        let (first, last) = (setup.min_keycode, setup.max_keycode);
        let table = self.connection.get_keyboard_mapping(first, last - first + 1)?.reply()?;
        let rows: Vec<&[u32]> = table.keysyms.chunks(usize::from(table.keysyms_per_keycode)).collect();
        let layout = LayoutLookup {
            first,
            rows: &rows,
            base_group: self.in_base_group(),
        };

        // Decide everything before the first key, so nothing is typed when
        // the characters do not fit.
        let shift = layout.find(SHIFT_L).filter(|key| !key.shift);
        let mut spare = layout.spare_codes();
        let mut keys: HashMap<u32, Key> = HashMap::new();
        let mut remapped = Vec::new();
        for &keysym in keysyms {
            if keys.contains_key(&keysym) {
                continue;
            }
            let found = layout.find(keysym).filter(|key| !key.shift || shift.is_some());
            let key = match found {
                Some(key) => key,
                None => {
                    let code = spare.next().ok_or("no unused keycode to type with")?;
                    remapped.push((code, keysym));
                    Key { code, shift: false }
                }
            };
            keys.insert(keysym, key);
        }

        for &(code, keysym) in &remapped {
            // Both levels the same, so a held Shift changes nothing.
            self.connection.change_keyboard_mapping(1, code, 2, &[keysym, keysym])?;
        }
        if !remapped.is_empty() {
            self.connection.get_input_focus()?.reply()?;
            std::thread::sleep(REMAP_SETTLE);
        }
        let typed = self.press_all(keysyms, &keys, shift);
        if !remapped.is_empty() {
            std::thread::sleep(REMAP_SETTLE);
            for &(code, _) in &remapped {
                self.connection.change_keyboard_mapping(1, code, 2, &[0, 0])?;
            }
            self.connection.flush()?;
        }
        typed
    }

    fn press_all(&self, keysyms: &[u32], keys: &HashMap<u32, Key>, shift: Option<Key>) -> Result<(), Box<dyn Error>> {
        for keysym in keysyms {
            let key = keys[keysym];
            let shift = shift.filter(|_| key.shift);
            if let Some(shift) = shift {
                self.fake(KEY_PRESS, shift.code)?;
            }
            self.fake(KEY_PRESS, key.code)?;
            self.fake(KEY_RELEASE, key.code)?;
            if let Some(shift) = shift {
                self.fake(KEY_RELEASE, shift.code)?;
            }
            self.connection.flush()?;
            std::thread::sleep(KEY_DELAY);
        }
        Ok(())
    }

    fn fake(&self, kind: u8, code: u8) -> Result<(), Box<dyn Error>> {
        self.connection.xtest_fake_input(kind, code, 0, self.root, 0, 0, 0)?;
        Ok(())
    }

    /// The core keyboard mapping describes the first layout group only; with
    /// another one active (or if that cannot be told) the mapping is not
    /// trusted.
    fn in_base_group(&self) -> bool {
        let state = self
            .connection
            .xkb_use_extension(1, 0)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .filter(|extension| extension.supported)
            .and_then(|_| self.connection.xkb_get_state(xkb::ID::USE_CORE_KBD.into()).ok())
            .and_then(|cookie| cookie.reply().ok());
        state.is_some_and(|state| u8::from(state.group) == 0)
    }
}

/// The server's keyboard mapping, one row of keysyms per keycode from `first`.
struct LayoutLookup<'a> {
    first: u8,
    rows: &'a [&'a [u32]],
    base_group: bool,
}

impl LayoutLookup<'_> {
    /// A key that gives `keysym` on its first or (with Shift) second level.
    fn find(&self, keysym: u32) -> Option<Key> {
        if !self.base_group {
            return None;
        }
        let at = |level: usize| {
            self.rows
                .iter()
                .position(|row| row.get(level) == Some(&keysym))
                .map(|row| self.first + row as u8)
        };
        at(0)
            .map(|code| Key { code, shift: false })
            .or_else(|| at(1).map(|code| Key { code, shift: true }))
    }

    /// Keycodes nobody has a symbol on.
    fn spare_codes(&self) -> impl Iterator<Item = u8> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.iter().all(|&keysym| keysym == 0))
            .map(|(index, _)| self.first + index as u8)
    }
}
