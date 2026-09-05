// SPDX-License-Identifier: GPL-3.0-or-later
//
// Packet framing.
//
// Host -> device (`REQ_MAGIC`):
//   [0]    0xAA
//   [1]    command
//   [2]    payload length carried by this packet (or a command-specific scalar)
//   [3..5] little-endian address / offset of this chunk
//   [5..8] up to three command-specific header bytes; when they are absent,
//          byte 6 carries the "final chunk" flag
//   [8..]  payload
//
// Device -> host (`RESP_MAGIC`):
//   [0]    0x55
//   [1]    command being answered
//   [2]    length / type
//   [3..5] little-endian address
//   [8..]  payload

use crate::{Cmd, DEFAULT_PACKET_LEN, HEADER_LEN, ProtocolError, REQ_MAGIC, RESP_MAGIC, Result};

/// A fully formed outbound packet, sized to the report length the device
/// advertises. Always sent whole and zero-padded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet(pub Vec<u8>);

impl Packet {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

/// Describes one outbound request before it is split into packets.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub cmd: Cmd,
    /// Total number of payload bytes the exchange covers. For reads this is
    /// how much to ask for; for writes it matches `data`.
    pub content_size: usize,
    /// Address of the first byte, in device address space.
    pub addr_start: u16,
    /// Payload for writes; `None` for reads.
    pub data: Option<&'a [u8]>,
    /// Command-specific header bytes occupying slots 5, 6 and 7.
    pub other_header: Option<[Option<u8>; 3]>,
    /// Report length advertised by the descriptor.
    pub packet_len: usize,
}

impl<'a> Request<'a> {
    pub fn read(cmd: Cmd, content_size: usize) -> Self {
        Self {
            cmd,
            content_size,
            addr_start: 0,
            data: None,
            other_header: None,
            packet_len: DEFAULT_PACKET_LEN,
        }
    }

    pub fn write(cmd: Cmd, data: &'a [u8]) -> Self {
        Self {
            cmd,
            content_size: data.len(),
            addr_start: 0,
            data: Some(data),
            other_header: None,
            packet_len: DEFAULT_PACKET_LEN,
        }
    }

    /// A command that carries no payload; `scalar` lands in the length slot,
    /// which several commands reuse as an argument (reset scope, for example).
    pub fn control(cmd: Cmd, scalar: u8) -> Self {
        Self {
            cmd,
            content_size: scalar as usize,
            addr_start: 0,
            data: None,
            other_header: None,
            packet_len: DEFAULT_PACKET_LEN,
        }
    }

    pub fn with_packet_len(mut self, len: usize) -> Self {
        self.packet_len = len.max(HEADER_LEN + 1);
        self
    }

    pub fn with_addr(mut self, addr: u16) -> Self {
        self.addr_start = addr;
        self
    }

    pub fn with_other_header(mut self, hdr: [Option<u8>; 3]) -> Self {
        self.other_header = Some(hdr);
        self
    }

    /// Bytes of payload each packet can carry.
    pub fn chunk_capacity(&self) -> usize {
        self.packet_len - HEADER_LEN
    }

    /// Split the request into the packets that must be sent, in order.
    ///
    /// Every packet expects its own response before the next is sent.
    pub fn plan(&self) -> ChunkPlan<'a> {
        let cap = self.chunk_capacity();
        let count = if self.content_size == 0 {
            1
        } else {
            self.content_size.div_ceil(cap)
        };
        ChunkPlan {
            req: self.clone(),
            cap,
            count,
            index: 0,
        }
    }

    /// Build a single control packet (no chunking, no payload).
    pub fn control_packet(&self) -> Packet {
        let scalar = u8::try_from(self.content_size).unwrap_or(0);
        Packet(encode(
            self.cmd,
            scalar,
            self.addr_start,
            None,
            self.packet_len,
            self.other_header,
            true,
        ))
    }
}

/// Iterator over the packets of a chunked request.
#[derive(Debug, Clone)]
pub struct ChunkPlan<'a> {
    req: Request<'a>,
    cap: usize,
    count: usize,
    index: usize,
}

