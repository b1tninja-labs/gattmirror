//! A controller over any byte transport: command credits, Reset-first init, advertising.

use core::fmt;
use std::collections::VecDeque;
use std::io::{self, Read, Write};

use hci_wire::{Decoder, Event, Opcode, Packet, opcodes};

use crate::advertising::{AdvParams, Advertisement, padded};

/// The event code a not-yet-reset controller answers every command with (observed live: `10 01 42`).
///
/// Divergence: Core 6.3 §7.7.16 defines `0x10` as Hardware Error, an implementation-specific
/// hardware failure report, and says nothing about commands sent before Reset. Rootcanal's use of
/// it for "not reset" is its own behaviour; a real controller sending Hardware Error is also
/// reported as [`Error::NotReset`] here.
const EVENT_NOT_READY: u8 = hci_wire::EventCode::HARDWARE_ERROR.0;
/// The no-operation opcode controllers use to hand out command credits unprompted.
const NOP: Opcode = Opcode(0x0000);
/// Set Event Mask (Core 6.3 §7.3.1): bits 0 to 44 (the specified default, `0x1FFF FFFF FFFF`) plus
/// bit 61, `HCI_LE_Meta`. Disconnection Complete is bit 4; Number Of Completed Packets has no bit
/// and cannot be masked.
const EVENT_MASK: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F, 0x00, 0x20];
/// LE Set Event Mask [v1] (Core 6.3 §7.8.1): bits 0 to 4, the specified default (Connection
/// Complete, Advertising Report, Connection Update Complete, Read Remote Features Complete, Long
/// Term Key Request). The LE Enhanced Connection Complete event (bit 9) is therefore never sent.
const LE_EVENT_MASK: [u8; 8] = [0x1F, 0, 0, 0, 0, 0, 0, 0];

/// What the controller reported during [`Controller::init`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerInfo {
    /// The controller's device address, as returned.
    pub bd_addr: [u8; 6],
    /// Largest ACL data packet the controller accepts for LE.
    pub le_acl_len: u16,
    /// How many such packets it buffers.
    pub le_acl_count: u8,
}

/// A host-side failure.
#[derive(Debug)]
pub enum Error {
    /// The transport failed.
    Io(io::Error),
    /// The transport timed out while waiting for the controller.
    Timeout,
    /// The transport reached end of stream.
    Closed,
    /// H4 framing was lost.
    Framing(hci_wire::Error),
    /// The controller answered a command with event `0x10`: it has not been reset.
    NotReset,
    /// The controller refused a command with this status.
    Status {
        /// The command.
        opcode: Opcode,
        /// The nonzero status byte.
        status: u8,
    },
    /// `LE_ACL_Data_Packet_Length` was zero: no dedicated LE buffer exists and the shared
    /// `HCI_Read_Buffer_Size` would be needed (Core 6.3 §7.8.2), which this host does not issue.
    NoLeBuffer,
    /// A command's answer was too short for what was expected of it.
    ShortReply(Opcode),
    /// Advertising data could not be built.
    Advertisement(crate::AdvertisementError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "transport: {e}"),
            Self::Timeout => f.write_str("timed out waiting for the controller"),
            Self::Closed => f.write_str("controller closed the connection"),
            Self::Framing(e) => write!(f, "framing: {e}"),
            Self::NotReset => f.write_str("controller answered event 0x10: it needs a Reset first"),
            Self::Status { opcode, status } => {
                write!(
                    f,
                    "command 0x{:04X} refused with status 0x{status:02X}",
                    opcode.0
                )
            }
            Self::NoLeBuffer => f.write_str("controller has no dedicated LE ACL buffer"),
            Self::ShortReply(op) => write!(f, "reply to command 0x{:04X} too short", op.0),
            Self::Advertisement(e) => write!(f, "advertisement: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Self::Timeout,
            _ => Self::Io(e),
        }
    }
}

/// A host's view of one controller behind a `Read + Write` transport (a TCP stream carrying H4).
///
/// One command is outstanding at a time, and a command is sent only while the controller's credit
/// count is nonzero. Packets that are not the answer awaited (ACL, other events) are kept in
/// arrival order for later slices; see [`Controller::take_backlog`].
#[derive(Debug)]
pub struct Controller<T: Read + Write> {
    io: T,
    decoder: Decoder,
    credits: u8,
    backlog: VecDeque<Packet>,
}

