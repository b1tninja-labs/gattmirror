//! Advertising data encoding and the advertising parameter block.
//!
//! Anchors: Bluetooth Core Specification 6.3 (final, 2026-03), Vol 4 Part E §7.8.5 (parameter
//! block), §7.8.7 and §7.8.8 (a length byte then 31 octets, length 0 to 31, default all zero). The
//! AD type numbers and the Flags bit values live in the Core Specification Supplement and the
//! Assigned Numbers, which are not in the corpus: they stay `[unread]`.

use core::fmt;

/// The most data a legacy advertising or scan response packet holds.
pub const MAX_AD_LEN: usize = 31;

const AD_FLAGS: u8 = 0x01;
const AD_SHORT_NAME: u8 = 0x08;
const AD_COMPLETE_NAME: u8 = 0x09;
const AD_UUID16_COMPLETE: u8 = 0x03;
const AD_UUID128_COMPLETE: u8 = 0x07;
/// LE General Discoverable plus "BR/EDR not supported". `[unread]`: the Flags bit values are in the
/// Core Specification Supplement, which the corpus does not hold.
const FLAGS_LE_GENERAL_NO_BREDR: u8 = 0x06;

/// A service UUID to advertise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceUuid {
    /// A 16-bit UUID.
    Uuid16(u16),
    /// A 128-bit UUID in the byte order the text form gives (most significant byte first).
    Uuid128([u8; 16]),
}

/// Why an advertisement could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The mandatory parts alone (flags and the service UUID) exceed the 31-byte limit.
    TooLong,
    /// The name is empty.
    EmptyName,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => f.write_str("advertising data over 31 bytes"),
            Self::EmptyName => f.write_str("empty device name"),
        }
    }
}

impl std::error::Error for Error {}

/// A connectable advertisement: a device name and optionally a service UUID.
///
/// Layout: the advertising packet carries flags, the service UUID and the name (shortened if it
/// does not fit); the scan response carries the name, complete if it fits in 29 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    name: String,
    service: Option<ServiceUuid>,
}

impl Advertisement {
    /// Build an advertisement.
    ///
    /// # Errors
    ///
    /// [`Error::EmptyName`] for an empty name.
    pub fn new(name: &str, service: Option<ServiceUuid>) -> Result<Self, Error> {
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        Ok(Self {
            name: name.to_owned(),
            service,
        })
    }

    /// The advertising data (at most 31 bytes, unpadded).
    ///
    /// # Errors
    ///
    /// [`Error::TooLong`] if flags and the UUID alone do not fit.
    pub fn advertising_data(&self) -> Result<Vec<u8>, Error> {
        let mut out = structure(AD_FLAGS, &[FLAGS_LE_GENERAL_NO_BREDR]);
        match self.service {
            Some(ServiceUuid::Uuid16(u)) => {
                out.extend(structure(AD_UUID16_COMPLETE, &u.to_le_bytes()));
            }
            Some(ServiceUuid::Uuid128(mut u)) => {
                u.reverse();
                out.extend(structure(AD_UUID128_COMPLETE, &u));
            }
            None => {}
        }
        if out.len() > MAX_AD_LEN {
            return Err(Error::TooLong);
        }
        let room = MAX_AD_LEN - out.len();
        if room > 2 {
            out.extend(name_structure(&self.name, room));
        }
        Ok(out)
    }

    /// The scan response data (at most 31 bytes, unpadded).
    #[must_use]
    pub fn scan_response(&self) -> Vec<u8> {
        name_structure(&self.name, MAX_AD_LEN)
    }
}

fn structure(ad_type: u8, data: &[u8]) -> Vec<u8> {
    let len = u8::try_from(data.len() + 1).expect("structure over 255 bytes");
    let mut v = vec![len, ad_type];
    v.extend_from_slice(data);
    v
}

/// The name as an AD structure within `room` bytes: complete if it fits, else shortened.
fn name_structure(name: &str, room: usize) -> Vec<u8> {
    let bytes = name.as_bytes();
    let max = room - 2;
    if bytes.len() <= max {
        structure(AD_COMPLETE_NAME, bytes)
    } else {
        structure(AD_SHORT_NAME, &bytes[..max])
    }
}

/// The 15-byte parameter block of LE Set Advertising Parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvParams {
    /// Minimum interval in 0.625 ms units.
    pub interval_min: u16,
    /// Maximum interval in 0.625 ms units.
    pub interval_max: u16,
}

