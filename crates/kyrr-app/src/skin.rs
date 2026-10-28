//! Omarchy's look for kyrr: the theme in use for the window and the
//! dictation wave, looked at again every few seconds, so a change of theme
//! shows right away. Elsewhere kyrr looks like the system.

use std::cell::RefCell;

use kyrr_linux::omarchy::{self, Rgb, Theme};
use slint::{Color, ComponentHandle};

use crate::{MainWindow, Skin};

thread_local! {
    /// The theme last applied (None: kyrr's own look) and its font family.
    static NOW: RefCell<Option<(Option<Theme>, String)>> = const { RefCell::new(None) };
}

/// Looks at the theme again; a change goes to the window and the wave.
pub fn refresh(window: &MainWindow) {
    let theme = omarchy::theme();
    if NOW.with_borrow(|now| now.as_ref().is_some_and(|(seen, _)| *seen == theme)) {
        return;
    }
    // The font only with a new theme: Omarchy sets both in its menu, and
    // asking fontconfig every few seconds is not needed.
    let font = theme.as_ref().and_then(|_| omarchy::mono_font());
    kyrr_common::wave::set_skin(theme.as_ref().map(|t| {
        let file = font.as_ref().and_then(|(_, file)| std::fs::read(file).ok());
        kyrr_common::wave::Skin {
            background: t.background,
            frame: t.border,
            bars: t.accent,
            text: t.foreground,
            yellow: t.yellow,
            red: t.red,
            font: file.and_then(kyrr_common::wave::Skin::font_from),
        }
    }));
    if let Some(theme) = &theme {
        window.invoke_use_scheme(theme.dark);
    }
    let family = font.map(|(family, _)| family).unwrap_or_default();
    NOW.with_borrow_mut(|now| *now = Some((theme, family)));
    apply(&window.global::<Skin>());
}

/// Whether the theme in use is dark (None: kyrr's own look).
pub fn dark() -> Option<bool> {
    NOW.with_borrow(|now| now.as_ref().and_then(|(theme, _)| theme.as_ref().map(|t| t.dark)))
}

/// Gives a window the look in use (each window has its own).
pub fn apply(skin: &Skin<'_>) {
    NOW.with_borrow(|now| {
        let Some((theme, family)) = now else {
            return;
        };
        let Some(t) = theme else {
            skin.set_omarchy(false);
            return;
        };
        let color = |rgb: Rgb| Color::from_rgb_u8(rgb[0], rgb[1], rgb[2]);
        skin.set_theme_background(color(t.background));
        skin.set_theme_foreground(color(t.foreground));
        skin.set_theme_accent(color(t.accent));
        skin.set_theme_red(color(t.red));
        skin.set_theme_yellow(color(t.yellow));
        skin.set_theme_green(color(t.green));
        skin.set_theme_font(family.as_str().into());
        skin.set_omarchy(true);
    });
}
