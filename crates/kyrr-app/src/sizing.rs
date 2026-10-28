//! How big the windows get: the rules, free of any window, so the layout
//! check (examples/layout.rs) includes this very file instead of a copy.
//! Lengths are logical pixels unless it says physical.

/// Left, top, right and bottom of the free screen a window is on, in
/// physical pixels.
pub type Area = (i32, i32, i32, i32);

/// Tallest the window gets, screen permitting; more content scrolls. Room
/// for the tallest settings page (Setup with an update note, in German about
/// 1420).
pub const MAX_HEIGHT: f32 = 1460.0;
/// Tallest when the screen is unknown: fits a 768-pixel-high laptop screen
/// with the taskbar and title bar.
pub const FALLBACK_HEIGHT: f32 = 660.0;
/// Never smaller than this, however small the screen.
pub const MIN_HEIGHT: f32 = 400.0;
/// Title bar and borders, about, plus a little air.
pub const FRAME: f32 = 40.0;
/// Borders left and right, about.
pub const FRAME_SIDES: f32 = 16.0;
/// The recordings page is this tall at least (screen permitting): a list
/// and a text to read want room.
pub const MEMOS_HEIGHT: f32 = 780.0;
/// Room for a memo's text: the window grows as tall as the screen allows
/// (up to a comfortable reading height) while it is open.
pub const SHEET_HEIGHT: f32 = 820.0;
/// What a panel (GNOME's bar, KDE's panel) takes off the screen on Linux. A
/// Wayland app cannot learn where the panels are; this much is kept free.
pub const LINUX_PANEL: f32 = 48.0;

/// The panel to keep free: Windows tells the free screen itself (the taskbar
/// is not in it), Linux does not.
pub const fn panel(linux: bool) -> f32 {
    if linux {
        LINUX_PANEL
    } else {
        0.0
    }
}

/// The tallest a window gets on the screen it is on, where `kept_free` is
/// left for panels (see [`panel`]).
pub fn height_cap(area: Option<Area>, scale: f32, kept_free: f32) -> f32 {
    area.map_or(FALLBACK_HEIGHT, |(_, top, _, bottom)| {
        ((bottom - top) as f32 / scale - kept_free - FRAME).clamp(MIN_HEIGHT, MAX_HEIGHT)
    })
}

/// The width the screen has room for, at least 520.
pub fn widest(area: Area, scale: f32) -> f32 {
    ((area.2 - area.0) as f32 / scale - FRAME_SIDES).max(520.0)
}

/// The height the content wants, up to what the screen has room for. The
/// recordings page wants more than its content.
pub fn wanted_height(content: f32, cap: f32, memos: bool) -> f32 {
    (if memos { content.max(MEMOS_HEIGHT) } else { content }).min(cap)
}

/// The height while a memo's text is open.
pub fn sheet_height(cap: f32) -> f32 {
    SHEET_HEIGHT.min(cap)
}

/// The news window's width, which is its preferred one in app.slint.
pub const NEWS_WIDTH: f32 = 380.0;

/// The news window shows all of its text, up to what the screen has room for.
pub fn news_height(content: f32, cap: f32) -> f32 {
    content.min(cap)
}

/// A height rounded up to whole physical pixels. A window is whole pixels;
/// asked for a fraction, Windows rounds to the nearest and can leave the
/// window a hair shorter than its content (417 px at 125 % is 521.25, the
/// window gets 521), and the page scrolls for nothing.
pub fn whole_pixels(height: f32, scale: f32) -> f32 {
    // A little slack for the product that lands just above a whole number.
    (height * scale - 0.001).ceil() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_land_on_whole_physical_pixels() {
        // 417 px at 125 % is 521.25 physical px: up to 522, never down to 521.
        let h = whole_pixels(417.0, 1.25);
        assert!(h * 1.25 >= 521.99 && h * 1.25 < 523.0 && h >= 417.0);
        // Already whole: stays.
        assert_eq!(whole_pixels(400.0, 1.0), 400.0);
        assert!((whole_pixels(300.0, 1.5) - 300.0).abs() < 0.001);
    }

    #[test]
    fn linux_keeps_room_for_panels() {
        let screen = Some((0, 0, 1920, 1080));
        let linux = height_cap(screen, 1.0, panel(true));
        let windows = height_cap(screen, 1.0, panel(false));
        assert_eq!(windows - linux, LINUX_PANEL);
        assert_eq!(linux, 1080.0 - LINUX_PANEL - FRAME);
    }

    #[test]
    fn cap_stays_within_its_limits() {
        assert_eq!(height_cap(None, 1.0, 0.0), FALLBACK_HEIGHT);
        assert_eq!(height_cap(Some((0, 0, 800, 300)), 1.0, 0.0), MIN_HEIGHT);
        assert_eq!(height_cap(Some((0, 0, 3840, 2160)), 1.0, 0.0), MAX_HEIGHT);
    }
}
