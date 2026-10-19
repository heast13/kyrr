//! The dictation wave on the screen (see `kyrr_common::wave`). On Wayland it
//! is a layer surface (Hyprland, KDE, Sway and other wlroots compositors);
//! GNOME has no layer surfaces, there and on X11 it is an X window outside
//! the window manager's control. Either way it takes no keyboard focus and
//! lets clicks through, so the text still goes where the user was typing.

mod wayland;
mod x11;

use std::sync::mpsc;
use std::time::Duration;

use ashpd::desktop::settings::{ColorScheme, Settings};
use futures_lite::future;
use kyrr_common::wave::Surface;

/// Whether the desktop is set to dark (asked of the settings portal). Dark when the desktop does not say: it is not worth
/// waiting for, so a slow answer counts as no answer.
fn dark() -> bool {
    let (tx, rx) = mpsc::channel();
    let asked = std::thread::Builder::new().name("kyrr-scheme".into()).spawn(move || {
        let scheme = future::block_on(async { Settings::new().await?.color_scheme().await });
        let _ = tx.send(scheme.ok());
    });
    if asked.is_err() {
        return true;
    }
    !matches!(
        rx.recv_timeout(Duration::from_millis(300)),
        Ok(Some(ColorScheme::PreferLight))
    )
}

/// None where none of this works (GNOME without X11 support, say).
pub fn open() -> Option<Box<dyn Surface>> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        if let Some(layer) = wayland::Layer::open() {
            return Some(Box::new(layer));
        }
    }
    x11::Popup::open().map(|popup| Box::new(popup) as Box<dyn Surface>)
}
