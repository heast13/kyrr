//! Whether the desktop shows tray icons. KDE, Hyprland bars and most others
//! do; GNOME only with an extension. Without a tray kyrr's window is the
//! only way to reach it, so the app must not hide it out of reach.

use futures_lite::future;

/// True when a StatusNotifierWatcher (the tray host's registry) is on the
/// session bus.
pub fn available() -> bool {
    future::block_on(async {
        let Ok(connection) = zbus::Connection::session().await else {
            return false;
        };
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&connection).await else {
            return false;
        };
        let Ok(name) = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher") else {
            return false;
        };
        dbus.name_has_owner(name).await.unwrap_or(false)
    })
}
