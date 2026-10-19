//! The virtual keyboard protocol, the way `wtype` uses it: the keymap is
//! made up for the text, one key per distinct character, so the layout of the
//! user does not matter.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use rustix::fs::{memfd_create, MemfdFlags};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_keyboard::KeymapFormat;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use super::KEY_DELAY;

/// Distinct characters per keymap, below what the usable key codes hold. A text with
/// at most this many gets one keymap for all of it: a keymap that changes in the middle
/// of a text reaches the apps late on some compositors (Hyprland), and the
/// rest of the text then comes out as other letters.
const MAX_KEYS: usize = 120;

/// Apps read a new keymap when it arrives; they need a moment before the
/// first key of the text.
const KEYMAP_SETTLE: Duration = Duration::from_millis(30);

/// XKB counts 8 above evdev, and evdev codes above 247 do not fit a keymap.
const XKB_OFFSET: usize = 8;
const LAST_CODE: u32 = 247;

/// The typing block of a keyboard (digits, the letter rows, space): the keys
/// of the made-up keymap go here first.
const TEXT_CODES: [std::ops::RangeInclusive<u32>; 5] = [2..=13, 16..=27, 30..=41, 43..=53, 57..=57];

/// Evdev codes the made-up keys must never use. Browsers read the evdev code
/// and not only the keysym: a character on Esc, Backspace or Enter is thrown
/// away or acted on instead of typed (the space of a dictation was lost this
/// way). Reserved 0, then Esc, Backspace, Tab, Enter, the modifiers, the lock
/// keys, the function keys, the cursor block, Pause/Compose, and the keys with
/// an effect of their own (volume, power, menu, browser and media keys).
const SPECIAL_CODES: [std::ops::RangeInclusive<u32>; 21] = [
    0..=1,
    14..=15,
    28..=29,
    42..=42,
    54..=56,
    58..=70,
    87..=88,
    96..=97,
    99..=100,
    102..=111,
    113..=116,
    119..=119,
    125..=127,
    139..=139,
    142..=143,
    152..=152,
    158..=160,
    163..=165,
    172..=174,
    183..=194,
    210..=247,
];

/// The evdev codes for `count` made-up keys: the typing block first (enough
/// for any ordinary text), then whatever else is no special key, which only a
/// text with an unusual lot of different characters reaches.
fn key_codes(count: usize) -> Vec<u32> {
    let is_special = |code: &u32| SPECIAL_CODES.iter().any(|range| range.contains(code));
    let preferred = TEXT_CODES.iter().flat_map(|range| range.clone());
    let rest = (0..=LAST_CODE).filter(|code| !is_special(code) && !TEXT_CODES.iter().any(|r| r.contains(code)));
    preferred.chain(rest).take(count).collect()
}

/// Each text gets a virtual keyboard of its own (a connection of its own),
/// the way `wtype` works: Hyprland hands a changed keymap of a keyboard that
/// already exists to the focused window only when the focus changes, so the
/// second text would come out as other letters.
pub struct Keyboard {
    /// The one made by `connect`, kept for the first text.
    spare: Option<Session>,
}

struct Session {
    queue: EventQueue<State>,
    keyboard: ZwpVirtualKeyboardV1,
    started: Instant,
}

/// The objects have no events we care about.
struct State;

impl Keyboard {
    /// None when there is no Wayland compositor or it has no virtual keyboard.
    pub fn connect() -> Option<Keyboard> {
        Some(Keyboard {
            spare: Some(Session::open()?),
        })
    }

    pub fn type_keysyms(&mut self, keysyms: &[u32]) -> bool {
        let Some(mut session) = self.spare.take().or_else(Session::open) else {
            return false;
        };
        // `session` is dropped at the end: the keyboard goes with the connection.
        segments(keysyms)
            .into_iter()
            .all(|segment| session.type_chunk(segment).is_some())
    }
}

