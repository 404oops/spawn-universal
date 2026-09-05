// SPDX-License-Identifier: GPL-3.0-or-later
//
// Typed views over the firmware's fixed-size records.
//
// Each struct owns its own `decode` / `encode` pair so the byte offsets stay
// next to the field they describe.

use crate::{Cmd, KEY_SLOTS, ProtocolError, Request, Result};

fn le16(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn need(b: &[u8], n: usize) -> Result<()> {
    if b.len() < n {
        return Err(ProtocolError::Truncated {
            got: b.len(),
            want: n,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------- device info

pub const DEVICE_INFO_LEN: usize = 48;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub rom_size: u8,
    pub vid: u16,
    pub pid: u16,
    /// Firmware version, encoded BCD-ish in two bytes; kept as hundredths.
    pub version_centi: u16,
    pub sensor: u16,
    pub manufacturer: u16,
    pub product: u16,
    pub work_mode: u8,
    pub battery_level: u8,
    pub charge_status: u8,
    pub current_profile: u8,
    pub axis_info: u16,
    pub tft_max_frames: u16,
    pub gif_max_frames: u16,
    pub led_max_frames: u16,
    pub tft_direction: u8,
    /// 0 = rapid-trigger values are hundredths of a millimetre,
    /// 1 = thousandths.
    pub rt_precision: u8,
}

impl DeviceInfo {
    pub fn request() -> Request<'static> {
        Request::read(Cmd::GetDeviceInfo, DEVICE_INFO_LEN)
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        need(b, DEVICE_INFO_LEN)?;
        Ok(Self {
            rom_size: b[0],
            vid: le16(b, 4),
            pid: le16(b, 6),
            // low nibble = units, high nibble = tens, next byte = hundreds
            version_centi: ((b[8] & 0x0F) as u16)
                + (((b[8] & 0xF0) >> 4) as u16) * 10
                + (b[9] as u16) * 100,
            sensor: le16(b, 10),
            manufacturer: le16(b, 12),
            product: le16(b, 14),
            work_mode: b[16],
            battery_level: b[17],
            charge_status: b[18],
            current_profile: b[19],
            axis_info: le16(b, 20),
            tft_max_frames: le16(b, 22),
            gif_max_frames: le16(b, 24),
            led_max_frames: le16(b, 26),
            tft_direction: b[28],
            rt_precision: b[29],
        })
    }

    pub fn version_string(&self) -> String {
        format!(
            "{}.{:02}",
            self.version_centi / 100,
            self.version_centi % 100
        )
    }

    /// Scale factor converting millimetres to the firmware's integer units.
    pub fn rt_scale(&self) -> f32 {
        if self.rt_precision == 1 {
            1000.0
        } else {
            100.0
        }
    }
}

// ----------------------------------------------------------------- game mode

pub const GAME_MODE_LEN: usize = 64;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameMode {
    pub game_mode: u8,
    pub fn_switch: u8,
    pub sleep_time: u8,
    pub key_delay: u8,
    /// Index into the board's supported polling rates, not a raw Hz value.
    pub report_rate: u8,
    pub system_mode: u8,
    pub tft_display_time: u8,
    /// Millimetres.
    pub top_dead_zone: f32,
    /// Millimetres.
    pub bottom_dead_zone: f32,
    pub stability_mode: u8,
    pub auto_calibration: u8,
    pub single_key_wakeup: u8,
}

impl GameMode {
    /// What the firmware reports after a full factory reset.
    ///
    /// Measured on `0C45:8A01` firmware 1.09 rather than inferred: the
    /// vendor interface shows its own defaults, which are not always the
    /// board's. Note `report_rate` 6 is the 8000 Hz code, and both dead
    /// zones come back at zero.
    pub fn factory_default() -> Self {
        Self {
            game_mode: 0,
            fn_switch: 0,
            sleep_time: 1,
            key_delay: 0,
            report_rate: 6,
            system_mode: 0,
            tft_display_time: 0,
            top_dead_zone: 0.0,
            bottom_dead_zone: 0.0,
            stability_mode: 1,
            auto_calibration: 1,
            single_key_wakeup: 0,
        }
    }

    pub fn request() -> Request<'static> {
        Request::read(Cmd::GetGameMode, GAME_MODE_LEN)
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        need(b, 16)?;
        Ok(Self {
            game_mode: b[1],
            fn_switch: b[2],
            sleep_time: b[3],
            key_delay: b[4],
            report_rate: b[5],
            system_mode: b[6],
            tft_display_time: b[7],
            top_dead_zone: b[8] as f32 / 100.0,
            bottom_dead_zone: b[9] as f32 / 100.0,
            stability_mode: b[11],
            auto_calibration: b[14],
            single_key_wakeup: b[15],
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; GAME_MODE_LEN];
        b[1] = self.game_mode;
        b[2] = self.fn_switch;
        b[3] = self.sleep_time;
        b[4] = self.key_delay;
        b[5] = self.report_rate;
        b[6] = self.system_mode;
        b[7] = self.tft_display_time;
        b[8] = (self.top_dead_zone * 100.0).round().clamp(0.0, 255.0) as u8;
        b[9] = (self.bottom_dead_zone * 100.0).round().clamp(0.0, 255.0) as u8;
        b[11] = self.stability_mode;
        b[14] = self.auto_calibration;
        b[15] = self.single_key_wakeup;
        b
    }
}

// ------------------------------------------------------------- rapid trigger

pub const RT_ENTRY_LEN: usize = 8;
pub const RT_TABLE_LEN: usize = KEY_SLOTS * RT_ENTRY_LEN; // 1008

/// Per-key actuation behaviour. This is the heart of a Hall-effect board:
/// instead of one fixed actuation point, each key gets a trigger depth plus
/// independent press/release rapid-trigger deltas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtKey {
    /// Which switch profile the key is calibrated against.
    pub axis_type: u8,
    /// Continuous rapid trigger over the whole travel rather than only past
    /// the actuation point.
    pub whole_fast: bool,
    /// Firmware's aggressive re-trigger mode.
    pub rampage: bool,
    /// Actuation depth in millimetres.
    pub trigger_mm: f32,
    /// Rapid-trigger press sensitivity in millimetres.
    pub press_rt_mm: f32,
    /// Rapid-trigger release sensitivity in millimetres.
    pub release_rt_mm: f32,
}

impl Default for RtKey {
    fn default() -> Self {
        Self {
            axis_type: 1,
            whole_fast: false,
            rampage: false,
            trigger_mm: 1.2,
            press_rt_mm: 0.1,
            release_rt_mm: 0.1,
        }
    }
}

impl RtKey {
    /// A keyboard that has never been configured reports an all-zero entry.
    /// Zero is not a usable actuation depth, so callers must not present or
    /// write it back verbatim.
    pub fn is_unconfigured(&self) -> bool {
        self.axis_type == 0
            && self.trigger_mm == 0.0
            && self.press_rt_mm == 0.0
            && self.release_rt_mm == 0.0
    }

    /// Substitute firmware defaults for an unconfigured entry.
    pub fn or_default(self) -> Self {
        if self.is_unconfigured() {
            Self::default()
        } else {
            self
        }
    }
}

/// Decode the whole rapid-trigger table. `scale` comes from
/// [`DeviceInfo::rt_scale`]; the trigger depth is always hundredths.
pub fn decode_rt_table(b: &[u8], scale: f32) -> Result<Vec<RtKey>> {
    need(b, RT_TABLE_LEN)?;
    let mut out = Vec::with_capacity(KEY_SLOTS);
    for i in 0..KEY_SLOTS {
        let o = i * RT_ENTRY_LEN;
        let flags = b[o + 1];
        out.push(RtKey {
            axis_type: b[o],
            whole_fast: flags & 0x01 != 0,
            rampage: flags & 0x02 != 0,
            trigger_mm: le16(b, o + 2) as f32 / 100.0,
            press_rt_mm: le16(b, o + 4) as f32 / scale,
            release_rt_mm: le16(b, o + 6) as f32 / scale,
        });
    }
    Ok(out)
}

pub fn encode_rt_table(keys: &[RtKey], scale: f32) -> Vec<u8> {
    let mut b = vec![0u8; RT_TABLE_LEN];
    for (i, k) in keys.iter().take(KEY_SLOTS).enumerate() {
        let o = i * RT_ENTRY_LEN;
        let mut flags = 0u8;
        if k.whole_fast {
            flags |= 0x01;
        }
        if k.rampage {
            flags |= 0x02;
        }
        b[o] = k.axis_type;
        b[o + 1] = flags;
        b[o + 2..o + 4]
            .copy_from_slice(&((k.trigger_mm * 100.0).round().max(0.0) as u16).to_le_bytes());
        b[o + 4..o + 6]
            .copy_from_slice(&((k.press_rt_mm * scale).round().max(0.0) as u16).to_le_bytes());
        b[o + 6..o + 8]
            .copy_from_slice(&((k.release_rt_mm * scale).round().max(0.0) as u16).to_le_bytes());
    }
    b
}

// ------------------------------------------------------------------ lighting

pub const LED_EFFECT_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Global lighting state.
///
/// Verified against firmware 1.09 on `0C45:8A01`.
///
/// `color_mode` decides where the colour comes from, and nothing else works
/// until it is right:
///
/// * `0` — use `primary` (and `secondary`) from this record.
/// * `1` — the firmware chooses its own colours, producing a rainbow, and the
///   RGB bytes here are ignored.
///
/// The vendor software forces `color_mode` to 0 whenever the user picks a
/// colour, which is the only reason its colour picker appears to work.
///
/// Reading the record back does not reliably echo the colour that was written,
/// so treat `GET_LED_EFFECT` as authoritative for `mode`, `brightness`,
/// `speed` and `direction`, but not for colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedEffect {
    pub mode: u8,
    /// Only used when `color_mode` is 0.
    pub primary: Rgb,
    /// Only used when `color_mode` is 0.
    pub secondary: Rgb,
    /// 0 = use `primary`/`secondary`; 1 = firmware picks (rainbow).
    pub color_mode: u8,
    /// 1..=5. The vendor reads the bounds as `minBrightness || 1` /
    /// `maxBrightness || 5`, and this board declares no override.
    pub brightness: u8,
    /// 1..=5, by the same rule as `brightness`. Higher values are not a
    /// wider range — the firmware misreads them.
    pub speed: u8,
    pub direction: u8,
    pub effect_mode_type: u8,
}

impl LedEffect {
    /// What the firmware reports after a full factory reset. Measured, not
    /// inferred; `color_mode` 1 means the board picks its own colours, which
    /// is why a reset board shows a moving rainbow.
    pub fn factory_default() -> Self {
        Self {
            mode: 11,
            primary: Rgb::new(255, 255, 255),
            secondary: Rgb::new(0, 0, 0),
            color_mode: 1,
            brightness: 5,
            speed: 4,
            direction: 0,
            effect_mode_type: 0,
        }
    }

    pub fn request() -> Request<'static> {
        Request::read(Cmd::GetLedEffect, LED_EFFECT_LEN)
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        need(b, LED_EFFECT_LEN)?;
        Ok(Self {
            mode: b[0],
            primary: Rgb::new(b[1], b[2], b[3]),
            secondary: Rgb::new(b[5], b[6], b[7]),
            color_mode: b[8],
            brightness: b[9],
            speed: b[10],
            direction: b[11],
            effect_mode_type: b[12],
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; LED_EFFECT_LEN];
        b[0] = self.mode;
        b[1] = self.primary.r;
        b[2] = self.primary.g;
        b[3] = self.primary.b;
        b[4] = 0xFF; // driver setting, always full-scale
        b[5] = self.secondary.r;
        b[6] = self.secondary.g;
        b[7] = self.secondary.b;
        b[8] = self.color_mode;
        b[9] = self.brightness;
        b[10] = self.speed;
        b[11] = self.direction;
        b[12] = self.effect_mode_type;
        // Trailing check bytes the firmware validates.
        b[14] = 0xAA;
        b[15] = 0x55;
        b
    }
}

pub const CUSTOM_LED_LEN: usize = KEY_SLOTS * 4; // 504

/// Per-key static colours used when `LedEffect::mode` selects custom lighting.
pub fn decode_custom_led(b: &[u8]) -> Result<Vec<Rgb>> {
    need(b, CUSTOM_LED_LEN)?;
    Ok((0..KEY_SLOTS)
        .map(|i| {
            let o = i * 4;
            Rgb::new(b[o + 1], b[o + 2], b[o + 3])
        })
        .collect())
}

pub fn encode_custom_led(colors: &[Rgb]) -> Vec<u8> {
    let mut b = vec![0u8; CUSTOM_LED_LEN];
    for (i, c) in colors.iter().take(KEY_SLOTS).enumerate() {
        let o = i * 4;
        b[o] = i as u8; // LED index
        b[o + 1] = c.r;
        b[o + 2] = c.g;
        b[o + 3] = c.b;
    }
    b
}

// --------------------------------------------------------------- key actions

/// What a key slot does. The firmware stores four bytes per slot; the first
/// selects the page and the rest are page-specific arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Page {
    Default = 0,
    Mouse = 1,
    Keyboard = 2,
    ConsumerKey = 3,
    SystemKey = 4,
    ExtraFunction = 5,
    Macro = 6,
    Cb = 7,
    Dks = 8,
    Mt = 9,
    Tgl = 10,
    Socd = 11,
    Rs = 12,
    Func = 13,
}

