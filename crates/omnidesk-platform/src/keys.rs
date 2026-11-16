//! Decides what to do with a key press that arrives from another device.
//!
//! Keys travel as browser key names (W3C `KeyboardEvent.code` for the physical key and `.key`
//! for what it produces). The two devices can run different systems and keyboard layouts, so:
//!
//! * named keys (Enter, arrows, Shift, F5...) are pressed as that key;
//! * a character typed without Ctrl / Alt / ⌘ is entered as text, so "ψ" arrives as "ψ" whatever
//!   layout the other computer uses;
//! * with Ctrl / Alt / ⌘ held it is a shortcut, pressed by physical key, so Ctrl+C is copy even
//!   when the keyboard is set to Greek.
//!
//! Kept free of OS types so it is tested on every platform; `desktop.rs` turns the result into
//! the input library's calls.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Enter,
    Tab,
    Backspace,
    Escape,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    Space,
    CapsLock,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    Alt,
    Meta,
    F(u8),
    Numpad(u8),
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MediaNext,
    MediaPrev,
    MediaPlayPause,
}

/// Modifier keys currently held down by the controlling device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    /// Reads the modifier state from the physical keys that are down.
    pub fn from_codes<'a>(codes: impl Iterator<Item = &'a str>) -> Mods {
        let mut mods = Mods::default();
        for code in codes {
            match code {
                "ControlLeft" | "ControlRight" => mods.ctrl = true,
                "AltLeft" | "AltRight" => mods.alt = true,
                "MetaLeft" | "MetaRight" | "OSLeft" | "OSRight" => mods.meta = true,
                _ => {}
            }
        }
        mods
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Press (and later release) a named key.
    Named(Named),
    /// Press (and later release) a physical key as part of a shortcut.
    Shortcut(Physical),
    /// Enter a character. Nothing to release.
    Text(char),
    Ignore,
}

/// A letter, digit or punctuation key, identified by its position on the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Physical {
    /// What the key types on a US keyboard, unshifted.
    pub us: char,
    /// Windows virtual-key code.
    pub windows_vk: u16,
    /// macOS virtual key code (`kVK_ANSI_*`).
    pub mac_keycode: u16,
}

const fn phys(us: char, windows_vk: u16, mac_keycode: u16) -> Physical {
    Physical {
        us,
        windows_vk,
        mac_keycode,
    }
}

/// Letter, digit and punctuation keys by W3C `code`.
pub fn physical(code: &str) -> Option<Physical> {
    Some(match code {
        "KeyA" => phys('a', 0x41, 0x00),
        "KeyB" => phys('b', 0x42, 0x0B),
        "KeyC" => phys('c', 0x43, 0x08),
        "KeyD" => phys('d', 0x44, 0x02),
        "KeyE" => phys('e', 0x45, 0x0E),
        "KeyF" => phys('f', 0x46, 0x03),
        "KeyG" => phys('g', 0x47, 0x05),
        "KeyH" => phys('h', 0x48, 0x04),
        "KeyI" => phys('i', 0x49, 0x22),
        "KeyJ" => phys('j', 0x4A, 0x26),
        "KeyK" => phys('k', 0x4B, 0x28),
        "KeyL" => phys('l', 0x4C, 0x25),
        "KeyM" => phys('m', 0x4D, 0x2E),
        "KeyN" => phys('n', 0x4E, 0x2D),
        "KeyO" => phys('o', 0x4F, 0x1F),
        "KeyP" => phys('p', 0x50, 0x23),
        "KeyQ" => phys('q', 0x51, 0x0C),
        "KeyR" => phys('r', 0x52, 0x0F),
        "KeyS" => phys('s', 0x53, 0x01),
        "KeyT" => phys('t', 0x54, 0x11),
        "KeyU" => phys('u', 0x55, 0x20),
        "KeyV" => phys('v', 0x56, 0x09),
        "KeyW" => phys('w', 0x57, 0x0D),
        "KeyX" => phys('x', 0x58, 0x07),
        "KeyY" => phys('y', 0x59, 0x10),
        "KeyZ" => phys('z', 0x5A, 0x06),
        "Digit0" => phys('0', 0x30, 0x1D),
        "Digit1" => phys('1', 0x31, 0x12),
        "Digit2" => phys('2', 0x32, 0x13),
        "Digit3" => phys('3', 0x33, 0x14),
        "Digit4" => phys('4', 0x34, 0x15),
        "Digit5" => phys('5', 0x35, 0x17),
        "Digit6" => phys('6', 0x36, 0x16),
        "Digit7" => phys('7', 0x37, 0x1A),
        "Digit8" => phys('8', 0x38, 0x1C),
        "Digit9" => phys('9', 0x39, 0x19),
        "Minus" => phys('-', 0xBD, 0x1B),
        "Equal" => phys('=', 0xBB, 0x18),
        "BracketLeft" => phys('[', 0xDB, 0x21),
        "BracketRight" => phys(']', 0xDD, 0x1E),
        "Backslash" => phys('\\', 0xDC, 0x2A),
        "Semicolon" => phys(';', 0xBA, 0x29),
        "Quote" => phys('\'', 0xDE, 0x27),
        "Backquote" => phys('`', 0xC0, 0x32),
        "Comma" => phys(',', 0xBC, 0x2B),
        "Period" => phys('.', 0xBE, 0x2F),
        "Slash" => phys('/', 0xBF, 0x2C),
        _ => return None,
    })
}

