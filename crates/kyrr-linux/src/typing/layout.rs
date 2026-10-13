//! The keysyms the user's keyboard layout can type. GNOME and KDE type a
//! keysym from the RemoteDesktop portal by finding it on that layout and
//! silently drop the ones they cannot find, so dictated "Straße" would come
//! out as "Strae" on a US layout.

use std::collections::HashSet;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_keyboard::{self, KeymapFormat, WlKeyboard};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use xkbcommon::xkb;

/// Every keysym on any key, level and layout of the current keymap. None
/// when the compositor did not hand one out.
pub fn keysyms() -> Option<HashSet<u32>> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection).ok()?;
    let handle = queue.handle();
    let seat: WlSeat = globals.bind(&handle, 1..=7, ()).ok()?;
    let keyboard = seat.get_keyboard(&handle, ());
    let mut state = State { keymap: None };
    queue.roundtrip(&mut state).ok()?;
    queue.roundtrip(&mut state).ok()?;
    if keyboard.version() >= 3 {
        keyboard.release();
    }
    let text = state.keymap?;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap =
        xkb::Keymap::new_from_string(&context, text, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)?;
    let mut found = HashSet::new();
    keymap.key_for_each(|keymap, key| {
        for layout in 0..keymap.num_layouts_for_key(key) {
            for level in 0..keymap.num_levels_for_key(key, layout) {
                found.extend(keymap.key_get_syms_by_level(key, layout, level).iter().map(|s| s.raw()));
            }
        }
    });
    Some(found)
}

struct State {
    keymap: Option<String>,
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut State,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_keyboard::Event::Keymap {
            format: WEnum::Value(KeymapFormat::XkbV1),
            fd,
            size,
        } = event
        {
            state.keymap = read(fd, size as usize);
        }
    }
}

/// The keymap text, without its closing NUL. Read from the start whatever
/// the file's offset: the compositor may hand every client the same file.
fn read(fd: OwnedFd, size: usize) -> Option<String> {
    let mut bytes = vec![0; size];
    std::fs::File::from(fd).read_exact_at(&mut bytes, 0).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    Some(text.trim_end_matches('\0').to_string())
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut State,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(_: &mut State, _: &WlSeat, _: <WlSeat as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {}
}
