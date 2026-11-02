use mojito_core::ModifierFlags;
use objc2_core_graphics::CGEventFlags;

const PAIRS: [(ModifierFlags, CGEventFlags); 4] = [
    (ModifierFlags::SHIFT, CGEventFlags::MaskShift),
    (ModifierFlags::CTRL, CGEventFlags::MaskControl),
    (ModifierFlags::OPT, CGEventFlags::MaskAlternate),
    (ModifierFlags::CMD, CGEventFlags::MaskCommand),
];

pub(crate) fn from_cg(cg: CGEventFlags) -> ModifierFlags {
    PAIRS
        .iter()
        .filter(|(_, mask)| cg.0 & mask.0 != 0)
        .fold(ModifierFlags::NONE, |acc, (flag, _)| {
            ModifierFlags(acc.0 | flag.0)
        })
}

pub(crate) fn to_cg(flags: ModifierFlags) -> CGEventFlags {
    PAIRS
        .iter()
        .filter(|(flag, _)| flags.intersects(*flag))
        .fold(CGEventFlags(0), |acc, (_, mask)| {
            CGEventFlags(acc.0 | mask.0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_modifier_bits_are_dropped() {
        let cg = CGEventFlags(
            CGEventFlags::MaskCommand.0
                | CGEventFlags::MaskShift.0
                | CGEventFlags::MaskAlphaShift.0
                | CGEventFlags::MaskNumericPad.0
                | CGEventFlags::MaskNonCoalesced.0,
        );
        assert_eq!(
            from_cg(cg),
            ModifierFlags(ModifierFlags::CMD.0 | ModifierFlags::SHIFT.0)
        );
    }

    #[test]
    fn every_modifier_round_trips_to_its_cg_bit() {
        for (flag, mask) in PAIRS {
            assert_eq!(to_cg(flag), mask);
            assert_eq!(from_cg(mask), flag);
        }
    }
}