/// Keys that are pressed as themselves, whatever the layout.
pub fn named(code: &str, key: &str) -> Option<Named> {
    use Named::*;
    let by_code = match code {
        "ShiftLeft" => Some(ShiftLeft),
        "ShiftRight" => Some(ShiftRight),
        "ControlLeft" => Some(ControlLeft),
        "ControlRight" => Some(ControlRight),
        "AltLeft" | "AltRight" => Some(Alt),
        "MetaLeft" | "MetaRight" | "OSLeft" | "OSRight" => Some(Meta),
        "Space" => Some(Space),
        _ => None,
    };
    let by_key = || match key {
        "Enter" => Some(Enter),
        "Tab" => Some(Tab),
        "Backspace" => Some(Backspace),
        "Escape" => Some(Escape),
        "Delete" => Some(Delete),
        "Home" => Some(Home),
        "End" => Some(End),
        "PageUp" => Some(PageUp),
        "PageDown" => Some(PageDown),
        "ArrowLeft" => Some(Left),
        "ArrowRight" => Some(Right),
        "ArrowUp" => Some(Up),
        "ArrowDown" => Some(Down),
        "CapsLock" => Some(CapsLock),
        "Shift" => Some(ShiftLeft),
        "Control" => Some(ControlLeft),
        "Alt" | "AltGraph" => Some(Alt),
        "Meta" | "OS" => Some(Meta),
        "AudioVolumeUp" => Some(VolumeUp),
        "AudioVolumeDown" => Some(VolumeDown),
        "AudioVolumeMute" => Some(VolumeMute),
        "MediaTrackNext" => Some(MediaNext),
        "MediaTrackPrevious" => Some(MediaPrev),
        "MediaPlayPause" => Some(MediaPlayPause),
        _ => None,
    };
    let function_key = || {
        let n = key.strip_prefix('F')?.parse::<u8>().ok()?;
        (key.len() <= 3 && (1..=20).contains(&n)).then_some(F(n))
    };
    let numpad_digit = || {
        let digit = single_char(key)?.to_digit(10)?;
        code.starts_with("Numpad").then_some(Numpad(digit as u8))
    };
    by_code
        .or_else(by_key)
        .or_else(function_key)
        .or_else(numpad_digit)
}

