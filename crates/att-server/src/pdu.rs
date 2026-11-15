use crate::uuid::Uuid;

/// ATT opcodes. Values are from memory of the protocol and are `[unread]`; see `ROADMAP.md`.
pub mod opcode {
    /// Error Response.
    pub const ERROR_RSP: u8 = 0x01;
    /// Exchange MTU Request.
    pub const MTU_REQ: u8 = 0x02;
    /// Exchange MTU Response.
    pub const MTU_RSP: u8 = 0x03;
    /// Find Information Request.
    pub const FIND_INFO_REQ: u8 = 0x04;
    /// Find Information Response.
    pub const FIND_INFO_RSP: u8 = 0x05;
    /// Find By Type Value Request.
    pub const FIND_BY_TYPE_VALUE_REQ: u8 = 0x06;
    /// Find By Type Value Response.
    pub const FIND_BY_TYPE_VALUE_RSP: u8 = 0x07;
    /// Read By Type Request.
    pub const READ_BY_TYPE_REQ: u8 = 0x08;
    /// Read By Type Response.
    pub const READ_BY_TYPE_RSP: u8 = 0x09;
    /// Read Request.
    pub const READ_REQ: u8 = 0x0A;
    /// Read Response.
    pub const READ_RSP: u8 = 0x0B;
    /// Read Blob Request.
    pub const READ_BLOB_REQ: u8 = 0x0C;
    /// Read Blob Response.
    pub const READ_BLOB_RSP: u8 = 0x0D;
    /// Read Multiple Request.
    pub const READ_MULTIPLE_REQ: u8 = 0x0E;
    /// Read Multiple Response.
    pub const READ_MULTIPLE_RSP: u8 = 0x0F;
    /// Read By Group Type Request.
    pub const READ_BY_GROUP_TYPE_REQ: u8 = 0x10;
    /// Read By Group Type Response.
    pub const READ_BY_GROUP_TYPE_RSP: u8 = 0x11;
    /// Write Request.
    pub const WRITE_REQ: u8 = 0x12;
    /// Write Response.
    pub const WRITE_RSP: u8 = 0x13;
    /// Prepare Write Request.
    pub const PREPARE_WRITE_REQ: u8 = 0x16;
    /// Prepare Write Response.
    pub const PREPARE_WRITE_RSP: u8 = 0x17;
    /// Execute Write Request.
    pub const EXECUTE_WRITE_REQ: u8 = 0x18;
    /// Execute Write Response.
    pub const EXECUTE_WRITE_RSP: u8 = 0x19;
    /// Handle Value Notification.
    pub const NOTIFICATION: u8 = 0x1B;
    /// Handle Value Indication.
    pub const INDICATION: u8 = 0x1D;
    /// Handle Value Confirmation.
    pub const CONFIRMATION: u8 = 0x1E;
    /// Write Command.
    pub const WRITE_CMD: u8 = 0x52;
}

use opcode as op;

/// An ATT error code. Codes this crate does not name are carried as [`ErrorCode::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The handle is not in the table.
    InvalidHandle,
    /// The attribute may not be read.
    ReadNotPermitted,
    /// The attribute may not be written.
    WriteNotPermitted,
    /// The PDU is malformed.
    InvalidPdu,
    /// Authentication is required.
    InsufficientAuthentication,
    /// The request opcode is not supported.
    RequestNotSupported,
    /// The offset is past the end of the value.
    InvalidOffset,
    /// Authorization is required.
    InsufficientAuthorization,
    /// The prepare-write queue is full.
    PrepareQueueFull,
    /// Nothing matched.
    AttributeNotFound,
    /// The value cannot be read with Read Blob.
    AttributeNotLong,
    /// The encryption key is too short.
    InsufficientEncryptionKeySize,
    /// The value length is wrong.
    InvalidAttributeValueLength,
    /// An unspecified error.
    Unlikely,
    /// Encryption is required.
    InsufficientEncryption,
    /// The group type is not a grouping attribute.
    UnsupportedGroupType,
    /// The server is out of resources.
    InsufficientResources,
    /// Any other code (application range, or one not named here).
    Other(u8),
}

