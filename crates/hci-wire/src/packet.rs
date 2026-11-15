//! H4 packets: one indicator byte, then the type's header and payload.

use crate::opcode::Opcode;

/// The H4 packet indicator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketType {
    /// Host to controller command (`0x01`).
    Command,
    /// ACL data in either direction (`0x02`).
    Acl,
    /// Synchronous (SCO) data (`0x03`).
    Sco,
    /// Controller to host event (`0x04`).
    Event,
    /// Isochronous data (`0x05`).
    Iso,
}

impl PacketType {
    /// The indicator byte.
    #[must_use]
    pub const fn indicator(self) -> u8 {
        match self {
            Self::Command => 0x01,
            Self::Acl => 0x02,
            Self::Sco => 0x03,
            Self::Event => 0x04,
            Self::Iso => 0x05,
        }
    }

    /// The type an indicator byte names, if it names one.
    #[must_use]
    pub const fn from_indicator(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::Command),
            0x02 => Some(Self::Acl),
            0x03 => Some(Self::Sco),
            0x04 => Some(Self::Event),
            0x05 => Some(Self::Iso),
            _ => None,
        }
    }
}

/// The two-bit packet boundary flag of an ACL header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketBoundary {
    /// `0b00`: first, non-automatically-flushable (host to controller only).
    FirstNonFlushable,
    /// `0b01`: a continuing fragment.
    Continuing,
    /// `0b10`: first, automatically flushable.
    FirstFlushable,
    /// `0b11`: reserved.
    Reserved,
}

impl PacketBoundary {
    const fn bits(self) -> u16 {
        match self {
            Self::FirstNonFlushable => 0b00,
            Self::Continuing => 0b01,
            Self::FirstFlushable => 0b10,
            Self::Reserved => 0b11,
        }
    }

    const fn from_bits(bits: u16) -> Self {
        match bits & 0b11 {
            0b00 => Self::FirstNonFlushable,
            0b01 => Self::Continuing,
            0b10 => Self::FirstFlushable,
            _ => Self::Reserved,
        }
    }
}

/// One decoded H4 packet. SCO and ISO payloads are carried opaquely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    /// A command: opcode and parameters.
    Command {
        /// The command opcode.
        opcode: Opcode,
        /// The command parameters.
        params: Vec<u8>,
    },
    /// ACL data.
    Acl {
        /// The 12-bit connection handle.
        handle: u16,
        /// The packet boundary flag.
        boundary: PacketBoundary,
        /// The two-bit broadcast flag, kept as received.
        broadcast: u8,
        /// The data.
        data: Vec<u8>,
    },
    /// SCO data: the 16-bit handle-and-flags word, then the data.
    Sco {
        /// Handle and flag bits as received.
        handle_flags: u16,
        /// The data.
        data: Vec<u8>,
    },
    /// An event: code and parameters.
    Event {
        /// The event code.
        code: u8,
        /// The event parameters.
        params: Vec<u8>,
    },
    /// ISO data: the handle-and-flags word, then the data (whose length field is 14 bits).
    Iso {
        /// Handle and flag bits as received.
        handle_flags: u16,
        /// The data.
        data: Vec<u8>,
    },
}

impl Packet {
    /// Encode to H4 bytes.
    ///
    /// # Panics
    ///
    /// If a payload is longer than its header's length field can express (255 for a command,
    /// event or SCO packet; 65535 for ACL; 16383 for ISO).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Command { opcode, params } => {
                out.push(PacketType::Command.indicator());
                out.extend_from_slice(&opcode.0.to_le_bytes());
                out.push(u8::try_from(params.len()).expect("command parameters over 255 bytes"));
                out.extend_from_slice(params);
            }
            Self::Acl {
                handle,
                boundary,
                broadcast,
                data,
            } => {
                out.push(PacketType::Acl.indicator());
                let word = (handle & 0x0FFF)
                    | (boundary.bits() << 12)
                    | (u16::from(broadcast & 0b11) << 14);
                out.extend_from_slice(&word.to_le_bytes());
                let len = u16::try_from(data.len()).expect("ACL data over 65535 bytes");
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(data);
            }
            Self::Sco { handle_flags, data } => {
                out.push(PacketType::Sco.indicator());
                out.extend_from_slice(&handle_flags.to_le_bytes());
                out.push(u8::try_from(data.len()).expect("SCO data over 255 bytes"));
                out.extend_from_slice(data);
            }
            Self::Event { code, params } => {
                out.push(PacketType::Event.indicator());
                out.push(*code);
                out.push(u8::try_from(params.len()).expect("event parameters over 255 bytes"));
                out.extend_from_slice(params);
            }
            Self::Iso { handle_flags, data } => {
                out.push(PacketType::Iso.indicator());
                out.extend_from_slice(&handle_flags.to_le_bytes());
                let len = u16::try_from(data.len())
                    .ok()
                    .filter(|l| *l <= 0x3FFF)
                    .expect("ISO data over 16383 bytes");
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(data);
            }
        }
        out
    }

    /// Split an ACL handle-and-flags word into handle, boundary and broadcast flags.
    pub(crate) fn acl_parts(word: u16) -> (u16, PacketBoundary, u8) {
        (
            word & 0x0FFF,
            PacketBoundary::from_bits(word >> 12),
            ((word >> 14) & 0b11) as u8,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Packet, PacketBoundary};
    use crate::opcode::opcodes;

    #[test]
    fn command_wire_bytes() {
        let p = Packet::Command {
            opcode: opcodes::RESET,
            params: vec![],
        };
        assert_eq!(p.encode(), [0x01, 0x03, 0x0C, 0x00]);
    }

    #[test]
    fn command_with_parameters() {
        let p = Packet::Command {
            opcode: opcodes::LE_SET_ADVERTISING_ENABLE,
            params: vec![1],
        };
        assert_eq!(p.encode(), [0x01, 0x0A, 0x20, 0x01, 0x01]);
    }

    #[test]
    fn acl_header_packs_handle_and_flags() {
        let p = Packet::Acl {
            handle: 0x0041,
            boundary: PacketBoundary::FirstFlushable,
            broadcast: 0,
            data: vec![1, 2, 3],
        };
        assert_eq!(p.encode(), [0x02, 0x41, 0x20, 0x03, 0x00, 1, 2, 3]);
    }

    #[test]
    fn event_wire_bytes() {
        let p = Packet::Event {
            code: 0x0E,
            params: vec![1, 3, 12, 0],
        };
        assert_eq!(p.encode(), [0x04, 0x0E, 0x04, 1, 3, 12, 0]);
    }
}
