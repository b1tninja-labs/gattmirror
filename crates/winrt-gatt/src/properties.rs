//! Characteristic properties as the bit set a peripheral declares.
//!
//! Bit positions for the low byte are those of the characteristic declaration (Core Specification 7.0,
//! Vol 3 Part G, Table 3.5), which the Windows `GattCharacteristicProperties` flags mirror (observed).
//! In 7.0 bit `0x40` is "previously used".

use std::fmt;

/// A set of characteristic properties.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Properties(pub u16);

impl Properties {
    /// Broadcast.
    pub const BROADCAST: u16 = 0x01;
    /// Read.
    pub const READ: u16 = 0x02;
    /// Write without response.
    pub const WRITE_WITHOUT_RESPONSE: u16 = 0x04;
    /// Write with response.
    pub const WRITE: u16 = 0x08;
    /// Notify.
    pub const NOTIFY: u16 = 0x10;
    /// Indicate.
    pub const INDICATE: u16 = 0x20;
    /// Authenticated signed writes.
    pub const SIGNED_WRITE: u16 = 0x40;
    /// Extended properties descriptor present.
    pub const EXTENDED: u16 = 0x80;

    const NAMES: [(u16, &'static str); 8] = [
        (Self::BROADCAST, "broadcast"),
        (Self::READ, "read"),
        (Self::WRITE_WITHOUT_RESPONSE, "write-without-response"),
        (Self::WRITE, "write"),
        (Self::NOTIFY, "notify"),
        (Self::INDICATE, "indicate"),
        (Self::SIGNED_WRITE, "signed-write"),
        (Self::EXTENDED, "extended"),
    ];

    /// True if every bit in `flag` is set.
    #[must_use]
    pub const fn has(self, flag: u16) -> bool {
        self.0 & flag == flag
    }

    /// Whether a notification or indication can be subscribed to.
    #[must_use]
    pub const fn can_subscribe(self) -> bool {
        self.0 & (Self::NOTIFY | Self::INDICATE) != 0
    }

    /// Whether any write is allowed.
    #[must_use]
    pub const fn can_write(self) -> bool {
        self.0 & (Self::WRITE | Self::WRITE_WITHOUT_RESPONSE) != 0
    }

    /// Names of the set properties, in bit order.
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(bit, _)| self.has(*bit))
            .map(|(_, n)| *n)
            .collect()
    }
}

impl fmt::Debug for Properties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Properties({:#04x}: {})", self.0, self.names().join("|"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_write_predicates() {
        assert!(Properties(Properties::NOTIFY).can_subscribe());
        assert!(Properties(Properties::INDICATE).can_subscribe());
        assert!(!Properties(Properties::READ).can_subscribe());
        assert!(Properties(Properties::WRITE_WITHOUT_RESPONSE).can_write());
        assert!(!Properties(Properties::READ | Properties::NOTIFY).can_write());
    }

    #[test]
    fn names_follow_bit_order() {
        let p = Properties(Properties::NOTIFY | Properties::READ | Properties::WRITE);
        assert_eq!(p.names(), ["read", "write", "notify"]);
        assert_eq!(Properties::default().names().len(), 0);
    }

    #[test]
    fn declaration_bits_follow_core_table_3_5() {
        // Core Specification 7.0, Vol 3 Part G, Table 3.5.
        assert_eq!(Properties::BROADCAST, 0x01);
        assert_eq!(Properties::READ, 0x02);
        assert_eq!(Properties::WRITE_WITHOUT_RESPONSE, 0x04);
        assert_eq!(Properties::WRITE, 0x08);
        assert_eq!(Properties::NOTIFY, 0x10);
        assert_eq!(Properties::INDICATE, 0x20);
        assert_eq!(Properties::SIGNED_WRITE, 0x40);
        assert_eq!(Properties::EXTENDED, 0x80);
    }

    #[test]
    fn has_requires_all_bits() {
        let p = Properties(Properties::READ);
        assert!(!p.has(Properties::READ | Properties::WRITE));
    }
}
