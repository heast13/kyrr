//! The dictation wave on the screen (see `kyrr_common::wave`). On Wayland it
//! is a layer surface (Hyprland, KDE, Sway and other wlroots compositors);
//! GNOME has no layer surfaces, there and on X11 it is an X window outside
//! the window manager's control. Either way it takes no keyboard focus and
//! lets clicks through, so the text still goes where the user was typing.

mod wayland;
mod x11;

use kyrr_common::wave::Surface;

/// None where none of this works (GNOME without X11 support, say).
pub fn open() -> Option<Box<dyn Surface>> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        if let Some(layer) = wayland::Layer::open() {
            return Some(Box::new(layer));
        }
    }
    x11::Popup::open().map(|popup| Box::new(popup) as Box<dyn Surface>)
}