impl Default for AdvParams {
    /// 50 ms.
    fn default() -> Self {
        Self {
            interval_min: 0x0050,
            interval_max: 0x0050,
        }
    }
}

impl AdvParams {
    /// Encode for a connectable undirected advertisement, public own address, all three channels,
    /// no filter. Core 6.3 §7.8.5 order: interval min and max (2 each, units of 0.625 ms, range
    /// `0x0020..=0x4000`), `Advertising_Type` (`0x00` is `ADV_IND`, connectable and scannable),
    /// `Own_Address_Type` (`0x00` public), `Peer_Address_Type`, `Peer_Address` (6), channel map
    /// (`0x07` is channels 37, 38 and 39) and filter policy (`0x00` accepts everyone).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(15);
        v.extend_from_slice(&self.interval_min.to_le_bytes());
        v.extend_from_slice(&self.interval_max.to_le_bytes());
        v.push(0x00); // connectable undirected
        v.push(0x00); // own address type: public
        v.push(0x00); // peer address type
        v.extend_from_slice(&[0; 6]); // peer address, unused
        v.push(0x07); // channel map: 37, 38, 39
        v.push(0x00); // filter policy: none
        v
    }
}

/// Lay data out as the data commands take it: a length byte and 31 data bytes, zero padded.
pub(crate) fn padded(data: &[u8]) -> Vec<u8> {
    debug_assert!(data.len() <= MAX_AD_LEN);
    let mut v = Vec::with_capacity(MAX_AD_LEN + 1);
    v.push(u8::try_from(data.len()).expect("data over 255 bytes"));
    v.extend_from_slice(data);
    v.resize(MAX_AD_LEN + 1, 0);
    v
}

#[cfg(test)]
mod tests {
    use super::{AdvParams, Advertisement, Error, ServiceUuid, padded};

    #[test]
    fn data_has_flags_uuid_and_name() {
        let a = Advertisement::new("TEST", Some(ServiceUuid::Uuid16(0xFFF0))).unwrap();
        assert_eq!(
            a.advertising_data().unwrap(),
            [
                2, 0x01, 0x06, 3, 0x03, 0xF0, 0xFF, 5, 0x09, b'T', b'E', b'S', b'T'
            ]
        );
        assert_eq!(a.scan_response(), [5, 0x09, b'T', b'E', b'S', b'T']);
    }

    #[test]
    fn a_128_bit_uuid_is_sent_least_significant_byte_first() {
        let mut u = [0u8; 16];
        u[0] = 0xAA;
        u[15] = 0xBB;
        let a = Advertisement::new("N", Some(ServiceUuid::Uuid128(u))).unwrap();
        let d = a.advertising_data().unwrap();
        assert_eq!(&d[3..5], [17, 0x07]);
        assert_eq!(d[5], 0xBB);
        assert_eq!(d[20], 0xAA);
        // flags 3 + uuid 18 = 21, leaving 10 bytes: the name fits complete.
        assert_eq!(&d[21..], [2, 0x09, b'N']);
    }

    #[test]
    fn a_long_name_is_shortened_to_fit() {
        let name = "A".repeat(40);
        let a = Advertisement::new(&name, Some(ServiceUuid::Uuid16(1))).unwrap();
        let d = a.advertising_data().unwrap();
        assert_eq!(d.len(), 31);
        assert_eq!(d[8], 0x08);
        let s = a.scan_response();
        assert_eq!((s.len(), s[1]), (31, 0x08));
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(Advertisement::new("", None), Err(Error::EmptyName));
    }

    #[test]
    fn parameter_block_is_fifteen_bytes() {
        let p = AdvParams {
            interval_min: 0x0020,
            interval_max: 0x0040,
        }
        .encode();
        assert_eq!(p.len(), 15);
        assert_eq!(&p[..5], [0x20, 0x00, 0x40, 0x00, 0x00]);
        // Own address type public, peer type and address unused, channel map, filter policy.
        assert_eq!(p[5], 0x00);
        assert_eq!(&p[6..13], [0; 7]);
        assert_eq!(p[13], 0x07);
        assert_eq!(p[14], 0x00);
    }

    #[test]
    fn data_commands_carry_a_length_and_31_padded_bytes() {
        let p = padded(&[1, 2, 3]);
        assert_eq!(p.len(), 32);
        assert_eq!(&p[..5], [3, 1, 2, 3, 0]);
    }
}
