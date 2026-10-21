//! The connection to the Wayland compositor for typing and the dictation
//! wave. In a Flatpak the session's socket carries a security context, and
//! Hyprland, sway and other wlroots compositors offer neither the virtual
//! keyboard nor layer surfaces on it. kyrr's lines in the compositor's
//! config (see [`crate::binds`]) have the compositor put a hard link to its
//! own socket into the app's runtime folder, which the sandbox shares with
//! the host: that one comes first.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use wayland_client::Connection;

/// The compositor's own socket, as linked into the app's runtime folder;
/// None outside a Flatpak.
pub fn linked_socket() -> Option<PathBuf> {
    let id = std::env::var_os("FLATPAK_ID")?;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(dir).join("app").join(id).join(crate::binds::WAYLAND_LINK))
}

/// The compositor's own socket where it was linked and is still alive (a
/// link from before a restart of the compositor is not), else the session's.
pub fn connect() -> Option<Connection> {
    linked_socket()
        .and_then(|path| UnixStream::connect(path).ok())
        .and_then(|stream| Connection::from_socket(stream).ok())
        .or_else(|| Connection::connect_to_env().ok())
}