impl<'a> ChunkPlan<'a> {
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl<'a> Iterator for ChunkPlan<'a> {
    type Item = Packet;

    fn next(&mut self) -> Option<Packet> {
        if self.index >= self.count {
            return None;
        }
        let i = self.index;
        self.index += 1;

        let offset = i * self.cap;
        let remaining = self.req.content_size.saturating_sub(offset);
        let is_last = i == self.count - 1;
        let this_len = if is_last { remaining } else { self.cap };

        let slice = self.req.data.map(|d| {
            let start = offset.min(d.len());
            let end = (offset + self.cap).min(d.len());
            &d[start..end]
        });

        let addr = self.req.addr_start.wrapping_add(offset as u16);
        Some(Packet(encode(
            self.req.cmd,
            u8::try_from(this_len).unwrap_or(u8::MAX),
            addr,
            slice,
            self.req.packet_len,
            self.req.other_header,
            is_last,
        )))
    }
}

/// Lay out one outbound packet.
pub fn encode(
    cmd: Cmd,
    len_or_type: u8,
    addr: u16,
    payload: Option<&[u8]>,
    packet_len: usize,
    other_header: Option<[Option<u8>; 3]>,
    is_last: bool,
) -> Vec<u8> {
    let mut buf = vec![0u8; packet_len];
    buf[0] = REQ_MAGIC;
    buf[1] = cmd.raw();
    buf[2] = len_or_type;
    buf[3] = (addr & 0xFF) as u8;
    buf[4] = (addr >> 8) as u8;

    // Slots 5..8 are command-specific. The "final chunk" flag lives in slot 6
    // and is only written when the command has not claimed that slot.
    let mut slot6_claimed = false;
    if let Some(hdr) = other_header {
        for (i, v) in hdr.iter().enumerate() {
            if let Some(v) = v {
                buf[5 + i] = *v;
                if i == 1 {
                    slot6_claimed = true;
                }
            }
        }
    }
    if !slot6_claimed {
        buf[6] = u8::from(is_last);
    }

    if let Some(p) = payload {
        let end = (HEADER_LEN + p.len()).min(packet_len);
        buf[HEADER_LEN..end].copy_from_slice(&p[..end - HEADER_LEN]);
    }
    buf
}

/// A decoded device -> host packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub cmd: u8,
    pub len_or_type: u8,
    pub addr: u16,
    pub data: Vec<u8>,
}

impl Response {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < HEADER_LEN {
            return Err(ProtocolError::Short {
                got: raw.len(),
                need: HEADER_LEN,
            });
        }
        if raw[0] != RESP_MAGIC {
            return Err(ProtocolError::BadMagic(raw[0]));
        }
        Ok(Response {
            cmd: raw[1],
            len_or_type: raw[2],
            addr: u16::from_le_bytes([raw[3], raw[4]]),
            data: raw[HEADER_LEN..].to_vec(),
        })
    }

    pub fn expect(self, cmd: Cmd) -> Result<Self> {
        if self.cmd != cmd.raw() {
            return Err(ProtocolError::CmdMismatch {
                expected: cmd.raw(),
                got: self.cmd,
            });
        }
        Ok(self)
    }
}