impl<T: Read + Write> Controller<T> {
    /// Wrap a transport. The controller is assumed to accept one command until it says otherwise.
    pub fn new(io: T) -> Self {
        Self {
            io,
            decoder: Decoder::new(),
            credits: 1,
            backlog: VecDeque::new(),
        }
    }

    /// The command credits the controller last granted, less commands sent since.
    pub fn credits(&self) -> u8 {
        self.credits
    }

    /// Packets received while waiting for a command's answer, oldest first.
    pub fn take_backlog(&mut self) -> Vec<Packet> {
        self.backlog.drain(..).collect()
    }

    /// Read until one whole packet is available.
    fn next_packet(&mut self) -> Result<Packet, Error> {
        let mut chunk = [0u8; 512];
        loop {
            if let Some(p) = self.decoder.next_packet().map_err(Error::Framing)? {
                return Ok(p);
            }
            let n = self.io.read(&mut chunk)?;
            if n == 0 {
                return Err(Error::Closed);
            }
            self.decoder.push(&chunk[..n]);
        }
    }

    /// The next packet from the controller, oldest backlog first; `None` if the transport timed
    /// out with nothing to read. Command credits are tracked from Command Complete and Status.
    pub(crate) fn recv(&mut self) -> Result<Option<Packet>, Error> {
        let packet = match self.backlog.pop_front() {
            Some(p) => p,
            None => match self.next_packet() {
                Ok(p) => p,
                Err(Error::Timeout) => return Ok(None),
                Err(e) => return Err(e),
            },
        };
        if let Packet::Event { code, params } = &packet {
            if *code == EVENT_NOT_READY {
                return Err(Error::NotReset);
            }
            if let Event::CommandComplete { num_packets, .. }
            | Event::CommandStatus { num_packets, .. } = Event::parse(*code, params)
            {
                self.credits = num_packets;
            }
        }
        Ok(Some(packet))
    }

    /// Write one packet (ACL data) without waiting for any answer.
    pub(crate) fn send(&mut self, packet: &Packet) -> Result<(), Error> {
        self.io.write_all(&packet.encode())?;
        self.io.flush()?;
        Ok(())
    }

    /// Absorb one packet: update credits from a Command Complete or Status, keep the rest.
    /// Returns the event if it answers `awaited`.
    fn absorb(&mut self, awaited: Option<Opcode>) -> Result<Option<Event>, Error> {
        let packet = self.next_packet()?;
        let Packet::Event { code, params } = &packet else {
            self.backlog.push_back(packet);
            return Ok(None);
        };
        if *code == EVENT_NOT_READY {
            return Err(Error::NotReset);
        }
        let event = Event::parse(*code, params);
        let (num, op) = match &event {
            Event::CommandComplete {
                num_packets,
                opcode,
                ..
            }
            | Event::CommandStatus {
                num_packets,
                opcode,
                ..
            } => (*num_packets, *opcode),
            _ => {
                self.backlog.push_back(packet);
                return Ok(None);
            }
        };
        self.credits = num;
        // A credit-only grant (opcode 0) answers nothing; anything else not awaited is ignored.
        Ok((Some(op) == awaited && op != NOP).then_some(event))
    }

    /// Send a command and return its return parameters (status byte first) from the Command
    /// Complete. A Command Status with status 0 returns an empty vector; a nonzero status of
    /// either kind is [`Error::Status`].
    ///
    /// # Errors
    ///
    /// Transport and framing errors, [`Error::NotReset`], and [`Error::Status`].
    pub fn command(&mut self, opcode: Opcode, params: &[u8]) -> Result<Vec<u8>, Error> {
        while self.credits == 0 {
            self.absorb(None)?;
        }
        let wire = Packet::Command {
            opcode,
            params: params.to_vec(),
        }
        .encode();
        self.io.write_all(&wire)?;
        self.io.flush()?;
        self.credits -= 1;
        loop {
            match self.absorb(Some(opcode))? {
                Some(Event::CommandComplete { return_params, .. }) => {
                    return match return_params.first() {
                        Some(0) => Ok(return_params),
                        Some(&status) => Err(Error::Status { opcode, status }),
                        None => Err(Error::ShortReply(opcode)),
                    };
                }
                Some(Event::CommandStatus { status, .. }) => {
                    return if status == 0 {
                        Ok(Vec::new())
                    } else {
                        Err(Error::Status { opcode, status })
                    };
                }
                _ => {}
            }
        }
    }

