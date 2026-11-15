//! HCI command opcodes: a 6-bit opcode group field (OGF) over a 10-bit command field (OCF).

use core::fmt;

/// A 16-bit HCI opcode.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Opcode(pub u16);

impl Opcode {
    /// Build an opcode from its group and command fields. Bits above 6 (group) and 10 (command)
    /// are discarded.
    #[must_use]
    pub const fn new(ogf: u8, ocf: u16) -> Self {
        Self((((ogf & 0x3F) as u16) << 10) | (ocf & 0x03FF))
    }

    /// The opcode group field.
    #[must_use]
    pub const fn ogf(self) -> u8 {
        (self.0 >> 10) as u8
    }

    /// The opcode command field.
    #[must_use]
    pub const fn ocf(self) -> u16 {
        self.0 & 0x03FF
    }
}

impl fmt::Debug for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Opcode(0x{:04X}: ogf 0x{:02X}, ocf 0x{:03X})",
            self.0,
            self.ogf(),
            self.ocf()
        )
    }
}

/// Opcodes this project has exercised against a live controller. Not a catalogue.
pub mod opcodes {
    use super::Opcode;

    /// Reset (controller and baseband group `0x03`, command `0x003`).
    pub const RESET: Opcode = Opcode::new(0x03, 0x003);
    /// Read Local Version Information (informational group `0x04`, command `0x001`).
    pub const READ_LOCAL_VERSION: Opcode = Opcode::new(0x04, 0x001);
    /// Read `BD_ADDR` (informational group `0x04`, command `0x009`).
    pub const READ_BD_ADDR: Opcode = Opcode::new(0x04, 0x009);
    /// LE Read Buffer Size (LE group `0x08`, command `0x002`).
    pub const LE_READ_BUFFER_SIZE: Opcode = Opcode::new(0x08, 0x002);
    /// LE Set Advertising Enable (LE group `0x08`, command `0x00A`).
    pub const LE_SET_ADVERTISING_ENABLE: Opcode = Opcode::new(0x08, 0x00A);
}

#[cfg(test)]
mod tests {
    use super::{Opcode, opcodes};

    #[test]
    fn fields_round_trip() {
        let op = Opcode::new(0x08, 0x00A);
        assert_eq!((op.ogf(), op.ocf()), (0x08, 0x00A));
    }

    #[test]
    fn opcodes_have_the_values_a_controller_accepted() {
        // Sent to a live controller as little-endian bytes 03 0c, 01 10, 09 10, 02 20 and 0a 20.
        assert_eq!(opcodes::RESET.0, 0x0C03);
        assert_eq!(opcodes::READ_LOCAL_VERSION.0, 0x1001);
        assert_eq!(opcodes::READ_BD_ADDR.0, 0x1009);
        assert_eq!(opcodes::LE_READ_BUFFER_SIZE.0, 0x2002);
        assert_eq!(opcodes::LE_SET_ADVERTISING_ENABLE.0, 0x200A);
    }

    #[test]
    fn out_of_range_fields_are_masked() {
        assert_eq!(Opcode::new(0xFF, 0xFFFF).0, 0xFFFF);
        assert_eq!(Opcode::new(0x40, 0).0, 0);
    }
}