/// Concatenate the payloads of a chunked reply and trim to the expected size.
pub fn join(responses: &[Response], want: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(want);
    for r in responses {
        out.extend_from_slice(&r.data);
    }
    if out.len() < want {
        return Err(ProtocolError::Truncated {
            got: out.len(),
            want,
        });
    }
    out.truncate(want);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout_is_stable() {
        let p = encode(Cmd::GetDeviceInfo, 24, 0, None, 32, None, true);
        assert_eq!(p.len(), 32);
        assert_eq!(p[0], 0xAA);
        assert_eq!(p[1], 0x10);
        assert_eq!(p[2], 24);
        assert_eq!(p[3], 0);
        assert_eq!(p[4], 0);
        assert_eq!(p[6], 1, "final-chunk flag");
    }

    #[test]
    fn address_is_little_endian() {
        let p = encode(Cmd::GetKey, 4, 0x0140, None, 32, None, true);
        assert_eq!(p[3], 0x40);
        assert_eq!(p[4], 0x01);
    }

    #[test]
    fn other_header_claims_slot_six() {
        let hdr = [Some(1u8), Some(2u8), Some(3u8)];
        let p = encode(Cmd::SetMusicData, 0, 0, None, 32, Some(hdr), true);
        assert_eq!(&p[5..8], &[1, 2, 3]);
        assert_eq!(p[6], 2, "command header wins over the final-chunk flag");
    }

    #[test]
    fn partial_other_header_leaves_flag_intact() {
        let hdr = [Some(9u8), None, None];
        let p = encode(Cmd::SetMusicData, 0, 0, None, 32, Some(hdr), true);
        assert_eq!(p[5], 9);
        assert_eq!(p[6], 1, "slot 6 unclaimed, flag still written");
    }

    #[test]
    fn chunking_covers_payload_exactly() {
        // 1008 bytes of rapid-trigger data over 24-byte chunks.
        let data = vec![7u8; 1008];
        let req = Request::write(Cmd::SetMagneticAxisRt, &data);
        let packets: Vec<_> = req.plan().collect();
        assert_eq!(packets.len(), 42, "1008 / 24");

        let mut seen = Vec::new();
        for (i, p) in packets.iter().enumerate() {
            assert_eq!(p.0[1], Cmd::SetMagneticAxisRt.raw());
            let addr = u16::from_le_bytes([p.0[3], p.0[4]]);
            assert_eq!(addr as usize, i * 24);
            seen.extend_from_slice(&p.0[8..8 + p.0[2] as usize]);
        }
        assert_eq!(seen, data, "reassembled payload matches the source");
        assert_eq!(packets.last().unwrap().0[6], 1, "last packet flagged");
        assert_eq!(packets[0].0[6], 0, "first packet not flagged");
    }

    #[test]
    fn ragged_final_chunk_reports_its_real_length() {
        let data = vec![1u8; 50]; // 24 + 24 + 2
        let packets: Vec<_> = Request::write(Cmd::SetKey, &data).plan().collect();
        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0].0[2], 24);
        assert_eq!(packets[1].0[2], 24);
        assert_eq!(packets[2].0[2], 2);
    }

    #[test]
    fn sixty_four_byte_reports_use_larger_chunks() {
        let data = vec![0u8; 504];
        let req = Request::write(Cmd::SetCustomLedData, &data).with_packet_len(64);
        let packets: Vec<_> = req.plan().collect();
        assert_eq!(req.chunk_capacity(), 56);
        assert_eq!(packets.len(), 9, "504 / 56");
        assert_eq!(packets[0].0.len(), 64);
    }

    #[test]
    fn response_round_trip() {
        let mut raw = vec![0u8; 32];
        raw[0] = 0x55;
        raw[1] = Cmd::GetDeviceInfo.raw();
        raw[2] = 24;
        raw[3] = 0x10;
        raw[4] = 0x00;
        raw[8] = 0xAB;
        let r = Response::parse(&raw)
            .unwrap()
            .expect(Cmd::GetDeviceInfo)
            .unwrap();
        assert_eq!(r.addr, 0x0010);
        assert_eq!(r.data[0], 0xAB);
    }

    #[test]
    fn response_rejects_bad_magic() {
        let raw = [0xAAu8; 32];
        assert!(matches!(
            Response::parse(&raw),
            Err(ProtocolError::BadMagic(0xAA))
        ));
    }

    #[test]
    fn response_rejects_wrong_command() {
        let mut raw = vec![0u8; 32];
        raw[0] = 0x55;
        raw[1] = Cmd::GetKey.raw();
        let err = Response::parse(&raw).unwrap().expect(Cmd::GetLedEffect);
        assert!(matches!(err, Err(ProtocolError::CmdMismatch { .. })));
    }

    #[test]
    fn join_trims_padding() {
        let chunks = vec![
            Response {
                cmd: 0x10,
                len_or_type: 24,
                addr: 0,
                data: vec![1u8; 24],
            },
            Response {
                cmd: 0x10,
                len_or_type: 24,
                addr: 24,
                data: vec![2u8; 24],
            },
        ];
        let joined = join(&chunks, 48).unwrap();
        assert_eq!(joined.len(), 48);
        assert_eq!(joined[47], 2);
    }
}