    /// Reset, then set the event masks and read the address and LE buffer size.
    ///
    /// # Errors
    ///
    /// Any [`Error`] from the commands.
    pub fn init(&mut self) -> Result<ControllerInfo, Error> {
        self.command(opcodes::RESET, &[])?;
        self.command(opcodes::SET_EVENT_MASK, &EVENT_MASK)?;
        self.command(opcodes::LE_SET_EVENT_MASK, &LE_EVENT_MASK)?;
        let addr = self.command(opcodes::READ_BD_ADDR, &[])?;
        let buf = self.command(opcodes::LE_READ_BUFFER_SIZE, &[])?;
        let bd_addr: [u8; 6] = addr
            .get(1..7)
            .and_then(|s| s.try_into().ok())
            .ok_or(Error::ShortReply(opcodes::READ_BD_ADDR))?;
        let [_, l0, l1, count, ..] = buf[..] else {
            return Err(Error::ShortReply(opcodes::LE_READ_BUFFER_SIZE));
        };
        let le_acl_len = u16::from_le_bytes([l0, l1]);
        if le_acl_len == 0 || count == 0 {
            return Err(Error::NoLeBuffer);
        }
        Ok(ControllerInfo {
            bd_addr,
            le_acl_len,
            le_acl_count: count,
        })
    }

    /// Set parameters, advertising data and scan response, and enable connectable advertising.
    /// Advertising already running is disabled first, since a controller may refuse parameter
    /// changes while it is on (Core 6.3 §7.8.5 lists Command Disallowed `0x0C` for that case).
    ///
    /// # Errors
    ///
    /// Any [`Error`] from the commands, or [`Error::Advertisement`].
    pub fn start_advertising(
        &mut self,
        ad: &Advertisement,
        params: &AdvParams,
    ) -> Result<(), Error> {
        let data = ad.advertising_data().map_err(Error::Advertisement)?;
        self.command(opcodes::LE_SET_ADVERTISING_ENABLE, &[0])?;
        self.command(opcodes::LE_SET_ADVERTISING_PARAMETERS, &params.encode())?;
        self.command(opcodes::LE_SET_ADVERTISING_DATA, &padded(&data))?;
        self.command(
            opcodes::LE_SET_SCAN_RESPONSE_DATA,
            &padded(&ad.scan_response()),
        )?;
        self.command(opcodes::LE_SET_ADVERTISING_ENABLE, &[1])?;
        Ok(())
    }

    /// Disable advertising.
    ///
    /// # Errors
    ///
    /// Any [`Error`] from the command.
    pub fn stop_advertising(&mut self) -> Result<(), Error> {
        self.command(opcodes::LE_SET_ADVERTISING_ENABLE, &[0])
            .map(drop)
    }

    /// Give back the transport.
    pub fn into_inner(self) -> T {
        self.io
    }
}

#[cfg(test)]
mod tests {
    use hci_wire::opcodes;

    use crate::advertising::{AdvParams, Advertisement, ServiceUuid};
    use crate::fake::Fake;
    use crate::{Controller, Error};

    #[test]
    fn init_sends_reset_first_and_reads_the_buffer_size() {
        let mut c = Controller::new(Fake::default());
        let info = c.init().unwrap();
        assert_eq!((info.le_acl_len, info.le_acl_count), (27, 20));
        assert_eq!(info.bd_addr, [1, 2, 3, 4, 5, 6]);
        let ops: Vec<_> = c.into_inner().commands().iter().map(|(o, _)| *o).collect();
        assert_eq!(
            ops,
            [
                opcodes::RESET,
                opcodes::SET_EVENT_MASK,
                opcodes::LE_SET_EVENT_MASK,
                opcodes::READ_BD_ADDR,
                opcodes::LE_READ_BUFFER_SIZE
            ]
        );
    }

    #[test]
    fn a_controller_with_no_le_buffer_is_refused_at_init() {
        let mut fake = Fake::default();
        fake.no_le_buffer();
        let mut c = Controller::new(fake);
        assert!(matches!(c.init(), Err(Error::NoLeBuffer)));
    }