impl ErrorCode {
    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            ErrorCode::InvalidHandle => 0x01,
            ErrorCode::ReadNotPermitted => 0x02,
            ErrorCode::WriteNotPermitted => 0x03,
            ErrorCode::InvalidPdu => 0x04,
            ErrorCode::InsufficientAuthentication => 0x05,
            ErrorCode::RequestNotSupported => 0x06,
            ErrorCode::InvalidOffset => 0x07,
            ErrorCode::InsufficientAuthorization => 0x08,
            ErrorCode::PrepareQueueFull => 0x09,
            ErrorCode::AttributeNotFound => 0x0A,
            ErrorCode::AttributeNotLong => 0x0B,
            ErrorCode::InsufficientEncryptionKeySize => 0x0C,
            ErrorCode::InvalidAttributeValueLength => 0x0D,
            ErrorCode::Unlikely => 0x0E,
            ErrorCode::InsufficientEncryption => 0x0F,
            ErrorCode::UnsupportedGroupType => 0x10,
            ErrorCode::InsufficientResources => 0x11,
            ErrorCode::Other(v) => v,
        }
    }

    /// From the wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x01 => ErrorCode::InvalidHandle,
            0x02 => ErrorCode::ReadNotPermitted,
            0x03 => ErrorCode::WriteNotPermitted,
            0x04 => ErrorCode::InvalidPdu,
            0x05 => ErrorCode::InsufficientAuthentication,
            0x06 => ErrorCode::RequestNotSupported,
            0x07 => ErrorCode::InvalidOffset,
            0x08 => ErrorCode::InsufficientAuthorization,
            0x09 => ErrorCode::PrepareQueueFull,
            0x0A => ErrorCode::AttributeNotFound,
            0x0B => ErrorCode::AttributeNotLong,
            0x0C => ErrorCode::InsufficientEncryptionKeySize,
            0x0D => ErrorCode::InvalidAttributeValueLength,
            0x0E => ErrorCode::Unlikely,
            0x0F => ErrorCode::InsufficientEncryption,
            0x10 => ErrorCode::UnsupportedGroupType,
            0x11 => ErrorCode::InsufficientResources,
            other => ErrorCode::Other(other),
        }
    }
}

/// One entry of a Read By Group Type response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupEntry {
    /// First handle of the group.
    pub start: u16,
    /// Last handle of the group.
    pub end: u16,
    /// The group attribute's value (for a service, its UUID).
    pub value: Vec<u8>,
}

/// Why bytes did not decode to a [`Pdu`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No bytes at all.
    Empty,
    /// The opcode is not one this crate models.
    UnknownOpcode(u8),
    /// The opcode is known but the parameters are the wrong length or shape.
    Malformed(u8),
}

impl DecodeError {
    /// The opcode byte, if there was one.
    #[must_use]
    pub fn opcode(self) -> Option<u8> {
        match self {
            DecodeError::Empty => None,
            DecodeError::UnknownOpcode(o) | DecodeError::Malformed(o) => Some(o),
        }
    }
}