impl Session {
    fn open() -> Option<Session> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection).ok()?;
        let handle = queue.handle();
        let seat: WlSeat = globals.bind(&handle, 1..=1, ()).ok()?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals.bind(&handle, 1..=1, ()).ok()?;
        let keyboard = manager.create_virtual_keyboard(&seat, &handle, ());
        queue.roundtrip(&mut State).ok()?;
        Some(Session {
            queue,
            keyboard,
            started: Instant::now(),
        })
    }

    fn type_chunk(&mut self, keysyms: &[u32]) -> Option<()> {
        let mut distinct = keysyms.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        self.upload_keymap(&distinct)?;
        std::thread::sleep(KEYMAP_SETTLE);
        let codes = key_codes(distinct.len());
        for keysym in keysyms {
            let key = codes[distinct.binary_search(keysym).ok()?];
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

/// The text cut into pieces of at most `MAX_KEYS` distinct keysyms, which is
/// one piece for any ordinary text.
fn segments(keysyms: &[u32]) -> Vec<&[u32]> {
    let mut pieces = Vec::new();
    let mut seen = HashSet::new();
    let mut start = 0;
    for (index, keysym) in keysyms.iter().enumerate() {
        if !seen.contains(keysym) && seen.len() == MAX_KEYS {
            pieces.push(&keysyms[start..index]);
            start = index;
            seen.clear();
        }
        seen.insert(*keysym);
    }
    pieces.push(&keysyms[start..]);
    pieces
}

/// An XKB keymap with one key per keysym, on the codes of `key_codes` in the
/// order of `keysyms`.
fn keymap(keysyms: &[u32]) -> String {
    let mut keycodes = String::new();
    let mut symbols = String::new();
    let codes = key_codes(keysyms.len());
    for (&keysym, &code) in keysyms.iter().zip(&codes) {
        keycodes += &format!("<K{code}> = {};\n", code as usize + XKB_OFFSET);
        symbols += &format!("key <K{code}> {{ [ {} ] }};\n", name(keysym));
    }
    format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"kyrr\" {{ minimum = {XKB_OFFSET}; maximum = {}; {keycodes} }};\n\
         xkb_types \"kyrr\" {{ include \"complete\" }};\n\
         xkb_compatibility \"kyrr\" {{ include \"complete\" }};\n\
         xkb_symbols \"kyrr\" {{ {symbols} }};\n\
         }};\n",
        codes.iter().max().map_or(0, |&code| code as usize) + XKB_OFFSET,
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
    use super::{key_codes, keymap, name, segments, MAX_KEYS, SPECIAL_CODES, TEXT_CODES};

    #[test]
    fn a_long_ordinary_text_is_one_segment() {
        let text: Vec<u32> = "Ich muss ja irgendwie Schulen finanzieren, ich habe Staatsbedienstete. "
            .repeat(20)
            .chars()
            .map(u32::from)
            .collect();
        assert!(text.len() > 1000);
        assert_eq!(segments(&text), vec![&text[..]]);
    }

    #[test]
    fn many_distinct_characters_are_split_without_losing_any() {
        let text: Vec<u32> = (0x100..0x100 + 3 * MAX_KEYS as u32 + 5).collect();
        let pieces = segments(&text);
        assert_eq!(pieces.len(), 4);
        assert!(pieces.iter().all(|piece| piece.len() <= MAX_KEYS));
        assert_eq!(pieces.concat(), text);
    }

    #[test]
    fn names_are_what_xkb_parses() {
        assert_eq!(name(0xFF0D), "Return");
        assert_eq!(name(0xE4), "U00E4");
        assert_eq!(name(0x0100_20AC), "U20AC");
    }

    #[test]
    fn keymap_has_a_key_per_keysym_on_the_typing_block() {
        let text = keymap(&[0x61, 0xFF0D]);
        assert!(text.contains("minimum = 8; maximum = 11;"));
        assert!(text.contains("<K2> = 10;"));
        assert!(text.contains("<K3> = 11;"));
        assert!(text.contains("key <K2> { [ U0061 ] };"));
        assert!(text.contains("key <K3> { [ Return ] };"));
    }

    #[test]
    fn no_character_lands_on_a_special_key() {
        // The space of "Das ist ein Test" used to get code 1, which is Esc.
        let codes = key_codes(MAX_KEYS);
        assert_eq!(codes.len(), MAX_KEYS);
        for special in [1, 14, 15, 28, 29, 42, 54, 56, 58, 97, 100, 125] {
            assert!(!codes.contains(&special), "code {special} is a special key");
        }
        assert!(codes.iter().all(|code| SPECIAL_CODES.iter().all(|r| !r.contains(code))));
    }

    #[test]
    fn an_ordinary_text_stays_on_the_typing_block() {
        let codes = key_codes(48);
        assert!(codes
            .iter()
            .all(|code| TEXT_CODES.iter().any(|range| range.contains(code))));
        // Volume, power, menu and browser keys are never used either.
        for effect in [113, 116, 139, 142, 158, 163, 172] {
            assert!(
                !key_codes(MAX_KEYS).contains(&effect),
                "code {effect} has an effect of its own"
            );
        }
    }

    #[test]
    fn key_codes_are_distinct_and_fit_a_keymap() {
        let codes = key_codes(MAX_KEYS);
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
        assert!(codes.iter().all(|&code| (2..=247).contains(&code)));
    }

    #[test]
    fn xkbcommon_compiles_the_keymap_and_finds_the_space_on_a_text_key() {
        use xkbcommon::xkb;
        let mut keysyms: Vec<u32> = (0x21..0x21 + MAX_KEYS as u32 - 1).collect();
        keysyms.push(0x20);
        keysyms.sort_unstable();
        let text = keymap(&keysyms);
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let map =
            xkb::Keymap::new_from_string(&context, text, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)
                .expect("keymap compiles");
        let state = xkb::State::new(&map);
        let space = key_codes(keysyms.len())[0];
        assert_eq!(state.key_get_one_sym((space + 8).into()).raw(), 0x20);
    }
}
