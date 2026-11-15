use std::fmt;
use std::str::FromStr;

/// The Bluetooth base UUID, `0000xxxx-0000-1000-8000-00805f9b34fb`, with the `xxxx` field zero.
pub const BASE_UUID: Uuid = Uuid(0x0000_0000_0000_1000_8000_0080_5f9b_34fb);

/// A 128-bit UUID. A 16-bit short form is an alias for the base UUID with that value in bits 96..112.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid(u128);

/// Why a string is not a UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseUuidError {
    /// The length is neither 4 (short form) nor 36 (dashed).
    Length,
    /// A character is not a hexadecimal digit, or a dash is misplaced.
    Character,
}

impl fmt::Display for ParseUuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length => f.write_str("a UUID is 4 hex digits or 36 characters with dashes"),
            Self::Character => f.write_str("a UUID holds only hex digits and four dashes"),
        }
    }
}

impl std::error::Error for ParseUuidError {}

impl Uuid {
    /// Build from the 128-bit value.
    #[must_use]
    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    /// Expand a 16-bit short UUID onto the base UUID.
    #[must_use]
    pub const fn from_u16(short: u16) -> Self {
        Self(BASE_UUID.0 | ((short as u128) << 96))
    }

    /// The 128-bit value.
    #[must_use]
    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// The 16-bit short form, if this UUID is the base UUID with only that field set.
    #[must_use]
    pub const fn as_u16(self) -> Option<u16> {
        let rest = self.0 & !(0xffff_u128 << 96);
        if rest == BASE_UUID.0 {
            #[allow(clippy::cast_possible_truncation)]
            Some((self.0 >> 96) as u16)
        } else {
            None
        }
    }

    /// Little-endian bytes as they appear in an attribute value: 2 bytes for a short UUID, else 16.
    #[must_use]
    pub fn to_le_bytes(self) -> Vec<u8> {
        match self.as_u16() {
            Some(short) => short.to_le_bytes().to_vec(),
            None => self.0.to_le_bytes().to_vec(),
        }
    }
}

impl fmt::Display for Uuid {
    /// Four lowercase hex digits for a short UUID, otherwise the dashed 36-character form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(short) = self.as_u16() {
            return write!(f, "{short:04x}");
        }
        let v = self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            v >> 96,
            (v >> 80) & 0xffff,
            (v >> 64) & 0xffff,
            (v >> 48) & 0xffff,
            v & 0xffff_ffff_ffff
        )
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({self})")
    }
}

impl FromStr for Uuid {
    type Err = ParseUuidError;

    /// Accepts 4 hex digits or the dashed 36-character form, in either case.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        match bytes.len() {
            4 => {
                let short = u16::from_str_radix(s, 16).map_err(|_| ParseUuidError::Character)?;
                if bytes.iter().all(u8::is_ascii_hexdigit) {
                    Ok(Self::from_u16(short))
                } else {
                    Err(ParseUuidError::Character)
                }
            }
            36 => {
                let mut value: u128 = 0;
                for (i, &b) in bytes.iter().enumerate() {
                    if matches!(i, 8 | 13 | 18 | 23) {
                        if b != b'-' {
                            return Err(ParseUuidError::Character);
                        }
                        continue;
                    }
                    let digit = (b as char).to_digit(16).ok_or(ParseUuidError::Character)?;
                    value = (value << 4) | u128::from(digit);
                }
                Ok(Self(value))
            }
            _ => Err(ParseUuidError::Length),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_expands_onto_the_base_uuid() {
        let u = Uuid::from_u16(0xfff0);
        assert_eq!(u.as_u128(), 0x0000_fff0_0000_1000_8000_0080_5f9b_34fb);
        assert_eq!("fff0".parse::<Uuid>().unwrap(), u);
        assert_eq!(
            "0000fff0-0000-1000-8000-00805f9b34fb"
                .parse::<Uuid>()
                .unwrap(),
            u
        );
    }

    #[test]
    fn short_displays_short_and_long_displays_dashed() {
        assert_eq!(Uuid::from_u16(0x2902).to_string(), "2902");
        let nus = "6E400001-B5A3-F393-E0A9-E50E24DCCA9E"
            .parse::<Uuid>()
            .unwrap();
        assert_eq!(nus.to_string(), "6e400001-b5a3-f393-e0a9-e50e24dcca9e");
        assert_eq!(nus.as_u16(), None);
    }

    #[test]
    fn a_base_uuid_with_other_bits_set_is_not_short() {
        let almost = Uuid::from_u128(Uuid::from_u16(0x1800).as_u128() | 1 << 90);
        assert_eq!(almost.as_u16(), None);
        assert_eq!(almost.to_string().len(), 36);
    }

    #[test]
    fn le_bytes_are_two_or_sixteen() {
        assert_eq!(Uuid::from_u16(0x2803).to_le_bytes(), vec![0x03, 0x28]);
        let long = Uuid::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        assert_eq!(long.to_le_bytes()[0], 0x10);
        assert_eq!(long.to_le_bytes().len(), 16);
    }

    #[test]
    fn malformed_uuids_are_rejected() {
        assert_eq!("fff".parse::<Uuid>(), Err(ParseUuidError::Length));
        assert_eq!("fffg".parse::<Uuid>(), Err(ParseUuidError::Character));
        assert_eq!("+fff".parse::<Uuid>(), Err(ParseUuidError::Character));
        assert_eq!(
            "0000fff0_0000-1000-8000-00805f9b34fb".parse::<Uuid>(),
            Err(ParseUuidError::Character)
        );
        assert_eq!(
            "0000fff0-0000-1000-8000-00805f9b34fz".parse::<Uuid>(),
            Err(ParseUuidError::Character)
        );
    }
}