/// A decoded ATT PDU.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)] // field names are the protocol's; the variants say what they are
pub enum Pdu {
    ErrorRsp {
        request: u8,
        handle: u16,
        code: ErrorCode,
    },
    MtuReq {
        mtu: u16,
    },
    MtuRsp {
        mtu: u16,
    },
    FindInfoReq {
        start: u16,
        end: u16,
    },
    /// Entries all share one UUID width; the encoder takes the format from the first.
    FindInfoRsp {
        entries: Vec<(u16, Uuid)>,
    },
    FindByTypeValueReq {
        start: u16,
        end: u16,
        attr_type: u16,
        value: Vec<u8>,
    },
    /// `(found handle, group end handle)` pairs.
    FindByTypeValueRsp {
        ranges: Vec<(u16, u16)>,
    },
    ReadByTypeReq {
        start: u16,
        end: u16,
        attr_type: Uuid,
    },
    /// `(handle, value)` pairs; every value has the same length.
    ReadByTypeRsp {
        entries: Vec<(u16, Vec<u8>)>,
    },
    ReadReq {
        handle: u16,
    },
    ReadRsp {
        value: Vec<u8>,
    },
    ReadBlobReq {
        handle: u16,
        offset: u16,
    },
    ReadBlobRsp {
        value: Vec<u8>,
    },
    ReadMultipleReq {
        handles: Vec<u16>,
    },
    /// The values, concatenated.
    ReadMultipleRsp {
        values: Vec<u8>,
    },
    ReadByGroupTypeReq {
        start: u16,
        end: u16,
        group_type: Uuid,
    },
    /// Every entry's value has the same length.
    ReadByGroupTypeRsp {
        entries: Vec<GroupEntry>,
    },
    WriteReq {
        handle: u16,
        value: Vec<u8>,
    },
    WriteRsp,
    WriteCmd {
        handle: u16,
        value: Vec<u8>,
    },
    PrepareWriteReq {
        handle: u16,
        offset: u16,
        value: Vec<u8>,
    },
    PrepareWriteRsp {
        handle: u16,
        offset: u16,
        value: Vec<u8>,
    },
    ExecuteWriteReq {
        flags: u8,
    },
    ExecuteWriteRsp,
    Notification {
        handle: u16,
        value: Vec<u8>,
    },
    Indication {
        handle: u16,
        value: Vec<u8>,
    },
    Confirmation,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn put16(o: &mut Vec<u8>, v: u16) {
    o.extend_from_slice(&v.to_le_bytes());
}

impl Pdu {
    /// True for PDUs a client sends and the server must answer (a command is answered with
    /// nothing).
    #[must_use]
    pub fn is_request(&self) -> bool {
        matches!(
            self,
            Pdu::MtuReq { .. }
                | Pdu::FindInfoReq { .. }
                | Pdu::FindByTypeValueReq { .. }
                | Pdu::ReadByTypeReq { .. }
                | Pdu::ReadReq { .. }
                | Pdu::ReadBlobReq { .. }
                | Pdu::ReadMultipleReq { .. }
                | Pdu::ReadByGroupTypeReq { .. }
                | Pdu::WriteReq { .. }
                | Pdu::PrepareWriteReq { .. }
                | Pdu::ExecuteWriteReq { .. }
        )
    }

