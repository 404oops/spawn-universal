// SPDX-License-Identifier: GPL-3.0-or-later
//
// Physical layout of the SPAWN 65% board.
//
// `slot` is the index the firmware uses to address a key in every table
// (keymap, rapid trigger, per-key LED). Widths are in standard keyboard
// units, so every row sums to 16.0u.

/// One physical key.
#[derive(Debug, Clone, Copy)]
pub struct KeyCap {
    /// Index into the firmware's 126-slot tables.
    pub slot: u8,
    /// Legend shown in the UI.
    pub label: &'static str,
    /// Width in keyboard units (1.0 = one standard key).
    pub width: f32,
    /// Default HID Keyboard/Keypad usage ID for this position.
    pub usage: u8,
}

const fn k(slot: u8, label: &'static str, width: f32, usage: u8) -> KeyCap {
    KeyCap {
        slot,
        label,
        width,
        usage,
    }
}

/// The SPAWN's 68 keys in five rows.
pub const SPAWN_ROWS: &[&[KeyCap]] = &[
    &[
        k(0, "Esc", 1.0, 0x29),
        k(17, "1", 1.0, 0x1E),
        k(18, "2", 1.0, 0x1F),
        k(19, "3", 1.0, 0x20),
        k(20, "4", 1.0, 0x21),
        k(21, "5", 1.0, 0x22),
        k(22, "6", 1.0, 0x23),
        k(23, "7", 1.0, 0x24),
        k(24, "8", 1.0, 0x25),
        k(25, "9", 1.0, 0x26),
        k(26, "0", 1.0, 0x27),
        k(27, "-", 1.0, 0x2D),
        k(28, "=", 1.0, 0x2E),
        k(92, "Backspace", 2.0, 0x2A),
        k(103, "Ins", 1.0, 0x49),
    ],
    &[
        k(32, "Tab", 1.5, 0x2B),
        k(33, "Q", 1.0, 0x14),
        k(34, "W", 1.0, 0x1A),
        k(35, "E", 1.0, 0x08),
        k(36, "R", 1.0, 0x15),
        k(37, "T", 1.0, 0x17),
        k(38, "Y", 1.0, 0x1C),
        k(39, "U", 1.0, 0x18),
        k(40, "I", 1.0, 0x0C),
        k(41, "O", 1.0, 0x12),
        k(42, "P", 1.0, 0x13),
        k(43, "[", 1.0, 0x2F),
        k(44, "]", 1.0, 0x30),
        k(60, "\\", 1.5, 0x31),
        k(106, "Del", 1.0, 0x4C),
    ],
    &[
        k(48, "Caps", 1.75, 0x39),
        k(49, "A", 1.0, 0x04),
        k(50, "S", 1.0, 0x16),
        k(51, "D", 1.0, 0x07),
        k(52, "F", 1.0, 0x09),
        k(53, "G", 1.0, 0x0A),
        k(54, "H", 1.0, 0x0B),
        k(55, "J", 1.0, 0x0D),
        k(56, "K", 1.0, 0x0E),
        k(57, "L", 1.0, 0x0F),
        k(58, ";", 1.0, 0x33),
        k(59, "'", 1.0, 0x34),
        k(76, "Enter", 2.25, 0x28),
        k(105, "PgUp", 1.0, 0x4B),
    ],
    &[
        k(64, "Shift", 2.25, 0xE1),
        k(65, "Z", 1.0, 0x1D),
        k(66, "X", 1.0, 0x1B),
        k(67, "C", 1.0, 0x06),
        k(68, "V", 1.0, 0x19),
        k(69, "B", 1.0, 0x05),
        k(70, "N", 1.0, 0x11),
        k(71, "M", 1.0, 0x10),
        k(72, ",", 1.0, 0x36),
        k(73, ".", 1.0, 0x37),
        k(74, "/", 1.0, 0x38),
        k(75, "Shift", 1.75, 0xE5),
        k(90, "\u{2191}", 1.0, 0x52),
        k(108, "PgDn", 1.0, 0x4E),
    ],
    &[
        k(80, "Ctrl", 1.25, 0xE0),
        k(81, "Win", 1.25, 0xE3),
        k(82, "Alt", 1.25, 0xE2),
        k(83, "Space", 6.25, 0x2C),
        k(84, "Alt", 1.0, 0xE6),
        k(85, "Fn", 1.0, 0x00),
        k(87, "Ctrl", 1.0, 0xE4),
        k(88, "\u{2190}", 1.0, 0x50),
        k(89, "\u{2193}", 1.0, 0x51),
        k(91, "\u{2192}", 1.0, 0x4F),
    ],
];

/// Total width of the board in keyboard units.
pub const BOARD_UNITS: f32 = 16.0;

/// Every key on the board, row order.
pub fn all_keys() -> impl Iterator<Item = &'static KeyCap> {
    SPAWN_ROWS.iter().flat_map(|r| r.iter())
}

/// Look up a key by its firmware slot.
pub fn key_by_slot(slot: u8) -> Option<&'static KeyCap> {
    all_keys().find(|k| k.slot == slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_row_is_the_same_width() {
        for (i, row) in SPAWN_ROWS.iter().enumerate() {
            let w: f32 = row.iter().map(|k| k.width).sum();
            assert!(
                (w - BOARD_UNITS).abs() < 1e-4,
                "row {i} is {w}u, expected {BOARD_UNITS}u"
            );
        }
    }

    #[test]
    fn board_has_sixty_eight_keys() {
        assert_eq!(all_keys().count(), 68);
    }

    #[test]
    fn slots_are_unique_and_addressable() {
        let mut seen = HashSet::new();
        for key in all_keys() {
            assert!(seen.insert(key.slot), "duplicate slot {}", key.slot);
            assert!(
                (key.slot as usize) < crate::KEY_SLOTS,
                "slot {} out of range",
                key.slot
            );
        }
    }

    #[test]
    fn lookup_finds_known_slots() {
        assert_eq!(key_by_slot(83).unwrap().label, "Space");
        assert_eq!(key_by_slot(0).unwrap().label, "Esc");
        assert!(key_by_slot(200).is_none());
    }
}
