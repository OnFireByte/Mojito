//! Physical key identities. Everything in Mojito keys off physical position;
//! no layout-dependent character ever crosses the engine.

/// CGEvent virtual keycode: the physical position of a key, stable across
/// keyboard layouts and input languages. Built only from CGEvents (backend)
/// or config parsing (boundary), never from a character.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, serde::Serialize, serde::Deserialize,
)]
pub struct PhysicalKey(pub u16);

/// HID usage id in the keyboard page (the hidutil numbering space).
/// Deliberately a distinct type from [`PhysicalKey`]: Caps Lock is usage 0x39
/// and virtual keycode 0x39 by coincidence, but F18 is usage 0x6D and virtual
/// keycode 0x4F, and conflating the spaces breaks remaps silently.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HidUsage(pub u64);

/// Masked CGEvent modifier flags, opaque outside the platform backend.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ModifierFlags(pub u64);

impl ModifierFlags {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 17);
    pub const CTRL: Self = Self(1 << 18);
    pub const OPT: Self = Self(1 << 19);
    pub const CMD: Self = Self(1 << 20);

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub fn keep_only_shift(self) -> Self {
        Self(self.0 & Self::SHIFT.0)
    }
}

/// Virtual keycodes verified against the SDK's HIToolbox/Events.h.
pub const CAPS: PhysicalKey = PhysicalKey(0x39);
pub const F18: PhysicalKey = PhysicalKey(0x4F);
pub const ESC: PhysicalKey = PhysicalKey(0x35);
pub const RETURN: PhysicalKey = PhysicalKey(0x24);
pub const BACKSPACE: PhysicalKey = PhysicalKey(0x33);
pub const ARROW_LEFT: PhysicalKey = PhysicalKey(0x7B);
pub const ARROW_RIGHT: PhysicalKey = PhysicalKey(0x7C);
pub const ARROW_DOWN: PhysicalKey = PhysicalKey(0x7D);
pub const ARROW_UP: PhysicalKey = PhysicalKey(0x7E);
pub const M: PhysicalKey = PhysicalKey(0x2E);

/// HID usages for the hidutil remap. Apple TN2450 encodes a full usage as
/// (usage_page << 32) | usage, so keyboard-page values are 36-bit: caps lock
/// is 0x700000039 and F18 is 0x70000006D. u64 on purpose.
pub const CAPS_USAGE: HidUsage = HidUsage(0x7_0000_0039);
pub const F18_USAGE: HidUsage = HidUsage(0x7_0000_006D);

/// Reserved control keycodes: unassigned on macOS keyboards (hex's
/// chord-breaker hole and its neighbor). The UI posts unmarked synthetic
/// keyDowns here to reach the engine, and the tap swallows them.
pub const CONTROL: PhysicalKey = PhysicalKey(0x44);
/// Posts "the settings file changed": the tap turns it into a config reload.
pub const CONTROL_RELOAD: PhysicalKey = PhysicalKey(0x46);

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
        0x53..=0x5C => char::from(b'0' + (key.0 - 0x53) as u8),
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

    /// The trap the architecture doc warns about: keycode 0x4F and HID usage
    /// 0x70000006D both name F18, in different spaces. The TN2450 encoding is
    /// (usage_page << 32) | usage, so the usage constants are 36-bit.
    #[test]
    fn keycode_spaces_are_distinct_and_golden() {
        assert_eq!(F18, PhysicalKey(0x4F));
        assert_eq!(CAPS, PhysicalKey(0x39));
        assert_eq!(F18_USAGE, HidUsage(0x7_0000_006D));
        assert_eq!(CAPS_USAGE, HidUsage(0x7_0000_0039));
        assert_eq!(F18_USAGE.0, 30064771181);
        assert_eq!(CAPS_USAGE.0, 30064771129);
        assert_ne!(F18.0 as u64, F18_USAGE.0);
        assert_ne!(CAPS.0 as u64, CAPS_USAGE.0);
    }

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
        // F18 has no printable US-ANSI glyph; the table stays total.
        assert_eq!(glyph(F18), '·');
    }
}
