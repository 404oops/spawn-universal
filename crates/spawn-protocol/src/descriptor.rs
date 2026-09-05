// SPDX-License-Identifier: GPL-3.0-or-later
//
// Minimal HID report-descriptor scan.
//
// The only thing we need from the descriptor is how many payload bytes an
// output report carries, because that sets the chunk size. A full HID parser
// is overkill: we walk the item stream tracking the two global items that
// matter and read them off at each Output main item.

use crate::DEFAULT_PACKET_LEN;

const ITEM_REPORT_SIZE: u8 = 0x74; // Global, tag 0b0111
const ITEM_REPORT_COUNT: u8 = 0x94; // Global, tag 0b1001
const ITEM_OUTPUT: u8 = 0x90; // Main, tag 0b1001
const ITEM_LONG: u8 = 0xFE;

/// Size in bytes of the largest output report in `desc`.
///
/// Returns `None` when the descriptor declares no output report, so the
/// caller can decide on a fallback.
pub fn output_report_len(desc: &[u8]) -> Option<usize> {
    let mut report_size = 0u32;
    let mut report_count = 0u32;
    let mut best: Option<usize> = None;

    let mut i = 0usize;
    while i < desc.len() {
        let prefix = desc[i];
        i += 1;

        if prefix == ITEM_LONG {
            // Long items: [0xFE][dataSize][tag][data...]
            if i >= desc.len() {
                break;
            }
            let data_size = desc[i] as usize;
            i = i.saturating_add(1 + 1 + data_size);
            continue;
        }

        // Short item: low two bits give the data length (3 means 4 bytes).
        let len = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if i + len > desc.len() {
            break;
        }
        let mut value = 0u32;
        for (n, b) in desc[i..i + len].iter().enumerate() {
            value |= (*b as u32) << (8 * n);
        }
        i += len;

        match prefix & 0xFC {
            ITEM_REPORT_SIZE => report_size = value,
            ITEM_REPORT_COUNT => report_count = value,
            ITEM_OUTPUT => {
                let bits = report_size.saturating_mul(report_count);
                let bytes = (bits / 8) as usize;
                if bytes > 0 {
                    best = Some(best.map_or(bytes, |b: usize| b.max(bytes)));
                }
            }
            _ => {}
        }
    }
    best
}

/// Output report length, falling back to the 32-byte default.
pub fn output_report_len_or_default(desc: &[u8]) -> usize {
    match output_report_len(desc) {
        // Only 32 and 64 are plausible for this family; anything else is a
        // misparse or a different collection, so keep the safe default.
        Some(n @ (32 | 64)) => n,
        _ => DEFAULT_PACKET_LEN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vendor collection declaring 32-byte input and output reports.
    fn desc_32() -> Vec<u8> {
        vec![
            0x06, 0x67, 0xFF, // Usage Page (0xFF67)
            0x09, 0x01, // Usage (1)
            0xA1, 0x01, // Collection (Application)
            0x09, 0x02, //   Usage (2)
            0x15, 0x00, //   Logical Min (0)
            0x26, 0xFF, 0x00, //   Logical Max (255)
            0x75, 0x08, //   Report Size (8)
            0x95, 0x20, //   Report Count (32)
            0x81, 0x02, //   Input
            0x09, 0x03, //   Usage (3)
            0x75, 0x08, //   Report Size (8)
            0x95, 0x20, //   Report Count (32)
            0x91, 0x02, //   Output
            0xC0, // End Collection
        ]
    }

    #[test]
    fn reads_thirty_two_byte_output() {
        assert_eq!(output_report_len(&desc_32()), Some(32));
        assert_eq!(output_report_len_or_default(&desc_32()), 32);
    }

    #[test]
    fn reads_sixty_four_byte_output() {
        let mut d = desc_32();
        // Bump only the output report count (the second 0x95 item).
        let pos = d
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0] == 0x95)
            .nth(1)
            .map(|(i, _)| i + 1)
            .unwrap();
        d[pos] = 0x40;
        assert_eq!(output_report_len(&d), Some(64));
    }

    #[test]
    fn descriptor_without_output_yields_none() {
        let d = vec![
            0x06, 0x67, 0xFF, 0x09, 0x01, 0xA1, 0x01, 0x75, 0x08, 0x95, 0x20, 0x81, 0x02, 0xC0,
        ];
        assert_eq!(output_report_len(&d), None);
        assert_eq!(output_report_len_or_default(&d), DEFAULT_PACKET_LEN);
    }

    #[test]
    fn implausible_size_falls_back() {
        // 8-byte boot keyboard output report: not our vendor collection.
        let d = vec![0x75, 0x08, 0x95, 0x08, 0x91, 0x02];
        assert_eq!(output_report_len(&d), Some(8));
        assert_eq!(output_report_len_or_default(&d), DEFAULT_PACKET_LEN);
    }

    #[test]
    fn truncated_descriptor_does_not_panic() {
        for cut in 0..desc_32().len() {
            let _ = output_report_len(&desc_32()[..cut]);
        }
    }

    #[test]
    fn long_items_are_skipped() {
        let mut d = vec![0xFE, 0x02, 0x00, 0xAA, 0xBB];
        d.extend_from_slice(&desc_32());
        assert_eq!(output_report_len(&d), Some(32));
    }
}
