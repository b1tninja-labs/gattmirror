//! An in-memory controller for tests: a `Read + Write` that answers H4 commands.

use std::collections::VecDeque;
use std::io::{self, Read, Write};

use hci_wire::{Decoder, Opcode, Packet, opcodes};

/// A scripted controller. Answers are delivered one packet per `read`, so ordering is observable.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // independent test switches, not a state machine
pub struct Fake {
    decoder: Decoder,
    reset: bool,
    out: VecDeque<Vec<u8>>,
    log: Vec<(Opcode, Vec<u8>)>,
    refuse: Vec<(Opcode, u8)>,
    late_credit: bool,
    credit_delivered: bool,
    credit_delivered_at_second_command: Option<bool>,
    inject: Option<Vec<u8>>,
    silent: bool,
    block_when_idle: bool,
    auto_complete: bool,
    two_pair_complete: bool,
    acl_out: Vec<Packet>,
    no_le_buffer: bool,
}

fn complete(num: u8, op: Opcode, ret: &[u8]) -> Vec<u8> {
    let mut params = vec![num];
    params.extend_from_slice(&op.0.to_le_bytes());
    params.extend_from_slice(ret);
    Packet::Event { code: 0x0E, params }.encode()
}

impl Fake {
    pub fn commands(&self) -> Vec<(Opcode, Vec<u8>)> {
        self.log.clone()
    }
    /// Answer LE Read Buffer Size with a zero length and count, as a controller with no
    /// dedicated LE buffer does.
    pub fn no_le_buffer(&mut self) {
        self.no_le_buffer = true;
    }
    pub fn refuse(&mut self, op: Opcode, status: u8) {
        self.refuse.push((op, status));
    }
    pub fn grant_credit_late_after_reset(&mut self) {
        self.late_credit = true;
    }
    pub fn inject_before_next_answer(&mut self, bytes: Vec<u8>) {
        self.inject = Some(bytes);
    }
    /// Queue raw H4 bytes for the host to read.
    pub fn push_inbound(&mut self, bytes: Vec<u8>) {
        self.out.push_back(bytes);
    }
    /// Report an empty queue as a timeout rather than end of stream.
    pub fn block_when_idle(&mut self) {
        self.block_when_idle = true;
    }
    /// Answer each ACL packet the host writes with a Number Of Completed Packets event.
    pub fn complete_acl_automatically(&mut self) {
        self.auto_complete = true;
    }
    /// Answer each ACL packet with a Number Of Completed Packets event of two pairs in the
    /// specified layout (Core 6.3 §7.7.19: both handles, then both counts): another handle with
    /// nine, then the written packet's handle with one.
    pub fn complete_acl_in_two_pairs(&mut self) {
        self.two_pair_complete = true;
    }
    /// ACL packets the host has written.
    pub fn acl_out(&self) -> &[Packet] {
        &self.acl_out
    }
    pub fn go_silent(&mut self) {
        self.silent = true;
    }
    pub fn credit_was_delivered_before_second_command(&self) -> bool {
        self.credit_delivered_at_second_command == Some(true)
    }

    fn handle(&mut self, op: Opcode, params: Vec<u8>) {
        if self.log.len() == 1 {
            self.credit_delivered_at_second_command = Some(self.credit_delivered);
        }
        self.log.push((op, params));
        if self.silent {
            return;
        }
        if let Some(bytes) = self.inject.take() {
            self.out.push_back(bytes);
        }
        if op == opcodes::RESET {
            self.reset = true;
        } else if !self.reset {
            self.out.push_back(
                Packet::Event {
                    code: 0x10,
                    params: vec![0x42],
                }
                .encode(),
            );
            return;
        }
        if op == opcodes::DISCONNECT {
            let mut params = vec![0, 1];
            params.extend_from_slice(&op.0.to_le_bytes());
            self.out
                .push_back(Packet::Event { code: 0x0F, params }.encode());
            return;
        }
        let status = self
            .refuse
            .iter()
            .find(|(o, _)| *o == op)
            .map_or(0, |(_, s)| *s);
        let ret: Vec<u8> = if status != 0 {
            vec![status]
        } else if op == opcodes::READ_BD_ADDR {
            vec![0, 1, 2, 3, 4, 5, 6]
        } else if op == opcodes::LE_READ_BUFFER_SIZE {
            if self.no_le_buffer {
                vec![0, 0, 0, 0]
            } else {
                vec![0, 27, 0, 20]
            }
        } else {
            vec![0]
        };
        if op == opcodes::RESET && self.late_credit {
            self.out.push_back(complete(0, op, &ret));
            // The no-op grant, delivered on a later read.
            self.out.push_back(complete(1, Opcode(0), &[]));
        } else {
            self.out.push_back(complete(1, op, &ret));
        }
    }
}

impl Write for Fake {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.decoder.push(buf);
        while let Some(p) = self.decoder.next_packet().expect("host framing") {
            match p {
                Packet::Command { opcode, params } => self.handle(opcode, params),
                Packet::Acl { handle, .. } => {
                    if self.auto_complete {
                        let mut params = vec![1];
                        params.extend_from_slice(&handle.to_le_bytes());
                        params.extend_from_slice(&1u16.to_le_bytes());
                        self.out
                            .push_back(Packet::Event { code: 0x13, params }.encode());
                    }
                    if self.two_pair_complete {
                        let mut params = vec![2];
                        for h in [handle.wrapping_add(1), handle] {
                            params.extend_from_slice(&h.to_le_bytes());
                        }
                        for c in [9u16, 1] {
                            params.extend_from_slice(&c.to_le_bytes());
                        }
                        self.out
                            .push_back(Packet::Event { code: 0x13, params }.encode());
                    }
                    self.acl_out.push(p);
                }
                _ => panic!("host sent a packet the fake does not know"),
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(p) = self.out.pop_front() else {
            if self.block_when_idle {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            return Ok(0);
        };
        // A Command Complete for opcode 0 is the credit grant.
        if p.get(1) == Some(&0x0E) && p.get(4..6) == Some(&[0u8, 0][..]) {
            self.credit_delivered = true;
        }
        buf[..p.len()].copy_from_slice(&p);
        Ok(p.len())
    }
}