    #[test]
    fn the_event_masks_enable_what_the_host_reads() {
        // Core 6.3 §7.3.1: bit 4 Disconnection Complete, bit 61 LE Meta (and the default bits 0..=44).
        // §7.8.1: LE bit 0 is Connection Complete.
        let mut c = Controller::new(Fake::default());
        c.init().unwrap();
        let log = c.into_inner().commands();
        let mask = |op| {
            let (_, p) = log.iter().find(|(o, _)| *o == op).unwrap();
            u64::from_le_bytes(p[..8].try_into().unwrap())
        };
        let event = mask(opcodes::SET_EVENT_MASK);
        assert_eq!(event & (1 << 4), 1 << 4);
        assert_eq!(event & (1 << 61), 1 << 61);
        assert_eq!(event & 0x1FFF_FFFF_FFFF, 0x1FFF_FFFF_FFFF);
        assert_eq!(mask(opcodes::LE_SET_EVENT_MASK) & 1, 1);
    }

    #[test]
    fn a_command_before_reset_is_reported_not_reset() {
        let mut c = Controller::new(Fake::default());
        assert!(matches!(
            c.command(opcodes::LE_READ_BUFFER_SIZE, &[]),
            Err(Error::NotReset)
        ));
    }

    #[test]
    fn advertising_runs_the_five_commands_in_order() {
        let mut c = Controller::new(Fake::default());
        c.init().unwrap();
        let ad = Advertisement::new("TEST", Some(ServiceUuid::Uuid16(0xFFF0))).unwrap();
        c.start_advertising(&ad, &AdvParams::default()).unwrap();
        let log = c.into_inner().commands();
        let tail: Vec<_> = log[5..].iter().map(|(o, _)| *o).collect();
        assert_eq!(
            tail,
            [
                opcodes::LE_SET_ADVERTISING_ENABLE,
                opcodes::LE_SET_ADVERTISING_PARAMETERS,
                opcodes::LE_SET_ADVERTISING_DATA,
                opcodes::LE_SET_SCAN_RESPONSE_DATA,
                opcodes::LE_SET_ADVERTISING_ENABLE
            ]
        );
        assert_eq!(log[5].1, [0]);
        assert_eq!(log[9].1, [1]);
        assert_eq!(log[7].1.len(), 32);
        assert_eq!(&log[7].1[..4], [13, 2, 1, 6]);
    }

    #[test]
    fn a_refused_command_carries_its_status() {
        let mut fake = Fake::default();
        fake.refuse(opcodes::LE_SET_ADVERTISING_DATA, 0x12);
        let mut c = Controller::new(fake);
        c.init().unwrap();
        let ad = Advertisement::new("T", None).unwrap();
        let e = c.start_advertising(&ad, &AdvParams::default()).unwrap_err();
        assert!(
            matches!(e, Error::Status { status: 0x12, opcode } if opcode == opcodes::LE_SET_ADVERTISING_DATA)
        );
    }

    /// The controller answers Reset granting zero credits, and grants one later with a no-op
    /// Command Complete. The next command must not be written before that grant arrived.
    #[test]
    fn no_command_is_sent_while_the_credit_count_is_zero() {
        let mut fake = Fake::default();
        fake.grant_credit_late_after_reset();
        let mut c = Controller::new(fake);
        c.command(opcodes::RESET, &[]).unwrap();
        assert_eq!(c.credits(), 0);
        c.command(opcodes::READ_BD_ADDR, &[]).unwrap();
        assert!(c.into_inner().credit_was_delivered_before_second_command());
    }

    #[test]
    fn unrelated_packets_are_kept_not_dropped() {
        let mut fake = Fake::default();
        fake.inject_before_next_answer(vec![0x02, 0x41, 0x20, 0x01, 0x00, 0xAA]);
        let mut c = Controller::new(fake);
        c.command(opcodes::RESET, &[]).unwrap();
        let b = c.take_backlog();
        assert_eq!(b.len(), 1);
        assert!(matches!(&b[0], hci_wire::Packet::Acl { handle: 0x41, .. }));
    }

    #[test]
    fn a_closed_transport_is_an_error_not_a_hang() {
        let mut fake = Fake::default();
        fake.go_silent();
        let mut c = Controller::new(fake);
        assert!(matches!(c.command(opcodes::RESET, &[]), Err(Error::Closed)));
    }
}
