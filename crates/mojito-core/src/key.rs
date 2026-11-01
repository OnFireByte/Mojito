//! Physical key identities. Everything in Mojito keys off physical position;
//! no layout-dependent character ever crosses the engine.

/// CGEvent virtual keycode: the physical position of a key, stable across
/// keyboard layouts and input languages. Built only from CGEvents (backend)
/// or config parsing (boundary), never from a character.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PhysicalKey(pub u16);

/// HID usage id in the keyboard page (the hidutil numbering space).
/// Deliberately a distinct type from [`PhysicalKey`]: Caps Lock is usage 0x39
/// and virtual keycode 0x39 by coincidence, but F18 is usage 0x6D and virtual
/// keycode 0x4F, and conflating the spaces breaks remaps silently.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HidUsage(pub u32);

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

/// HID usages for the hidutil remap (keyboard page, 0x700000xx).
pub const CAPS_USAGE: HidUsage = HidUsage(0x7000_0039);
pub const F18_USAGE: HidUsage = HidUsage(0x7000_006D);

/// US-ANSI position to glyph, for badges and labels only. Never used to
/// interpret a keystroke.
pub fn glyph(key: PhysicalKey) -> char {
    let _ = key;
    todo!("key.rs glyph table")
}