    /// Encode to wire bytes. A list response whose entries differ in width or value length has no
    /// encoding; the encoder stops at the first entry that differs from the first.
    #[must_use]
    #[allow(clippy::too_many_lines)] // one flat match, one arm per PDU
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Pdu::ErrorRsp {
                request,
                handle,
                code,
            } => {
                o.push(op::ERROR_RSP);
                o.push(*request);
                put16(&mut o, *handle);
                o.push(code.to_u8());
            }
            Pdu::MtuReq { mtu } => {
                o.push(op::MTU_REQ);
                put16(&mut o, *mtu);
            }
            Pdu::MtuRsp { mtu } => {
                o.push(op::MTU_RSP);
                put16(&mut o, *mtu);
            }
            Pdu::FindInfoReq { start, end } => {
                o.push(op::FIND_INFO_REQ);
                put16(&mut o, *start);
                put16(&mut o, *end);
            }
            Pdu::FindInfoRsp { entries } => {
                o.push(op::FIND_INFO_RSP);
                let wide = entries.first().is_some_and(|(_, u)| u.wire_len() == 16);
                o.push(if wide { 2 } else { 1 });
                for (h, u) in entries {
                    if (u.wire_len() == 16) != wide {
                        break;
                    }
                    put16(&mut o, *h);
                    o.extend_from_slice(&u.to_le_bytes());
                }
            }
            Pdu::FindByTypeValueReq {
                start,
                end,
                attr_type,
                value,
            } => {
                o.push(op::FIND_BY_TYPE_VALUE_REQ);
                put16(&mut o, *start);
                put16(&mut o, *end);
                put16(&mut o, *attr_type);
                o.extend_from_slice(value);
            }
            Pdu::FindByTypeValueRsp { ranges } => {
                o.push(op::FIND_BY_TYPE_VALUE_RSP);
                for (a, b) in ranges {
                    put16(&mut o, *a);
                    put16(&mut o, *b);
                }
            }
            Pdu::ReadByTypeReq {
                start,
                end,
                attr_type,
            } => {
                o.push(op::READ_BY_TYPE_REQ);
                put16(&mut o, *start);
                put16(&mut o, *end);
                o.extend_from_slice(&attr_type.to_le_bytes());
            }
            Pdu::ReadByTypeRsp { entries } => {
                o.push(op::READ_BY_TYPE_RSP);
                let vlen = entries.first().map_or(0, |(_, v)| v.len());
                o.push(u8::try_from(2 + vlen).unwrap_or(u8::MAX));
                for (h, v) in entries {
                    if v.len() != vlen {
                        break;
                    }
                    put16(&mut o, *h);
                    o.extend_from_slice(v);
                }
            }
            Pdu::ReadReq { handle } => {
                o.push(op::READ_REQ);
                put16(&mut o, *handle);
            }
            Pdu::ReadRsp { value } => {
                o.push(op::READ_RSP);
                o.extend_from_slice(value);
            }
            Pdu::ReadBlobReq { handle, offset } => {
                o.push(op::READ_BLOB_REQ);
                put16(&mut o, *handle);
                put16(&mut o, *offset);
            }
            Pdu::ReadBlobRsp { value } => {
                o.push(op::READ_BLOB_RSP);
                o.extend_from_slice(value);
            }
            Pdu::ReadMultipleReq { handles } => {
                o.push(op::READ_MULTIPLE_REQ);
                for h in handles {
                    put16(&mut o, *h);
                }
            }
            Pdu::ReadMultipleRsp { values } => {
                o.push(op::READ_MULTIPLE_RSP);
                o.extend_from_slice(values);
            }
            Pdu::ReadByGroupTypeReq {
                start,
                end,
                group_type,
            } => {
                o.push(op::READ_BY_GROUP_TYPE_REQ);
                put16(&mut o, *start);
                put16(&mut o, *end);
                o.extend_from_slice(&group_type.to_le_bytes());
            }
            Pdu::ReadByGroupTypeRsp { entries } => {
                o.push(op::READ_BY_GROUP_TYPE_RSP);
                let vlen = entries.first().map_or(0, |e| e.value.len());
                o.push(u8::try_from(4 + vlen).unwrap_or(u8::MAX));
                for e in entries {
                    if e.value.len() != vlen {
                        break;
                    }
                    put16(&mut o, e.start);
                    put16(&mut o, e.end);
                    o.extend_from_slice(&e.value);
                }
            }
            Pdu::WriteReq { handle, value } => {
                o.push(op::WRITE_REQ);
                put16(&mut o, *handle);
                o.extend_from_slice(value);
            }
            Pdu::WriteRsp => o.push(op::WRITE_RSP),
            Pdu::WriteCmd { handle, value } => {
                o.push(op::WRITE_CMD);
                put16(&mut o, *handle);
                o.extend_from_slice(value);
            }
            Pdu::PrepareWriteReq {
                handle,
                offset,
                value,
            } => {
                o.push(op::PREPARE_WRITE_REQ);
                put16(&mut o, *handle);
                put16(&mut o, *offset);
                o.extend_from_slice(value);
            }
            Pdu::PrepareWriteRsp {
                handle,
                offset,
                value,
            } => {
                o.push(op::PREPARE_WRITE_RSP);
                put16(&mut o, *handle);
                put16(&mut o, *offset);
                o.extend_from_slice(value);
            }
            Pdu::ExecuteWriteReq { flags } => {
                o.push(op::EXECUTE_WRITE_REQ);
                o.push(*flags);
            }
            Pdu::ExecuteWriteRsp => o.push(op::EXECUTE_WRITE_RSP),
            Pdu::Notification { handle, value } => {
                o.push(op::NOTIFICATION);
                put16(&mut o, *handle);
                o.extend_from_slice(value);
            }
            Pdu::Indication { handle, value } => {
                o.push(op::INDICATION);
                put16(&mut o, *handle);
                o.extend_from_slice(value);
            }
            Pdu::Confirmation => o.push(op::CONFIRMATION),
        }
        o
    }

    /// Decode wire bytes.
    ///
    /// # Errors
    /// [`DecodeError`] for an empty input, an opcode not modelled here, or parameters of the wrong
    /// length.
    #[allow(clippy::too_many_lines)] // one flat match, one arm per PDU
    pub fn decode(b: &[u8]) -> Result<Pdu, DecodeError> {
        let Some(&opc) = b.first() else {
            return Err(DecodeError::Empty);
        };
        let bad = DecodeError::Malformed(opc);
        let p = &b[1..];
        let exact = |n: usize| if p.len() == n { Ok(()) } else { Err(bad) };
        let at_least = |n: usize| if p.len() >= n { Ok(()) } else { Err(bad) };
        Ok(match opc {
            op::ERROR_RSP => {
                exact(4)?;
                Pdu::ErrorRsp {
                    request: p[0],
                    handle: u16_at(p, 1),
                    code: ErrorCode::from_u8(p[3]),
                }
            }
            op::MTU_REQ => {
                exact(2)?;
                Pdu::MtuReq { mtu: u16_at(p, 0) }
            }
            op::MTU_RSP => {
                exact(2)?;
                Pdu::MtuRsp { mtu: u16_at(p, 0) }
            }
            op::FIND_INFO_REQ => {
                exact(4)?;
                Pdu::FindInfoReq {
                    start: u16_at(p, 0),
                    end: u16_at(p, 2),
                }
            }
            op::FIND_INFO_RSP => {
                at_least(1)?;
                let w = match p[0] {
                    1 => 2,
                    2 => 16,
                    _ => return Err(bad),
                };
                let body = &p[1..];
                if body.len() % (2 + w) != 0 {
                    return Err(bad);
                }
                let entries = body
                    .chunks(2 + w)
                    .map(|c| Uuid::from_le_bytes(&c[2..]).map(|u| (u16_at(c, 0), u)))
                    .collect::<Option<Vec<_>>>()
                    .ok_or(bad)?;
                Pdu::FindInfoRsp { entries }
            }
            op::FIND_BY_TYPE_VALUE_REQ => {
                at_least(6)?;
                Pdu::FindByTypeValueReq {
                    start: u16_at(p, 0),
                    end: u16_at(p, 2),
                    attr_type: u16_at(p, 4),
                    value: p[6..].to_vec(),
                }
            }
            op::FIND_BY_TYPE_VALUE_RSP => {
                if p.len() % 4 != 0 {
                    return Err(bad);
                }
                Pdu::FindByTypeValueRsp {
                    ranges: p.chunks(4).map(|c| (u16_at(c, 0), u16_at(c, 2))).collect(),
                }
            }
            op::READ_BY_TYPE_REQ => {
                at_least(4)?;
                let attr_type = Uuid::from_le_bytes(&p[4..]).ok_or(bad)?;
                Pdu::ReadByTypeReq {
                    start: u16_at(p, 0),
                    end: u16_at(p, 2),
                    attr_type,
                }
            }
            op::READ_BY_TYPE_RSP => {
                at_least(1)?;
                let n = usize::from(p[0]);
                if n < 2 || (p.len() - 1) % n != 0 {
                    return Err(bad);
                }
                Pdu::ReadByTypeRsp {
                    entries: p[1..]
                        .chunks(n)
                        .map(|c| (u16_at(c, 0), c[2..].to_vec()))
                        .collect(),
                }
            }
            op::READ_REQ => {
                exact(2)?;
                Pdu::ReadReq {
                    handle: u16_at(p, 0),
                }
            }
            op::READ_RSP => Pdu::ReadRsp { value: p.to_vec() },
            op::READ_BLOB_REQ => {
                exact(4)?;
                Pdu::ReadBlobReq {
                    handle: u16_at(p, 0),
                    offset: u16_at(p, 2),
                }
            }
            op::READ_BLOB_RSP => Pdu::ReadBlobRsp { value: p.to_vec() },
            op::READ_MULTIPLE_REQ => {
                if p.len() < 4 || p.len() % 2 != 0 {
                    return Err(bad);
                }
                Pdu::ReadMultipleReq {
                    handles: p.chunks(2).map(|c| u16_at(c, 0)).collect(),
                }
            }
            op::READ_MULTIPLE_RSP => Pdu::ReadMultipleRsp { values: p.to_vec() },
            op::READ_BY_GROUP_TYPE_REQ => {
                at_least(4)?;
                let group_type = Uuid::from_le_bytes(&p[4..]).ok_or(bad)?;
                Pdu::ReadByGroupTypeReq {
                    start: u16_at(p, 0),
                    end: u16_at(p, 2),
                    group_type,
                }
            }
            op::READ_BY_GROUP_TYPE_RSP => {
                at_least(1)?;
                let n = usize::from(p[0]);
                if n < 4 || (p.len() - 1) % n != 0 {
                    return Err(bad);
                }
                Pdu::ReadByGroupTypeRsp {
                    entries: p[1..]
                        .chunks(n)
                        .map(|c| GroupEntry {
                            start: u16_at(c, 0),
                            end: u16_at(c, 2),
                            value: c[4..].to_vec(),
                        })
                        .collect(),
                }
            }
            op::WRITE_REQ => {
                at_least(2)?;
                Pdu::WriteReq {
                    handle: u16_at(p, 0),
                    value: p[2..].to_vec(),
                }
            }
            op::WRITE_RSP => {
                exact(0)?;
                Pdu::WriteRsp
            }
            op::WRITE_CMD => {
                at_least(2)?;
                Pdu::WriteCmd {
                    handle: u16_at(p, 0),
                    value: p[2..].to_vec(),
                }
            }
            op::PREPARE_WRITE_REQ => {
                at_least(4)?;
                Pdu::PrepareWriteReq {
                    handle: u16_at(p, 0),
                    offset: u16_at(p, 2),
                    value: p[4..].to_vec(),
                }
            }
            op::PREPARE_WRITE_RSP => {
                at_least(4)?;
                Pdu::PrepareWriteRsp {
                    handle: u16_at(p, 0),
                    offset: u16_at(p, 2),
                    value: p[4..].to_vec(),
                }
            }
            op::EXECUTE_WRITE_REQ => {
                exact(1)?;
                Pdu::ExecuteWriteReq { flags: p[0] }
            }
            op::EXECUTE_WRITE_RSP => {
                exact(0)?;
                Pdu::ExecuteWriteRsp
            }
            op::NOTIFICATION => {
                at_least(2)?;
                Pdu::Notification {
                    handle: u16_at(p, 0),
                    value: p[2..].to_vec(),
                }
            }
            op::INDICATION => {
                at_least(2)?;
                Pdu::Indication {
                    handle: u16_at(p, 0),
                    value: p[2..].to_vec(),
                }
            }
            op::CONFIRMATION => {
                exact(0)?;
                Pdu::Confirmation
            }
            other => return Err(DecodeError::UnknownOpcode(other)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(p: &Pdu) {
        assert_eq!(&Pdu::decode(&p.encode()).unwrap(), p);
    }

    #[test]
    fn exact_wire_bytes() {
        assert_eq!(Pdu::MtuReq { mtu: 185 }.encode(), [0x02, 0xB9, 0x00]);
        assert_eq!(
            Pdu::ErrorRsp {
                request: 0x0A,
                handle: 0x0102,
                code: ErrorCode::InvalidHandle
            }
            .encode(),
            [0x01, 0x0A, 0x02, 0x01, 0x01]
        );
        assert_eq!(Pdu::ReadReq { handle: 3 }.encode(), [0x0A, 3, 0]);
        assert_eq!(
            Pdu::WriteCmd {
                handle: 7,
                value: vec![9]
            }
            .encode(),
            [0x52, 7, 0, 9]
        );
        assert_eq!(
            Pdu::ReadByGroupTypeReq {
                start: 1,
                end: 0xFFFF,
                group_type: Uuid::from_u16(0x2800)
            }
            .encode(),
            [0x10, 1, 0, 0xFF, 0xFF, 0x00, 0x28]
        );
    }

    #[test]
    fn every_variant_round_trips() {
        let u = Uuid::from_u128(0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E);
        for p in [
            Pdu::ErrorRsp {
                request: 1,
                handle: 2,
                code: ErrorCode::Other(0xFD),
            },
            Pdu::MtuReq { mtu: 247 },
            Pdu::MtuRsp { mtu: 23 },
            Pdu::FindInfoReq { start: 1, end: 9 },
            Pdu::FindInfoRsp {
                entries: vec![(1, Uuid::from_u16(0x2800)), (2, Uuid::from_u16(0x2803))],
            },
            Pdu::FindInfoRsp {
                entries: vec![(1, u), (2, u)],
            },
            Pdu::FindByTypeValueReq {
                start: 1,
                end: 9,
                attr_type: 0x2800,
                value: vec![1, 2],
            },
            Pdu::FindByTypeValueRsp {
                ranges: vec![(1, 4), (5, 9)],
            },
            Pdu::ReadByTypeReq {
                start: 1,
                end: 9,
                attr_type: u,
            },
            Pdu::ReadByTypeRsp {
                entries: vec![(1, vec![1, 2, 3]), (2, vec![4, 5, 6])],
            },
            Pdu::ReadReq { handle: 1 },
            Pdu::ReadRsp { value: vec![1, 2] },
            Pdu::ReadBlobReq {
                handle: 1,
                offset: 22,
            },
            Pdu::ReadBlobRsp { value: vec![] },
            Pdu::ReadMultipleReq {
                handles: vec![1, 2],
            },
            Pdu::ReadMultipleRsp {
                values: vec![1, 2, 3],
            },
            Pdu::ReadByGroupTypeReq {
                start: 1,
                end: 9,
                group_type: Uuid::from_u16(0x2800),
            },
            Pdu::ReadByGroupTypeRsp {
                entries: vec![GroupEntry {
                    start: 1,
                    end: 4,
                    value: vec![1, 2],
                }],
            },
            Pdu::WriteReq {
                handle: 1,
                value: vec![1],
            },
            Pdu::WriteRsp,
            Pdu::WriteCmd {
                handle: 1,
                value: vec![],
            },
            Pdu::PrepareWriteReq {
                handle: 1,
                offset: 2,
                value: vec![3],
            },
            Pdu::PrepareWriteRsp {
                handle: 1,
                offset: 2,
                value: vec![3],
            },
            Pdu::ExecuteWriteReq { flags: 1 },
            Pdu::ExecuteWriteRsp,
            Pdu::Notification {
                handle: 1,
                value: vec![1, 2, 3],
            },
            Pdu::Indication {
                handle: 1,
                value: vec![],
            },
            Pdu::Confirmation,
        ] {
            rt(&p);
        }
    }

    #[test]
    fn find_info_format_byte_follows_uuid_width() {
        let narrow = Pdu::FindInfoRsp {
            entries: vec![(1, Uuid::from_u16(0x2902))],
        }
        .encode();
        assert_eq!(narrow, [0x05, 1, 1, 0, 0x02, 0x29]);
        let wide = Pdu::FindInfoRsp {
            entries: vec![(1, Uuid::from_u128(1))],
        }
        .encode();
        assert_eq!(wide[1], 2);
        assert_eq!(wide.len(), 2 + 2 + 16);
    }

    #[test]
    fn malformed_pdus_are_refused() {
        assert_eq!(Pdu::decode(&[]), Err(DecodeError::Empty));
        assert_eq!(Pdu::decode(&[0x02, 1]), Err(DecodeError::Malformed(0x02)));
        assert_eq!(
            Pdu::decode(&[0x0A, 1, 0, 0]),
            Err(DecodeError::Malformed(0x0A))
        );
        assert_eq!(
            Pdu::decode(&[0x08, 1, 0, 2, 0, 1, 2, 3]),
            Err(DecodeError::Malformed(0x08))
        );
        assert_eq!(Pdu::decode(&[0x09, 0]), Err(DecodeError::Malformed(0x09)));
        assert_eq!(
            Pdu::decode(&[0x09, 4, 1, 0, 1]),
            Err(DecodeError::Malformed(0x09))
        );
        assert_eq!(
            Pdu::decode(&[0x0E, 1, 0]),
            Err(DecodeError::Malformed(0x0E))
        );
        assert_eq!(
            Pdu::decode(&[0x0E, 1, 0, 2]),
            Err(DecodeError::Malformed(0x0E))
        );
        assert_eq!(Pdu::decode(&[0x05, 3]), Err(DecodeError::Malformed(0x05)));
        assert_eq!(Pdu::decode(&[0xEE]), Err(DecodeError::UnknownOpcode(0xEE)));
    }

    #[test]
    fn error_codes_round_trip() {
        for v in 0..=0x20_u8 {
            assert_eq!(ErrorCode::from_u8(v).to_u8(), v);
        }
    }
}
