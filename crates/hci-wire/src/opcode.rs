//! HCI command opcodes: a 6-bit opcode group field (OGF) over a 10-bit command field (OCF).
//!
//! Anchors are Bluetooth Core Specification 6.3 (final, 2026-03), Vol 4 Part E: §5.4.1 (the opcode
//! is the OGF in the upper 6 bits and the OCF in the lower 10), §7.1.6, §7.3.1, §7.3.2, §7.4 and
//! §7.8. Each constant names the section its OCF was read from.

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

    /// Reset (controller and baseband group `0x03`, command `0x003`). Core 6.3 §7.3.2; OGF `0x03` is
    /// stated at §7.3.
    pub const RESET: Opcode = Opcode::new(0x03, 0x003);
    /// Read Local Version Information (informational group `0x04`, command `0x001`). Core 6.3 §7.4.
    pub const READ_LOCAL_VERSION: Opcode = Opcode::new(0x04, 0x001);
    /// Read `BD_ADDR` (informational group `0x04`, command `0x009`). Core 6.3 §7.4.
    pub const READ_BD_ADDR: Opcode = Opcode::new(0x04, 0x009);
    /// LE Read Buffer Size, version 1 (LE group `0x08`, command `0x002`). Core 6.3 §7.8.2. Version 2 of
    /// the command is `0x0060` and adds the ISO buffer fields; this project uses v1.
    pub const LE_READ_BUFFER_SIZE: Opcode = Opcode::new(0x08, 0x002);
    /// LE Set Advertising Enable (LE group `0x08`, command `0x00A`). Core 6.3 §7.8.9.
    pub const LE_SET_ADVERTISING_ENABLE: Opcode = Opcode::new(0x08, 0x00A);
    /// Set Event Mask (controller and baseband group `0x03`, command `0x001`). Core 6.3 §7.3.1.
    pub const SET_EVENT_MASK: Opcode = Opcode::new(0x03, 0x001);
    /// LE Set Event Mask, version 1 (LE group `0x08`, command `0x001`). Core 6.3 §7.8.1 (v2 is `0x00A4`).
    pub const LE_SET_EVENT_MASK: Opcode = Opcode::new(0x08, 0x001);
    /// LE Set Advertising Parameters (LE group `0x08`, command `0x006`). Core 6.3 §7.8.5.
    pub const LE_SET_ADVERTISING_PARAMETERS: Opcode = Opcode::new(0x08, 0x006);
    /// LE Set Advertising Data (LE group `0x08`, command `0x008`). Core 6.3 §7.8.7.
    pub const LE_SET_ADVERTISING_DATA: Opcode = Opcode::new(0x08, 0x008);
    /// LE Set Scan Response Data (LE group `0x08`, command `0x009`). Core 6.3 §7.8.8.
    pub const LE_SET_SCAN_RESPONSE_DATA: Opcode = Opcode::new(0x08, 0x009);
    /// Disconnect (link control group `0x01`, command `0x006`). Core 6.3 §7.1.6.
    pub const DISCONNECT: Opcode = Opcode::new(0x01, 0x006);
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
    fn opcodes_have_the_values_in_the_specification() {
        // OCFs from Core 6.3 §7.3.2, §7.4, §7.4, §7.8.2 and §7.8.9; also sent live as 03 0c, 01 10,
        // 09 10, 02 20 and 0a 20.
        assert_eq!(opcodes::RESET.0, 0x0C03);
        assert_eq!(opcodes::READ_LOCAL_VERSION.0, 0x1001);
        assert_eq!(opcodes::READ_BD_ADDR.0, 0x1009);
        assert_eq!(opcodes::LE_READ_BUFFER_SIZE.0, 0x2002);
        assert_eq!(opcodes::LE_SET_ADVERTISING_ENABLE.0, 0x200A);
    }

    /// OCFs from Core 6.3 §7.3.1 (`0x0001`), §7.8.1 (`0x0001`), §7.8.5 (`0x0006`), §7.8.7
    /// (`0x0008`) and §7.8.8 (`0x0009`); OGFs `0x03` (§7.3) and `0x08` (§7.8).
    #[test]
    fn advertising_opcodes_have_the_values_in_the_specification() {
        assert_eq!(opcodes::SET_EVENT_MASK.0, 0x0C01);
        assert_eq!(opcodes::LE_SET_EVENT_MASK.0, 0x2001);
        assert_eq!(opcodes::LE_SET_ADVERTISING_PARAMETERS.0, 0x2006);
        assert_eq!(opcodes::LE_SET_ADVERTISING_DATA.0, 0x2008);
        assert_eq!(opcodes::LE_SET_SCAN_RESPONSE_DATA.0, 0x2009);
    }

    /// OCF `0x0006` in §7.1.6; OGF `0x01` is the Link Control group (§7.1).
    #[test]
    fn disconnect_has_the_value_in_the_specification() {
        assert_eq!(opcodes::DISCONNECT.0, 0x0406);
    }

    #[test]
    fn out_of_range_fields_are_masked() {
        assert_eq!(Opcode::new(0xFF, 0xFFFF).0, 0xFFFF);
        assert_eq!(Opcode::new(0x40, 0).0, 0);
    }
}
