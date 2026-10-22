//! The connection to the Wayland compositor for typing and the dictation
//! wave.

use wayland_client::Connection;

/// The session's Wayland connection, if there is one.
pub fn connect() -> Option<Connection> {
    Connection::connect_to_env().ok()
}
