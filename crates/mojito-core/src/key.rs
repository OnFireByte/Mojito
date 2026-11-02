//! Physical key identities. Everything in Mojito keys off physical position;
//! no layout-dependent character ever crosses the engine.

/// CGEvent virtual keycode: the physical position of a key, stable across
/// keyboard layouts and input languages. Built only from CGEvents (backend)
/// or config parsing (boundary), never from a character.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, serde::Serialize)]
pub struct PhysicalKey(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ModifierFlags(pub u8);

impl ModifierFlags {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const CTRL: Self = Self(1 << 1);
    pub const OPT: Self = Self(1 << 2);
    pub const CMD: Self = Self(1 << 3);

    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// Virtual keycodes verified against the SDK's HIToolbox/Events.h.
pub const META: PhysicalKey = PhysicalKey(0x4F);
pub const ESC: PhysicalKey = PhysicalKey(0x35);
pub const RETURN: PhysicalKey = PhysicalKey(0x24);
pub const BACKSPACE: PhysicalKey = PhysicalKey(0x33);
pub const ARROW_LEFT: PhysicalKey = PhysicalKey(0x7B);
pub const ARROW_RIGHT: PhysicalKey = PhysicalKey(0x7C);
pub const ARROW_DOWN: PhysicalKey = PhysicalKey(0x7D);
pub const ARROW_UP: PhysicalKey = PhysicalKey(0x7E);
pub const SWITCHER: PhysicalKey = PhysicalKey(0x0E);
pub const TAB: PhysicalKey = PhysicalKey(0x30);

/// US-ANSI position to glyph, for badges and labels only. Never used to
/// interpret a keystroke. Positions without a printable glyph (F-keys,
/// modifiers) get the neutral middle dot; the platform backend overlays the
/// real layout glyph where it has one.
pub fn glyph(key: PhysicalKey) -> char {
    match key.0 {
        0x00 => 'A',
        0x01 => 'S',
        0x02 => 'D',
        0x03 => 'F',
        0x04 => 'H',
        0x05 => 'G',
        0x06 => 'Z',
        0x07 => 'X',
        0x08 => 'C',
        0x09 => 'V',
        0x0A => '§',
        0x0B => 'B',
        0x0C => 'Q',
        0x0D => 'W',
        0x0E => 'E',
        0x0F => 'R',
        0x10 => 'Y',
        0x11 => 'T',
        0x12 => '1',
        0x13 => '2',
        0x14 => '3',
        0x15 => '4',
        0x16 => '6',
        0x17 => '5',
        0x18 => '=',
        0x19 => '9',
        0x1A => '7',
        0x1B => '-',
        0x1C => '8',
        0x1D => '0',
        0x1E => ']',
        0x1F => 'O',
        0x20 => 'U',
        0x21 => '[',
        0x22 => 'I',
        0x23 => 'P',
        0x24 => '⏎',
        0x25 => 'L',
        0x26 => 'J',
        0x27 => '\'',
        0x28 => 'K',
        0x29 => ';',
        0x2A => '\\',
        0x2B => ',',
        0x2C => '/',
        0x2D => 'N',
        0x2E => 'M',
        0x2F => '.',
        0x30 => '⇥',
        0x31 => '␣',
        0x32 => '`',
        0x33 => '⌫',
        0x35 => '⎋',
        0x39 => '⇪',
        0x41 => '.',
        0x43 => '*',
        0x45 => '+',
        0x47 => '⌧',
        0x4B => '/',
        0x4C => '⏎',
        0x4E => '-',
        0x51 => '=',
        0x52 => '0',
        0x53..=0x59 => char::from(b'1' + (key.0 - 0x53) as u8),
        0x5B => '8',
        0x5C => '9',
        0x7B => '←',
        0x7C => '→',
        0x7D => '↓',
        0x7E => '↑',
        _ => '·',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_follow_us_ansi_positions() {
        assert_eq!(glyph(PhysicalKey(0x00)), 'A');
        assert_eq!(glyph(PhysicalKey(0x04)), 'H');
        assert_eq!(glyph(PhysicalKey(0x2E)), 'M');
        assert_eq!(glyph(PhysicalKey(0x12)), '1');
        assert_eq!(glyph(PhysicalKey(0x1D)), '0');
        assert_eq!(glyph(PhysicalKey(0x29)), ';');
        assert_eq!(glyph(PhysicalKey(0x31)), '␣');
        assert_eq!(glyph(ARROW_LEFT), '←');
        assert_eq!(glyph(ARROW_UP), '↑');
        assert_eq!(glyph(META), '·');
    }

    #[test]
    fn keypad_digits_follow_events_h_and_skip_f20() {
        let keypad = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
        for (digit, code) in ('0'..='9').zip(keypad) {
            assert_eq!(
                glyph(PhysicalKey(code)),
                digit,
                "keypad {digit} at {code:#x}"
            );
        }
        assert!(!glyph(PhysicalKey(0x5A)).is_ascii_digit());
    }
}
