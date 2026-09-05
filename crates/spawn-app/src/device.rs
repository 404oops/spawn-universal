// SPDX-License-Identifier: GPL-3.0-or-later
//
// HID transport. Wraps hidapi and drives the request/response exchange the
// firmware expects: one packet out, one matching packet in, retry on silence.

use anyhow::{Context as _, Result, anyhow, bail};
use hidapi::{HidApi, HidDevice};
use spawn_protocol::{
    CUSTOM_LED_LEN, Cmd, DEVICE_INFO_LEN, DeviceInfo, GAME_MODE_LEN, GameMode, KEY_TABLE_LEN,
    KeyAction, KeyTelemetry, LED_EFFECT_LEN, LedEffect, OUT_REPORT_ID, RT_TABLE_LEN, ResetScope,
    Rgb, RtKey, VENDOR_USAGE_PAGES, VID_SONIX,
    codec::{Request, Response, join},
    descriptor,
};
use std::time::{Duration, Instant};

/// How long to wait for a reply to one packet.
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);
/// Retries before an exchange is reported as failed.
const MAX_RETRIES: u32 = 3;

/// A configuration interface we could open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub product: String,
}

impl Candidate {
    pub fn display_name(&self) -> String {
        if self.product.is_empty() {
            format!("{:04X}:{:04X}", self.vid, self.pid)
        } else {
            self.product.clone()
        }
    }
}

