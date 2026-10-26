//! The keys dictation and voice memos can use, one table for every part:
//! Windows virtual-key code (what the settings file holds), X11 keysym, and
//! the name the shortcut portal and the tiling compositors know it by.
//!
//! Single keys only, and none a program needs for typing: letters, digits,
//! Space, Enter, Escape, arrows and the modifiers (right Ctrl apart) stay
//! with the apps, because the key is taken from every program while kyrr
//! listens for it.

pub const F9: u32 = 0x78;
pub const F8: u32 = 0x77;
pub const PAUSE: u32 = 0x13;
pub const SCROLL_LOCK: u32 = 0x91;
pub const RIGHT_CTRL: u32 = 0xA3;
pub const MENU: u32 = 0x5D;

/// The standard key for dictation.
pub const DICTATION_DEFAULT: u32 = F9;
/// The standard key for voice memos.
pub const MEMO_DEFAULT: u32 = F8;

/// Function keys F1 to F24 (virtual-key codes 0x70 to 0x87).
const FIRST_F: u32 = 0x70;
const LAST_F: u32 = 0x87;

/// Whether kyrr offers the key at all.
pub fn allowed(vk: u32) -> bool {
    (FIRST_F..=LAST_F).contains(&vk) || matches!(vk, PAUSE | SCROLL_LOCK | RIGHT_CTRL | MENU)
}

/// The X11 keysym of an allowed key.
pub fn keysym(vk: u32) -> Option<u32> {
    match vk {
        FIRST_F..=LAST_F => Some(0xFFBE + (vk - FIRST_F)),
        PAUSE => Some(0xFF13),
        SCROLL_LOCK => Some(0xFF14),
        RIGHT_CTRL => Some(0xFFE4),
        MENU => Some(0xFF67),
        _ => None,
    }
}

/// The name of an allowed key for the shortcut portal and the compositors'
/// configs (X keysym names): "F9", "Pause", "Control_R".
pub fn xkb_name(vk: u32) -> Option<String> {
    match vk {
        FIRST_F..=LAST_F => Some(format!("F{}", vk - FIRST_F + 1)),
        PAUSE => Some("Pause".into()),
        SCROLL_LOCK => Some("Scroll_Lock".into()),
        RIGHT_CTRL => Some("Control_R".into()),
        MENU => Some("Menu".into()),
        _ => None,
    }
}

/// The key an X keysym name stands for, the other way round from
/// [`xkb_name`]: allowed keys only.
pub fn from_xkb_name(name: &str) -> Option<u32> {
    let vk = match name {
        "Pause" => PAUSE,
        "Scroll_Lock" => SCROLL_LOCK,
        "Control_R" => RIGHT_CTRL,
        "Menu" => MENU,
        _ => FIRST_F + name.strip_prefix('F')?.parse::<u32>().ok()?.checked_sub(1)?,
    };
    allowed(vk).then_some(vk)
}

/// What the key is called on screen. `german` for the German names.
pub fn label(vk: u32, german: bool) -> String {
    match vk {
        FIRST_F..=LAST_F => format!("F{}", vk - FIRST_F + 1),
        PAUSE => "Pause".into(),
        SCROLL_LOCK if german => "Rollen".into(),
        SCROLL_LOCK => "Scroll Lock".into(),
        RIGHT_CTRL if german => "Strg rechts".into(),
        RIGHT_CTRL => "Right Ctrl".into(),
        MENU if german => "Menü".into(),
        MENU => "Menu".into(),
        _ => String::new(),
    }
}

/// Keys many programs use themselves (help, rename, reload, full screen,
/// the developer tools): allowed, with a word of warning.
pub fn busy(vk: u32) -> bool {
    matches!(vk.checked_sub(FIRST_F), Some(0..=6 | 9..=11)) // F1 to F7, F10 to F12
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_both_ways() {
        for vk in (FIRST_F..=LAST_F).chain([PAUSE, SCROLL_LOCK, RIGHT_CTRL, MENU]) {
            assert_eq!(from_xkb_name(&xkb_name(vk).unwrap()), Some(vk));
        }
        assert_eq!(from_xkb_name("F25"), None);
        assert_eq!(from_xkb_name("F0"), None);
        assert_eq!(from_xkb_name("a"), None);
    }

    #[test]
    fn the_tables_agree() {
        for vk in 0..0x100 {
            assert_eq!(allowed(vk), keysym(vk).is_some(), "{vk:#x}");
            assert_eq!(allowed(vk), xkb_name(vk).is_some(), "{vk:#x}");
            assert_eq!(allowed(vk), !label(vk, false).is_empty(), "{vk:#x}");
        }
    }

    #[test]
    fn names_and_keysyms() {
        assert_eq!(xkb_name(F9).as_deref(), Some("F9"));
        assert_eq!(keysym(F9), Some(0xFFC6));
        assert_eq!(keysym(F8), Some(0xFFC5));
        assert_eq!(xkb_name(0x87).as_deref(), Some("F24"));
        assert_eq!(keysym(0x87), Some(0xFFD5));
        assert_eq!(xkb_name(RIGHT_CTRL).as_deref(), Some("Control_R"));
    }

    #[test]
    fn typing_keys_are_not_offered() {
        // A, Space, Enter, Escape, Left Ctrl, Shift, Alt, Windows, left arrow.
        for vk in [0x41, 0x20, 0x0D, 0x1B, 0xA2, 0x10, 0x12, 0x5B, 0x25] {
            assert!(!allowed(vk), "{vk:#x}");
        }
    }

    #[test]
    fn busy_keys() {
        assert!(busy(0x74)); // F5
        assert!(busy(0x7A)); // F11
        assert!(!busy(F8));
        assert!(!busy(F9));
        assert!(!busy(PAUSE));
    }
}
