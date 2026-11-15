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
    pub fn refuse(&mut self, op: Opcode, status: u8) {
        self.refuse.push((op, status));
    }
    pub fn grant_credit_late_after_reset(&mut self) {
        self.late_credit = true;
    }
    pub fn inject_before_next_answer(&mut self, bytes: Vec<u8>) {
        self.inject = Some(bytes);
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
            vec![0, 27, 0, 20]
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
            let Packet::Command { opcode, params } = p else {
                panic!("host sent a non-command")
            };
            self.handle(opcode, params);
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
