// SPDX-License-Identifier: GPL-3.0-or-later
//
// Wire protocol for SONiX-based SPAWN magnetic (Hall-effect) keyboards.
//
// This crate is transport-agnostic: it turns typed configuration into raw
// HID report bytes and back. It performs no I/O, which keeps every framing
// and encoding rule directly unit-testable.

#![forbid(unsafe_code)]

pub mod codec;
pub mod descriptor;
pub mod layout;
pub mod model;

pub use codec::{ChunkPlan, Packet, Request, Response};
pub use model::*;

/// Report ID used for every vendor packet. The keyboard exposes a single
/// unnumbered vendor collection, so the ID is always zero.
pub const OUT_REPORT_ID: u8 = 0;

/// First byte of every host -> device packet.
pub const REQ_MAGIC: u8 = 0xAA;

/// First byte of every device -> host packet.
pub const RESP_MAGIC: u8 = 0x55;

/// Every packet reserves 8 bytes of header; the payload follows.
pub const HEADER_LEN: usize = 8;

/// Packet size when the descriptor does not tell us otherwise.
pub const DEFAULT_PACKET_LEN: usize = 32;

/// Number of physical key slots the firmware addresses, regardless of how
/// many are populated on a given board.
pub const KEY_SLOTS: usize = 126;

/// USB vendor ID (SONiX Technology).
pub const VID_SONIX: u16 = 0x0C45;

/// Vendor usage pages the configuration interface is known to appear on.
/// The keyboard also enumerates a boot-keyboard interface, which must not be
/// opened for configuration.
pub const VENDOR_USAGE_PAGES: &[u16] = &[0xFF67, 0xFF68, 0xFF80, 0xFF60, 0xFF00, 0xFF01, 0xFF1B];

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("packet too short: got {got} bytes, need at least {need}")]
    Short { got: usize, need: usize },
    #[error("bad response magic: expected 0x55, got {0:#04x}")]
    BadMagic(u8),
    #[error("unexpected command in response: expected {expected:#04x}, got {got:#04x}")]
    CmdMismatch { expected: u8, got: u8 },
    #[error("payload of {got} bytes does not fit in a {capacity}-byte packet")]
    Overflow { got: usize, capacity: usize },
    #[error("device returned {got} bytes, expected {want}")]
    Truncated { got: usize, want: usize },
}

pub type Result<T> = core::result::Result<T, ProtocolError>;

/// Command opcodes understood by the firmware.
///
/// Values are the firmware's own; names describe observed behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Cmd {
    CommunicationStart = 0x01,
    CommunicationEnd = 0x02,
    FactoryReset = 0x0F,
    GetDeviceInfo = 0x10,
    GetGameMode = 0x11,
    GetKey = 0x12,
    GetLedEffect = 0x13,
    GetCustomLedData = 0x14,
    GetMacro = 0x15,
    GetFnKey = 0x16,
    GetMagneticAxisRt = 0x17,
    GetMagneticAxisDks = 0x18,
    GetLightBox = 0x1B,
    GetDefaultFnKeyMatrix = 0x1C,
    GetDefaultKeyMatrix = 0x1F,
    SetGameMode = 0x21,
    SetKey = 0x22,
    SetLedEffect = 0x23,
    SetCustomLedData = 0x24,
    SetMacro = 0x25,
    SetFnKey = 0x26,
    SetMagneticAxisRt = 0x27,
    SetMagneticAxisDks = 0x28,
    SetDotMatrixMode = 0x2A,
    SetLightBox = 0x2B,
    SetCustomFunctionOn = 0x30,
    SetCustomFunctionOff = 0x31,
    GetLedData = 0x32,
    GetAllLightsRgb = 0x33,
    SetTemporaryCommandData = 0x34,
    SetMusicData = 0x35,
    ClearLedData = 0x36,
    GetAllLightsRgb24G = 0x37,
    SetLedBootAnimation = 0x40,
    SetLedUserAnimation = 0x41,
    SetLedData = 0x42,
    SetFlashDownload = 0x4F,
    SetTftUserAnimation = 0x50,
    SetTftBuiltInIndex = 0x51,
    GetMagneticAxisKeyStatus = 0x60,
    CalibrationOn = 0x64,
    CalibrationOff = 0x65,
    SimulationTestOn = 0x66,
    SimulationTestOff = 0x67,
    DeviceNotify = 0xFA,
    MagneticAxisCalibrationData = 0xFB,
    Disconnect24GNotify = 0xFC,
}

impl Cmd {
    pub const fn raw(self) -> u8 {
        self as u8
    }
}

/// Selective factory-reset scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResetScope {
    Keys = 1,
    Lighting = 2,
    Macros = 4,
    Calibration = 5,
    All = 0xFF,
}

impl ResetScope {
    pub const fn raw(self) -> u8 {
        self as u8
    }
}
