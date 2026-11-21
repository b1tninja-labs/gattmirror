//! Incremental H4 decoding: feed bytes as they arrive, take whole packets out.

use core::fmt;

use crate::opcode::Opcode;
use crate::packet::{Packet, PacketType};

/// A framing error. This decoder leaves the bad indicator in place. §4.A.4 describes a Reset
/// recovery this crate does not perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The byte where a packet should start is not a known packet indicator.
    UnknownIndicator(u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownIndicator(b) => write!(f, "unknown H4 packet indicator 0x{b:02X}"),
        }
    }
}

impl std::error::Error for Error {}

/// Reassembles packets from an arbitrarily chunked byte stream.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    /// An empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Bytes held that do not yet make a whole packet.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Take the next whole packet, or `None` if more bytes are needed.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIndicator`] if the next byte is not a packet indicator; the byte is left in
    /// place and every later call returns the same error.
    pub fn next_packet(&mut self) -> Result<Option<Packet>, Error> {
        let Some(&first) = self.buf.first() else {
            return Ok(None);
        };
        let kind = PacketType::from_indicator(first).ok_or(Error::UnknownIndicator(first))?;
        let b = &self.buf[1..];
        // (header length, offset of the length field, width of that field in bytes)
        let (header, len_at, len_width) = match kind {
            PacketType::Command | PacketType::Sco => (3, 2, 1),
            PacketType::Acl | PacketType::Iso => (4, 2, 2),
            PacketType::Event => (2, 1, 1),
        };
        if b.len() < header {
            return Ok(None);
        }
        let raw_len = if len_width == 1 {
            usize::from(b[len_at])
        } else {
            usize::from(u16::from_le_bytes([b[len_at], b[len_at + 1]]))
        };
        let len = if kind == PacketType::Iso {
            raw_len & 0x3FFF
        } else {
            raw_len
        };
        if b.len() < header + len {
            return Ok(None);
        }
        let word = u16::from_le_bytes([b[0], b[1]]);
        let body = b[header..header + len].to_vec();
        let packet = match kind {
            PacketType::Command => Packet::Command {
                opcode: Opcode(word),
                params: body,
            },
            PacketType::Acl => {
                let (handle, boundary, broadcast) = Packet::acl_parts(word);
                Packet::Acl {
                    handle,
                    boundary,
                    broadcast,
                    data: body,
                }
            }
            PacketType::Sco => Packet::Sco {
                handle_flags: word,
                data: body,
            },
            PacketType::Event => Packet::Event {
                code: b[0],
                params: body,
            },
            PacketType::Iso => Packet::Iso {
                handle_flags: word,
                data: body,
            },
        };
        self.buf.drain(..1 + header + len);
        Ok(Some(packet))
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoder, Error};
    use crate::opcode::opcodes;
    use crate::packet::{Packet, PacketBoundary};

    fn all(d: &mut Decoder) -> Vec<Packet> {
        let mut v = Vec::new();
        while let Some(p) = d.next_packet().expect("no framing error") {
            v.push(p);
        }
        v
    }

    #[test]
    fn two_packets_in_one_read() {
        let mut d = Decoder::new();
        d.push(&[
            0x04, 0x0E, 0x04, 0x01, 0x03, 0x0C, 0x00, 0x04, 0x0E, 0x04, 0x01, 0x01, 0x10, 0x00,
        ]);
        let got = all(&mut d);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            Packet::Event {
                code: 0x0E,
                params: vec![1, 3, 12, 0]
            }
        );
        assert_eq!(d.pending(), 0);
    }

    #[test]
    fn one_byte_at_a_time_gives_the_same_packets() {
        let wire = [
            Packet::Command {
                opcode: opcodes::RESET,
                params: vec![],
            }
            .encode(),
            Packet::Acl {
                handle: 0x0041,
                boundary: PacketBoundary::FirstFlushable,
                broadcast: 0,
                data: vec![9; 300],
            }
            .encode(),
            Packet::Event {
                code: 0x3E,
                params: vec![1, 2, 3],
            }
            .encode(),
        ]
        .concat();
        let mut d = Decoder::new();
        let mut got = Vec::new();
        for byte in wire {
            d.push(&[byte]);
            got.extend(all(&mut d));
        }
        assert_eq!(got.len(), 3);
        assert!(matches!(&got[1], Packet::Acl { handle: 0x0041, data, .. } if data.len() == 300));
    }

    #[test]
    fn a_partial_packet_waits() {
        let mut d = Decoder::new();
        d.push(&[0x04, 0x0E, 0x04, 0x01]);
        assert_eq!(d.next_packet(), Ok(None));
        assert_eq!(d.pending(), 4);
        d.push(&[0x03, 0x0C, 0x00]);
        assert!(d.next_packet().unwrap().is_some());
    }

    #[test]
    fn an_unknown_indicator_is_an_error_that_sticks() {
        let mut d = Decoder::new();
        d.push(&[0x09, 0x00]);
        assert_eq!(d.next_packet(), Err(Error::UnknownIndicator(0x09)));
        assert_eq!(d.next_packet(), Err(Error::UnknownIndicator(0x09)));
    }

    #[test]
    fn acl_flags_are_unpacked() {
        let mut d = Decoder::new();
        // 0x6041: handle 0x041, boundary bits 0b10, broadcast bits 0b01.
        d.push(&[0x02, 0x41, 0x60, 0x01, 0x00, 0xAA]);
        assert_eq!(
            d.next_packet().unwrap().unwrap(),
            Packet::Acl {
                handle: 0x0041,
                boundary: PacketBoundary::FirstFlushable,
                broadcast: 1,
                data: vec![0xAA]
            }
        );
    }
}
