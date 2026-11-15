//! 128-bit Bluetooth UUIDs with the 16- and 32-bit short forms.
//!
//! The short forms are shorthand for `xxxxxxxx-0000-1000-8000-00805F9B34FB`; that base value is what a
//! Windows GATT stack reports, not read from the specification (`[unread]`).

use std::fmt;
use std::str::FromStr;

use crate::error::Error;

const BASE: u128 = 0x0000_0000_0000_1000_8000_0080_5F9B_34FB;
const BASE_MASK: u128 = 0x0000_0000_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;

/// A 128-bit UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid(pub u128);

impl Uuid {
    /// Expand a 16-bit short UUID.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        Self(BASE | ((v as u128) << 96))
    }

    /// Expand a 32-bit short UUID.
    #[must_use]
    pub const fn from_u32(v: u32) -> Self {
        Self(BASE | ((v as u128) << 96))
    }

    /// The 16-bit short form, if this UUID is derived from the base.
    #[must_use]
    pub const fn as_u16(self) -> Option<u16> {
        match self.as_u32() {
            Some(v) if v <= 0xFFFF => {
                Some(v.to_le_bytes()[0] as u16 | (v.to_le_bytes()[1] as u16) << 8)
            }
            _ => None,
        }
    }

    /// The 32-bit short form, if this UUID is derived from the base.
    #[must_use]
    pub const fn as_u32(self) -> Option<u32> {
        if self.0 & BASE_MASK == BASE {
            Some((self.0 >> 96) as u32)
        } else {
            None
        }
    }

    /// Shortest textual form: 4 hex digits, 8 hex digits, or the full dashed form.
    #[must_use]
    pub fn short_string(self) -> String {
        if let Some(v) = self.as_u16() {
            format!("{v:04x}")
        } else if let Some(v) = self.as_u32() {
            format!("{v:08x}")
        } else {
            self.to_string()
        }
    }
}

impl fmt::Display for Uuid {
    /// Canonical lower-case `8-4-4-4-12`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (v >> 96) & 0xFFFF_FFFF,
            (v >> 80) & 0xFFFF,
            (v >> 64) & 0xFFFF,
            (v >> 48) & 0xFFFF,
            v & 0xFFFF_FFFF_FFFF
        )
    }
}

impl FromStr for Uuid {
    type Err = Error;

    /// Accepts `180f`, `0x180F`, an 8-digit short form, 32 undashed hex digits, or the dashed form.
    fn from_str(s: &str) -> Result<Self, Error> {
        let bad = || Error::InvalidArgument(format!("not a UUID: {s:?}"));
        let t = s.trim();
        let t = t
            .strip_prefix("0x")
            .or_else(|| t.strip_prefix("0X"))
            .unwrap_or(t);
        let t = t.trim_start_matches('{').trim_end_matches('}');
        let hex: String = if t.contains('-') {
            let parts: Vec<&str> = t.split('-').collect();
            let lens: Vec<usize> = parts.iter().map(|p| p.len()).collect();
            if lens != [8, 4, 4, 4, 12] {
                return Err(bad());
            }
            parts.concat()
        } else {
            t.to_owned()
        };
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        match hex.len() {
            4 => Ok(Self::from_u16(
                u16::from_str_radix(&hex, 16).map_err(|_| bad())?,
            )),
            8 => Ok(Self::from_u32(
                u32::from_str_radix(&hex, 16).map_err(|_| bad())?,
            )),
            32 => Ok(Self(u128::from_str_radix(&hex, 16).map_err(|_| bad())?)),
            _ => Err(bad()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_expands_onto_the_base() {
        assert_eq!(
            Uuid::from_u16(0x180F).to_string(),
            "0000180f-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn parse_accepts_every_documented_form() {
        let want = Uuid::from_u16(0x180F);
        for s in [
            "180f",
            "0x180F",
            "0000180f",
            "0000180f00001000800000805f9b34fb",
            "0000180F-0000-1000-8000-00805F9B34FB",
            "{0000180f-0000-1000-8000-00805f9b34fb}",
        ] {
            assert_eq!(s.parse::<Uuid>().unwrap(), want, "{s}");
        }
    }

    #[test]
    fn parse_rejects_malformed_text() {
        for s in ["", "18", "180g", "0000180f-0000-1000-8000", "1234567"] {
            assert!(s.parse::<Uuid>().is_err(), "{s}");
        }
    }

    #[test]
    fn short_forms_round_trip_only_on_the_base() {
        assert_eq!(Uuid::from_u16(0xFFF0).as_u16(), Some(0xFFF0));
        assert_eq!(Uuid::from_u32(0x1234_5678).as_u16(), None);
        assert_eq!(Uuid::from_u32(0x1234_5678).as_u32(), Some(0x1234_5678));
        let custom = Uuid(0x6e40_0001_b5a3_f393_e0a9_e50e_24dc_ca9e);
        assert_eq!(custom.as_u32(), None);
        assert_eq!(custom.short_string(), custom.to_string());
        assert_eq!(Uuid::from_u16(0x2A00).short_string(), "2a00");
    }
}