impl Page {
    pub fn from_raw(v: u8) -> Option<Page> {
        Some(match v {
            0 => Page::Default,
            1 => Page::Mouse,
            2 => Page::Keyboard,
            3 => Page::ConsumerKey,
            4 => Page::SystemKey,
            5 => Page::ExtraFunction,
            6 => Page::Macro,
            7 => Page::Cb,
            8 => Page::Dks,
            9 => Page::Mt,
            10 => Page::Tgl,
            11 => Page::Socd,
            12 => Page::Rs,
            13 => Page::Func,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyAction {
    pub page: Page,
    pub p1: u8,
    pub p2: u8,
    pub p3: u8,
}

impl Default for KeyAction {
    fn default() -> Self {
        Self {
            page: Page::Default,
            p1: 0,
            p2: 0,
            p3: 0,
        }
    }
}

impl KeyAction {
    /// A plain keystroke: `modifiers` is the HID modifier bitmap, `usage` a
    /// HID Keyboard/Keypad usage ID.
    pub const fn keyboard(modifiers: u8, usage: u8) -> Self {
        Self {
            page: Page::Keyboard,
            p1: modifiers,
            p2: usage,
            p3: 0,
        }
    }

    pub const fn mouse(button: u8, value: u8) -> Self {
        Self {
            page: Page::Mouse,
            p1: button,
            p2: value,
            p3: 0,
        }
    }

    pub fn consumer(usage: u16) -> Self {
        Self {
            page: Page::ConsumerKey,
            p1: (usage & 0xFF) as u8,
            p2: (usage >> 8) as u8,
            p3: 0,
        }
    }

    pub const fn disabled() -> Self {
        Self {
            page: Page::Default,
            p1: 0,
            p2: 0,
            p3: 0,
        }
    }
}

pub const KEY_TABLE_LEN: usize = KEY_SLOTS * 4; // 504

pub fn decode_key_table(b: &[u8]) -> Result<Vec<KeyAction>> {
    need(b, KEY_TABLE_LEN)?;
    Ok((0..KEY_SLOTS)
        .map(|i| {
            let o = i * 4;
            KeyAction {
                page: Page::from_raw(b[o]).unwrap_or(Page::Default),
                p1: b[o + 1],
                p2: b[o + 2],
                p3: b[o + 3],
            }
        })
        .collect())
}

pub fn encode_key_table(keys: &[KeyAction]) -> Vec<u8> {
    let mut b = vec![0u8; KEY_TABLE_LEN];
    for (i, k) in keys.iter().take(KEY_SLOTS).enumerate() {
        let o = i * 4;
        b[o] = k.page as u8;
        b[o + 1] = k.p1;
        b[o + 2] = k.p2;
        b[o + 3] = k.p3;
    }
    b
}

/// Request body for [`crate::Cmd::GetAllLightsRgb`]: the firmware wants the
/// LED indices it should report on, with the colour bytes left clear.
pub fn all_lights_request_body() -> Vec<u8> {
    let mut b = vec![0u8; CUSTOM_LED_LEN];
    for i in 0..KEY_SLOTS {
        b[i * 4] = i as u8;
    }
    b
}

/// Live colour of every LED, as the keyboard is currently driving them.
///
/// Unlike [`decode_custom_led`], which reports the stored per-key table, this
/// reflects what the lighting engine is actually displaying right now — the
/// only reliable way to tell whether a write changed anything.
pub fn decode_all_lights(b: &[u8]) -> Result<Vec<Rgb>> {
    decode_custom_led(b)
}

// -------------------------------------------------- live calibration reports

/// Unsolicited report streamed while calibration or simulation test is on.
/// Carries the live magnetic reading for one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyTelemetry {
    pub key_index: u8,
    pub calibration_status: u8,
    pub max_value: u16,
    pub min_value: u16,
    pub current_value: u16,
    /// Current travel in hundredths of a millimetre.
    pub key_stroke: u16,
    /// Total travel of the switch, hundredths of a millimetre.
    pub max_stroke: u16,
}

impl KeyTelemetry {
    /// Returns `None` for any report that is not a calibration frame, so the
    /// caller can pass every inbound report through unfiltered.
    pub fn parse(raw: &[u8]) -> Option<Self> {
        if raw.len() < 14
            || raw[0] != crate::RESP_MAGIC
            || raw[1] != Cmd::MagneticAxisCalibrationData.raw()
        {
            return None;
        }
        Some(Self {
            key_index: raw[2],
            calibration_status: raw[3],
            max_value: le16(raw, 4),
            // The firmware uses the top bit of the minimum as a flag.
            min_value: le16(raw, 6) & 0x7FFF,
            current_value: le16(raw, 8),
            key_stroke: le16(raw, 10),
            max_stroke: le16(raw, 12),
        })
    }

    pub fn stroke_mm(&self) -> f32 {
        self.key_stroke as f32 / 100.0
    }

    pub fn max_stroke_mm(&self) -> f32 {
        self.max_stroke as f32 / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_info_version_decodes_bcd() {
        let mut b = vec![0u8; DEVICE_INFO_LEN];
        b[4] = 0x45;
        b[5] = 0x0C; // 0x0C45
        b[8] = 0x23; // units 3, tens 2 -> 23
        b[9] = 1; // hundreds -> 100
        b[29] = 1;
        let d = DeviceInfo::decode(&b).unwrap();
        assert_eq!(d.vid, 0x0C45);
        assert_eq!(d.version_centi, 123);
        assert_eq!(d.version_string(), "1.23");
        assert_eq!(d.rt_scale(), 1000.0);
    }

    #[test]
    fn game_mode_round_trips() {
        let g = GameMode {
            game_mode: 1,
            report_rate: 2,
            top_dead_zone: 0.15,
            bottom_dead_zone: 0.25,
            stability_mode: 1,
            auto_calibration: 1,
            ..Default::default()
        };
        let back = GameMode::decode(&g.encode()).unwrap();
        assert_eq!(back.report_rate, 2);
        assert!((back.top_dead_zone - 0.15).abs() < 1e-6);
        assert!((back.bottom_dead_zone - 0.25).abs() < 1e-6);
        assert_eq!(back.auto_calibration, 1);
    }

    #[test]
    fn rt_table_round_trips_at_both_precisions() {
        for scale in [100.0f32, 1000.0] {
            let mut keys = vec![RtKey::default(); KEY_SLOTS];
            keys[0] = RtKey {
                axis_type: 4,
                whole_fast: true,
                rampage: false,
                trigger_mm: 1.75,
                press_rt_mm: 0.2,
                release_rt_mm: 0.3,
            };
            let bytes = encode_rt_table(&keys, scale);
            assert_eq!(bytes.len(), RT_TABLE_LEN);
            let back = decode_rt_table(&bytes, scale).unwrap();
            assert_eq!(back[0].axis_type, 4);
            assert!(back[0].whole_fast);
            assert!(!back[0].rampage);
            assert!((back[0].trigger_mm - 1.75).abs() < 1e-6);
            assert!((back[0].press_rt_mm - 0.2).abs() < 1e-3);
            assert!((back[0].release_rt_mm - 0.3).abs() < 1e-3);
        }
    }

    /// A factory-reset board reports every rapid-trigger slot as zeros, so
    /// "unconfigured" is the reset state and not a fault.
    #[test]
    fn a_reset_board_reports_an_unconfigured_rt_table() {
        let table = decode_rt_table(&vec![0u8; RT_TABLE_LEN], 100.0).unwrap();
        assert!(table.iter().all(|k| k.is_unconfigured()));
    }

    #[test]
    fn recorded_factory_defaults_round_trip() {
        let g = GameMode::factory_default();
        assert_eq!(g.report_rate, 6, "8000 Hz");
        assert_eq!(g.stability_mode, 1);
        assert_eq!(g.auto_calibration, 1);
        assert_eq!(GameMode::decode(&g.encode()).unwrap().report_rate, 6);

        let l = LedEffect::factory_default();
        assert_eq!(l.mode, 11);
        assert_eq!(l.color_mode, 1, "the board picks its own colours");
        assert_eq!(l.brightness, 5);
        assert_eq!(LedEffect::decode(&l.encode()).unwrap(), l);
    }

    #[test]
    fn all_zero_entries_are_treated_as_unconfigured() {
        let zero = RtKey {
            axis_type: 0,
            whole_fast: false,
            rampage: false,
            trigger_mm: 0.0,
            press_rt_mm: 0.0,
            release_rt_mm: 0.0,
        };
        assert!(zero.is_unconfigured());
        let fixed = zero.or_default();
        assert!(
            fixed.trigger_mm > 0.0,
            "must never present 0.00 mm actuation"
        );
        assert_eq!(fixed, RtKey::default());
    }

    #[test]
    fn configured_entries_are_left_alone() {
        let real = RtKey {
            trigger_mm: 0.4,
            axis_type: 2,
            ..RtKey::default()
        };
        assert!(!real.is_unconfigured());
        assert_eq!(real.or_default(), real);
    }

    #[test]
    fn a_freshly_read_zero_table_normalises_to_defaults() {
        let table = decode_rt_table(&vec![0u8; RT_TABLE_LEN], 100.0).unwrap();
        assert!(table.iter().all(|k| k.is_unconfigured()));
        let norm: Vec<_> = table.into_iter().map(|k| k.or_default()).collect();
        assert!(norm.iter().all(|k| k.trigger_mm >= 0.1));
    }

    #[test]
    fn rt_flags_are_independent() {
        let mut keys = vec![RtKey::default(); KEY_SLOTS];
        keys[3].rampage = true;
        keys[3].whole_fast = false;
        let b = encode_rt_table(&keys, 100.0);
        assert_eq!(b[3 * RT_ENTRY_LEN + 1], 0x02);
        let back = decode_rt_table(&b, 100.0).unwrap();
        assert!(back[3].rampage && !back[3].whole_fast);
    }

    #[test]
    fn led_effect_writes_check_bytes() {
        let e = LedEffect {
            mode: 3,
            primary: Rgb::new(10, 20, 30),
            secondary: Rgb::new(40, 50, 60),
            brightness: 200,
            speed: 5,
            ..Default::default()
        };
        let b = e.encode();
        assert_eq!(b[4], 0xFF);
        assert_eq!(b[14], 0xAA);
        assert_eq!(b[15], 0x55);
        let back = LedEffect::decode(&b).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn custom_led_indexes_each_slot() {
        let colors: Vec<_> = (0..KEY_SLOTS).map(|i| Rgb::new(i as u8, 0, 0)).collect();
        let b = encode_custom_led(&colors);
        assert_eq!(b[0], 0);
        assert_eq!(b[4], 1, "second entry carries its own index");
        assert_eq!(decode_custom_led(&b).unwrap()[5].r, 5);
    }

    #[test]
    fn key_table_round_trips() {
        let mut keys = vec![KeyAction::default(); KEY_SLOTS];
        keys[0] = KeyAction::keyboard(0x02, 0x04); // Shift+A
        keys[1] = KeyAction::consumer(0x00E9); // volume up
        let back = decode_key_table(&encode_key_table(&keys)).unwrap();
        assert_eq!(back[0], KeyAction::keyboard(0x02, 0x04));
        assert_eq!(back[1].page, Page::ConsumerKey);
        assert_eq!(back[1].p1, 0xE9);
        assert_eq!(back[1].p2, 0x00);
    }

    #[test]
    fn telemetry_masks_the_min_flag_bit() {
        let mut raw = vec![0u8; 32];
        raw[0] = 0x55;
        raw[1] = Cmd::MagneticAxisCalibrationData.raw();
        raw[2] = 12;
        raw[6..8].copy_from_slice(&0x8123u16.to_le_bytes());
        raw[10..12].copy_from_slice(&180u16.to_le_bytes());
        let t = KeyTelemetry::parse(&raw).unwrap();
        assert_eq!(t.key_index, 12);
        assert_eq!(t.min_value, 0x0123);
        assert!((t.stroke_mm() - 1.80).abs() < 1e-6);
    }

    #[test]
    fn telemetry_ignores_unrelated_reports() {
        let mut raw = vec![0u8; 32];
        raw[0] = 0x55;
        raw[1] = Cmd::GetDeviceInfo.raw();
        assert!(KeyTelemetry::parse(&raw).is_none());
    }
}