fn single_char(key: &str) -> Option<char> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// What to do when the key with `code` / `key` goes down while `mods` are held.
pub fn decide(code: &str, key: &str, mods: Mods) -> Action {
    if let Some(n) = named(code, key) {
        return Action::Named(n);
    }
    let typed = single_char(key);
    if mods.ctrl || mods.alt || mods.meta {
        if let Some(physical) = physical(code) {
            // Alt (Option on a Mac, AltGr on European keyboards) is also how people type
            // characters such as å or @. Then the key produces something other than its own
            // letter, and that character is what was meant.
            let composed = mods.alt
                && !mods.meta
                && typed.is_some_and(|c| !c.eq_ignore_ascii_case(&physical.us) && !c.is_control());
            return if composed {
                Action::Text(typed.unwrap_or(physical.us))
            } else {
                Action::Shortcut(physical)
            };
        }
    }
    match typed {
        Some(c) if !c.is_control() => Action::Text(c),
        _ => Action::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        meta: false,
    };
    const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        meta: false,
    };
    const ALT: Mods = Mods {
        ctrl: false,
        alt: true,
        meta: false,
    };
    const META: Mods = Mods {
        ctrl: false,
        alt: false,
        meta: true,
    };
    const ALTGR: Mods = Mods {
        ctrl: true,
        alt: true,
        meta: false,
    };

    fn shortcut(action: Action) -> char {
        match action {
            Action::Shortcut(p) => p.us,
            other => panic!("expected a shortcut, got {other:?}"),
        }
    }

    #[test]
    fn typing_is_text_in_any_language() {
        assert_eq!(decide("KeyA", "a", NONE), Action::Text('a'));
        assert_eq!(decide("KeyA", "A", NONE), Action::Text('A'));
        assert_eq!(decide("Digit1", "!", NONE), Action::Text('!'));
        assert_eq!(decide("KeyC", "ψ", NONE), Action::Text('ψ'));
        assert_eq!(decide("KeyE", "é", NONE), Action::Text('é'));
        assert_eq!(decide("IntlBackslash", "<", NONE), Action::Text('<'));
    }

    #[test]
    fn shortcuts_follow_the_physical_key() {
        assert_eq!(shortcut(decide("KeyC", "c", CTRL)), 'c');
        // Greek layout: Ctrl+C reports ψ, and is still copy.
        assert_eq!(shortcut(decide("KeyC", "ψ", CTRL)), 'c');
        assert_eq!(shortcut(decide("KeyV", "v", META)), 'v');
        assert_eq!(
            shortcut(decide(
                "KeyT",
                "T",
                Mods {
                    ctrl: true,
                    alt: false,
                    meta: false
                }
            )),
            't'
        );
        assert_eq!(shortcut(decide("KeyF", "f", ALT)), 'f');
        assert_eq!(shortcut(decide("Digit1", "1", CTRL)), '1');
        assert_eq!(shortcut(decide("Slash", "/", META)), '/');
    }

    #[test]
    fn characters_typed_with_option_or_altgr_stay_characters() {
        // Mac: Option+A types å.
        assert_eq!(decide("KeyA", "å", ALT), Action::Text('å'));
        // German keyboard on Windows: AltGr+Q types @ (the browser reports Ctrl+Alt held).
        assert_eq!(decide("KeyQ", "@", ALTGR), Action::Text('@'));
        // Ctrl+Alt+Q with nothing special on the key is a shortcut.
        assert_eq!(shortcut(decide("KeyQ", "q", ALTGR)), 'q');
    }

    #[test]
    fn named_keys() {
        assert_eq!(decide("Enter", "Enter", NONE), Action::Named(Named::Enter));
        assert_eq!(decide("Space", " ", NONE), Action::Named(Named::Space));
        assert_eq!(decide("Space", " ", CTRL), Action::Named(Named::Space));
        assert_eq!(
            decide("ShiftRight", "Shift", NONE),
            Action::Named(Named::ShiftRight)
        );
        assert_eq!(decide("MetaLeft", "Meta", NONE), Action::Named(Named::Meta));
        assert_eq!(decide("F5", "F5", NONE), Action::Named(Named::F(5)));
        assert_eq!(decide("F12", "F12", CTRL), Action::Named(Named::F(12)));
        assert_eq!(
            decide("Numpad7", "7", NONE),
            Action::Named(Named::Numpad(7))
        );
        assert_eq!(decide("ArrowUp", "ArrowUp", META), Action::Named(Named::Up));
        // The letter F is not the key F1.
        assert_eq!(decide("KeyF", "F", NONE), Action::Text('F'));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        assert_eq!(decide("Unidentified", "Unidentified", NONE), Action::Ignore);
        assert_eq!(decide("IntlRo", "Dead", NONE), Action::Ignore);
        assert_eq!(
            decide("KeyE", "Dead", ALT),
            Action::Shortcut(physical("KeyE").unwrap())
        );
    }

    #[test]
    fn modifier_state_from_held_keys() {
        assert_eq!(Mods::from_codes(["ShiftLeft", "KeyA"].into_iter()), NONE);
        assert_eq!(
            Mods::from_codes(["ControlLeft", "AltRight"].into_iter()),
            ALTGR
        );
        assert_eq!(Mods::from_codes(["OSLeft"].into_iter()), META);
    }

    #[test]
    fn every_physical_key_is_unique() {
        let codes = [
            "KeyA",
            "KeyB",
            "KeyC",
            "KeyD",
            "KeyE",
            "KeyF",
            "KeyG",
            "KeyH",
            "KeyI",
            "KeyJ",
            "KeyK",
            "KeyL",
            "KeyM",
            "KeyN",
            "KeyO",
            "KeyP",
            "KeyQ",
            "KeyR",
            "KeyS",
            "KeyT",
            "KeyU",
            "KeyV",
            "KeyW",
            "KeyX",
            "KeyY",
            "KeyZ",
            "Digit0",
            "Digit1",
            "Digit2",
            "Digit3",
            "Digit4",
            "Digit5",
            "Digit6",
            "Digit7",
            "Digit8",
            "Digit9",
            "Minus",
            "Equal",
            "BracketLeft",
            "BracketRight",
            "Backslash",
            "Semicolon",
            "Quote",
            "Backquote",
            "Comma",
            "Period",
            "Slash",
        ];
        let keys: Vec<Physical> = codes.iter().map(|c| physical(c).unwrap()).collect();
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert!(
                    a.us != b.us && a.windows_vk != b.windows_vk && a.mac_keycode != b.mac_keycode,
                    "{a:?} clashes with {b:?}"
                );
            }
        }
        // Letters and digits use their own ASCII code as the Windows key code.
        for k in keys.iter().filter(|k| k.us.is_ascii_alphanumeric()) {
            assert_eq!(k.windows_vk, k.us.to_ascii_uppercase() as u16);
        }
    }
}