/// Enumerate SONiX keyboards exposing a vendor configuration collection.
///
/// The boot-keyboard interface on the same device must never be opened for
/// configuration, which is why the usage page is part of the filter.
pub fn enumerate(api: &HidApi) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = api
        .device_list()
        .filter(|d| d.vendor_id() == VID_SONIX)
        .filter(|d| VENDOR_USAGE_PAGES.contains(&d.usage_page()))
        .map(|d| Candidate {
            path: d.path().to_string_lossy().into_owned(),
            vid: d.vendor_id(),
            pid: d.product_id(),
            usage_page: d.usage_page(),
            product: d.product_string().unwrap_or_default().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// A complete read of everything the UI shows, taken in one pass.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub info: DeviceInfo,
    pub game_mode: GameMode,
    pub rt: Vec<RtKey>,
    pub led: LedEffect,
    pub custom_led: Vec<Rgb>,
    pub keys: Vec<KeyAction>,
}

/// An open configuration interface.
pub struct Keyboard {
    dev: HidDevice,
    packet_len: usize,
    candidate: Candidate,
}

impl Keyboard {
    pub fn open(api: &HidApi, candidate: &Candidate) -> Result<Self> {
        let path = std::ffi::CString::new(candidate.path.as_str())
            .map_err(|_| anyhow!("device path contains an interior nul byte"))?;
        let dev = api.open_path(&path).map_err(|e| {
            // macOS (and Windows, for some interfaces) hand out HID devices
            // exclusively, so the raw message here is an opaque IOKit code.
            // Say what actually went wrong instead.
            let raw = e.to_string();
            if raw.contains("already open") || raw.contains("exclusive") {
                anyhow!(
                    "the keyboard is already open in another program \u{2014} close the other \
                     copy of this app, or the vendor software, and try again"
                )
            } else if raw.contains("Permission") || raw.contains("permission") {
                anyhow!(
                    "not allowed to open the keyboard \u{2014} on Linux install \
                     packaging/99-spawn.rules and replug the device"
                )
            } else {
                anyhow!("could not open the keyboard: {raw}")
            }
        })?;
        dev.set_blocking_mode(false).ok();

        // Chunk size follows the output report length; fall back to 32 when
        // the platform will not hand us a descriptor.
        let mut buf = [0u8; 4096];
        let packet_len = match dev.get_report_descriptor(&mut buf) {
            Ok(n) => descriptor::output_report_len_or_default(&buf[..n]),
            Err(_) => spawn_protocol::DEFAULT_PACKET_LEN,
        };

        Ok(Self {
            dev,
            packet_len,
            candidate: candidate.clone(),
        })
    }

    pub fn packet_len(&self) -> usize {
        self.packet_len
    }

    /// The interface this handle was opened from.
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }

    /// Write one packet, prefixed with the report ID hidapi expects.
    fn write_packet(&self, packet: &[u8]) -> Result<()> {
        let mut buf = Vec::with_capacity(packet.len() + 1);
        buf.push(OUT_REPORT_ID);
        buf.extend_from_slice(packet);
        self.dev.write(&buf).context("HID write failed")?;
        Ok(())
    }

    /// Read reports until one answers `cmd`, or the deadline passes.
    ///
    /// Reports for other commands are unsolicited telemetry; they are handed
    /// to `sink` rather than discarded, so a monitor running alongside a
    /// configuration write does not lose frames.
    fn await_reply(
        &self,
        cmd: Cmd,
        deadline: Instant,
        sink: &mut dyn FnMut(KeyTelemetry),
    ) -> Result<Response> {
        let mut buf = vec![0u8; self.packet_len.max(64)];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timed out waiting for a reply to {cmd:?}");
            }
            let n = self
                .dev
                .read_timeout(&mut buf, remaining.as_millis().min(i32::MAX as u128) as i32)
                .context("HID read failed")?;
            if n == 0 {
                continue; // timeout tick with nothing pending
            }
            let raw = &buf[..n];
            if let Some(t) = KeyTelemetry::parse(raw) {
                sink(t);
                continue;
            }
            match Response::parse(raw) {
                Ok(r) if r.cmd == cmd.raw() => return Ok(r),
                // Anything else on the wire is for someone else.
                _ => continue,
            }
        }
    }

    /// Run a full request, chunk by chunk, and return the replies in order.
    pub fn exchange(
        &self,
        req: &Request<'_>,
        sink: &mut dyn FnMut(KeyTelemetry),
    ) -> Result<Vec<Response>> {
        let mut replies = Vec::new();
        for packet in req.plan() {
            let mut attempt = 0;
            loop {
                self.write_packet(packet.as_slice())?;
                match self.await_reply(req.cmd, Instant::now() + REPLY_TIMEOUT, sink) {
                    Ok(r) => {
                        replies.push(r);
                        break;
                    }
                    Err(e) => {
                        attempt += 1;
                        if attempt > MAX_RETRIES {
                            return Err(e.context(format!(
                                "{:?} failed after {MAX_RETRIES} retries",
                                req.cmd
                            )));
                        }
                        log::warn!("{:?} packet timed out, resend {attempt}", req.cmd);
                    }
                }
            }
        }
        Ok(replies)
    }

    /// Fire a command that the firmware answers with action rather than data.
    pub fn send_control(&self, cmd: Cmd, scalar: u8) -> Result<()> {
        let req = Request::control(cmd, scalar).with_packet_len(self.packet_len);
        self.write_packet(req.control_packet().as_slice())
    }

    fn read_block(
        &self,
        cmd: Cmd,
        len: usize,
        sink: &mut dyn FnMut(KeyTelemetry),
    ) -> Result<Vec<u8>> {
        let req = Request::read(cmd, len).with_packet_len(self.packet_len);
        let replies = self.exchange(&req, sink)?;
        Ok(join(&replies, len)?)
    }

    fn write_block(&self, cmd: Cmd, data: &[u8], sink: &mut dyn FnMut(KeyTelemetry)) -> Result<()> {
        let req = Request::write(cmd, data).with_packet_len(self.packet_len);
        self.exchange(&req, sink)?;
        Ok(())
    }

    // ------------------------------------------------------------- reads

    pub fn read_info(&self, sink: &mut dyn FnMut(KeyTelemetry)) -> Result<DeviceInfo> {
        let b = self.read_block(Cmd::GetDeviceInfo, DEVICE_INFO_LEN, sink)?;
        Ok(DeviceInfo::decode(&b)?)
    }

    pub fn read_snapshot(&self, sink: &mut dyn FnMut(KeyTelemetry)) -> Result<Snapshot> {
        let info = self.read_info(sink)?;
        let scale = info.rt_scale();

        let game_mode =
            GameMode::decode(&self.read_block(Cmd::GetGameMode, GAME_MODE_LEN, sink)?)?;
        let rt = spawn_protocol::decode_rt_table(
            &self.read_block(Cmd::GetMagneticAxisRt, RT_TABLE_LEN, sink)?,
            scale,
        )?;
        let led = LedEffect::decode(&self.read_block(Cmd::GetLedEffect, LED_EFFECT_LEN, sink)?)?;
        let custom_led = spawn_protocol::decode_custom_led(&self.read_block(
            Cmd::GetCustomLedData,
            CUSTOM_LED_LEN,
            sink,
        )?)?;
        let keys = spawn_protocol::decode_key_table(&self.read_block(
            Cmd::GetKey,
            KEY_TABLE_LEN,
            sink,
        )?)?;

        Ok(Snapshot {
            info,
            game_mode,
            rt,
            led,
            custom_led,
            keys,
        })
    }

    // ------------------------------------------------------------ writes

    pub fn write_rt(
        &self,
        rt: &[RtKey],
        scale: f32,
        sink: &mut dyn FnMut(KeyTelemetry),
    ) -> Result<()> {
        let data = spawn_protocol::encode_rt_table(rt, scale);
        self.write_block(Cmd::SetMagneticAxisRt, &data, sink)
    }

    pub fn write_led(&self, led: &LedEffect, sink: &mut dyn FnMut(KeyTelemetry)) -> Result<()> {
        self.write_block(Cmd::SetLedEffect, &led.encode(), sink)
    }

    pub fn write_custom_led(
        &self,
        colors: &[Rgb],
        sink: &mut dyn FnMut(KeyTelemetry),
    ) -> Result<()> {
        let data = spawn_protocol::encode_custom_led(colors);
        self.write_block(Cmd::SetCustomLedData, &data, sink)
    }

    pub fn write_game_mode(&self, gm: &GameMode, sink: &mut dyn FnMut(KeyTelemetry)) -> Result<()> {
        self.write_block(Cmd::SetGameMode, &gm.encode(), sink)
    }

    pub fn write_keys(&self, keys: &[KeyAction], sink: &mut dyn FnMut(KeyTelemetry)) -> Result<()> {
        let data = spawn_protocol::encode_key_table(keys);
        debug_assert_eq!(data.len(), KEY_TABLE_LEN);
        self.write_block(Cmd::SetKey, &data, sink)
    }

    /// Read the colour every LED is currently displaying.
    pub fn read_live_leds(&self, sink: &mut dyn FnMut(KeyTelemetry)) -> Result<Vec<Rgb>> {
        let body = spawn_protocol::all_lights_request_body();
        let req = Request::write(Cmd::GetAllLightsRgb, &body).with_packet_len(self.packet_len);
        let replies = self.exchange(&req, sink)?;
        Ok(spawn_protocol::decode_all_lights(&join(
            &replies,
            CUSTOM_LED_LEN,
        )?)?)
    }

    // ------------------------------------------------------- live modes

    /// Ask the firmware to stream per-key travel. Frames arrive as
    /// unsolicited reports and are picked up by [`Self::pump`].
    pub fn start_monitor(&self) -> Result<()> {
        self.send_control(Cmd::SimulationTestOn, 0)
    }

    pub fn stop_monitor(&self) -> Result<()> {
        self.send_control(Cmd::SimulationTestOff, 0)
    }

    pub fn start_calibration(&self) -> Result<()> {
        self.send_control(Cmd::CalibrationOn, 0)
    }

    pub fn stop_calibration(&self) -> Result<()> {
        self.send_control(Cmd::CalibrationOff, 0)
    }

    pub fn factory_reset(&self, scope: ResetScope) -> Result<()> {
        self.send_control(Cmd::FactoryReset, scope.raw())
    }

    /// Drain any waiting telemetry without blocking. Returns how many frames
    /// were delivered so the caller can tell a quiet device from a dead one.
    pub fn pump(&self, sink: &mut dyn FnMut(KeyTelemetry)) -> Result<usize> {
        let mut buf = vec![0u8; self.packet_len.max(64)];
        let mut count = 0;
        loop {
            let n = self
                .dev
                .read_timeout(&mut buf, 0)
                .context("HID read failed")?;
            if n == 0 {
                return Ok(count);
            }
            if let Some(t) = KeyTelemetry::parse(&buf[..n]) {
                sink(t);
                count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_falls_back_to_ids_without_a_product_string() {
        let c = Candidate {
            path: "x".into(),
            vid: 0x0C45,
            pid: 0x8A01,
            usage_page: 0xFF67,
            product: String::new(),
        };
        assert_eq!(c.display_name(), "0C45:8A01");
    }

    #[test]
    fn candidate_prefers_the_product_string() {
        let c = Candidate {
            path: "x".into(),
            vid: 0x0C45,
            pid: 0x8A01,
            usage_page: 0xFF67,
            product: "SPAWN".into(),
        };
        assert_eq!(c.display_name(), "SPAWN");
    }
}
