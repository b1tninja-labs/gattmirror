//! A typed view of the events this project reads.

use crate::opcode::Opcode;

/// Event codes seen from a live controller. Not a catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventCode(pub u8);

impl EventCode {
    /// Disconnection Complete. `[unread]`
    pub const DISCONNECTION_COMPLETE: Self = Self(0x05);
    /// Number Of Completed Packets. `[unread]`
    pub const NUMBER_OF_COMPLETED_PACKETS: Self = Self(0x13);
    /// Command Complete.
    pub const COMMAND_COMPLETE: Self = Self(0x0E);
    /// Command Status.
    pub const COMMAND_STATUS: Self = Self(0x0F);
    /// LE Meta: the first parameter byte is a subevent code.
    pub const LE_META: Self = Self(0x3E);
}

/// A decoded event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Command Complete: the controller's credit count, the opcode answered, and the return
    /// parameters (whose first byte is the status for the commands used here).
    CommandComplete {
        /// How many command packets the controller will now accept.
        num_packets: u8,
        /// The opcode this answers.
        opcode: Opcode,
        /// The return parameters.
        return_params: Vec<u8>,
    },
    /// Command Status: the command was accepted (`status` 0) or refused, and a later event will
    /// report completion.
    CommandStatus {
        /// The status byte.
        status: u8,
        /// The controller's credit count.
        num_packets: u8,
        /// The opcode this answers.
        opcode: Opcode,
    },
    /// An LE Meta event: subevent code and its parameters.
    LeMeta {
        /// The subevent code.
        subevent: u8,
        /// The parameters after the subevent byte.
        params: Vec<u8>,
    },
    /// Any other event, undecoded.
    Other {
        /// The event code.
        code: u8,
        /// The event parameters.
        params: Vec<u8>,
    },
}

impl Event {
    /// Interpret an event's code and parameters. A parameter block too short for its event type
    /// is returned as [`Event::Other`] rather than guessed at.
    #[must_use]
    pub fn parse(code: u8, params: &[u8]) -> Self {
        let other = || Self::Other {
            code,
            params: params.to_vec(),
        };
        match EventCode(code) {
            EventCode::COMMAND_COMPLETE if params.len() >= 3 => Self::CommandComplete {
                num_packets: params[0],
                opcode: Opcode(u16::from_le_bytes([params[1], params[2]])),
                return_params: params[3..].to_vec(),
            },
            EventCode::COMMAND_STATUS if params.len() >= 4 => Self::CommandStatus {
                status: params[0],
                num_packets: params[1],
                opcode: Opcode(u16::from_le_bytes([params[2], params[3]])),
            },
            EventCode::LE_META if !params.is_empty() => Self::LeMeta {
                subevent: params[0],
                params: params[1..].to_vec(),
            },
            _ => other(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Event;
    use crate::opcode::opcodes;

    /// Captured from a live controller in answer to Reset.
    #[test]
    fn command_complete_for_reset() {
        let e = Event::parse(0x0E, &[0x01, 0x03, 0x0C, 0x00]);
        assert_eq!(
            e,
            Event::CommandComplete {
                num_packets: 1,
                opcode: opcodes::RESET,
                return_params: vec![0]
            }
        );
    }

    /// Captured from a live controller in answer to LE Read Buffer Size: status, then a 16-bit
    /// ACL length (27) and a count (20) in the layout the live test checks.
    #[test]
    fn command_complete_carries_return_parameters() {
        let e = Event::parse(0x0E, &[0x01, 0x02, 0x20, 0x00, 0x1B, 0x00, 0x14]);
        let Event::CommandComplete { return_params, .. } = e else {
            panic!("not a command complete")
        };
        assert_eq!(return_params, [0x00, 0x1B, 0x00, 0x14]);
    }

    #[test]
    fn command_status_layout() {
        let e = Event::parse(0x0F, &[0x00, 0x01, 0x0D, 0x20]);
        assert!(
            matches!(e, Event::CommandStatus { status: 0, num_packets: 1, opcode } if opcode.0 == 0x200D)
        );
    }

    #[test]
    fn le_meta_splits_the_subevent() {
        assert_eq!(
            Event::parse(0x3E, &[0x01, 9, 9]),
            Event::LeMeta {
                subevent: 1,
                params: vec![9, 9]
            }
        );
    }

    #[test]
    fn short_blocks_are_not_guessed_at() {
        assert!(matches!(
            Event::parse(0x0E, &[0x01, 0x03]),
            Event::Other { code: 0x0E, .. }
        ));
        assert!(matches!(
            Event::parse(0x0F, &[0x00]),
            Event::Other { code: 0x0F, .. }
        ));
        assert!(matches!(
            Event::parse(0x3E, &[]),
            Event::Other { code: 0x3E, .. }
        ));
    }
}
