use std::fmt;

/// The Bluetooth base UUID, `00000000-0000-1000-8000-00805F9B34FB`, as a number.
const BASE: u128 = 0x0000_0000_0000_1000_8000_0080_5F9B_34FB;

/// A 16-bit or 128-bit UUID. A 128-bit value built on the Bluetooth base UUID is the same
/// `Uuid` as its 16-bit short form, so comparisons never depend on how it arrived.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid(u128);

impl Uuid {
    /// A 16-bit UUID.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        Uuid(BASE | ((v as u128) << 96))
    }

    /// A full 128-bit UUID, written most-significant byte first as people print it.
    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Uuid(v)
    }

    /// The 16-bit short form, if this UUID is on the base.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // the bits above 16 were just checked to be the base
    pub const fn as_u16(self) -> Option<u16> {
        if self.0 & !(0xFFFF_u128 << 96) == BASE {
            Some((self.0 >> 96) as u16)
        } else {
            None
        }
    }

    /// The 128-bit value.
    #[must_use]
    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// Parse the wire form: 2 bytes or 16 bytes, little-endian. Any other length is `None`.
    #[must_use]
    pub fn from_le_bytes(b: &[u8]) -> Option<Self> {
        match b.len() {
            2 => Some(Self::from_u16(u16::from_le_bytes([b[0], b[1]]))),
            16 => Some(Self(u128::from_le_bytes(b.try_into().ok()?))),
            _ => None,
        }
    }

    /// The wire form: 2 bytes if the UUID is on the base, else 16, little-endian.
    #[must_use]
    pub fn to_le_bytes(self) -> Vec<u8> {
        match self.as_u16() {
            Some(v) => v.to_le_bytes().to_vec(),
            None => self.0.to_le_bytes().to_vec(),
        }
    }

    /// 2 or 16: the length of [`to_le_bytes`](Self::to_le_bytes).
    #[must_use]
    pub const fn wire_len(self) -> usize {
        if self.as_u16().is_some() { 2 } else { 16 }
    }
}

impl From<u16> for Uuid {
    fn from(v: u16) -> Self {
        Self::from_u16(v)
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(v) = self.as_u16() {
            return write!(f, "Uuid({v:#06x})");
        }
        let v = self.0;
        let part = |shift: u32, mask: u128| v >> shift & mask;
        write!(
            f,
            "Uuid({:08x}-{:04x}-{:04x}-{:04x}-{:012x})",
            part(96, 0xFFFF_FFFF),
            part(80, 0xFFFF),
            part(64, 0xFFFF),
            part(48, 0xFFFF),
            part(0, 0xFFFF_FFFF_FFFF)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_round_trips() {
        let u = Uuid::from_u16(0x2902);
        assert_eq!(u.as_u16(), Some(0x2902));
        assert_eq!(u.to_le_bytes(), [0x02, 0x29]);
        assert_eq!(Uuid::from_le_bytes(&[0x02, 0x29]), Some(u));
    }

    #[test]
    fn base_form_equals_short_form() {
        let wide = Uuid::from_u128(0x0000_FFE1_0000_1000_8000_0080_5F9B_34FB);
        assert_eq!(wide, Uuid::from_u16(0xFFE1));
        assert_eq!(wide.wire_len(), 2);
    }

    #[test]
    fn vendor_uuid_stays_wide() {
        let v = Uuid::from_u128(0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E);
        assert_eq!(v.as_u16(), None);
        let bytes = v.to_le_bytes();
        assert_eq!(bytes.len(), 16);
        assert_eq!(bytes[0], 0x9E);
        assert_eq!(Uuid::from_le_bytes(&bytes), Some(v));
    }

    #[test]
    fn odd_lengths_are_refused() {
        assert_eq!(Uuid::from_le_bytes(&[1, 2, 3, 4]), None);
        assert_eq!(Uuid::from_le_bytes(&[]), None);
    }
}
