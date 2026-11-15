//! Talk to a live Rootcanal controller, if one is named.
//!
//! Set `GATTMIRROR_HCI` to the `host:port` of the controller's H4-over-TCP socket (the emulator's
//! `netsimd` serves it; its port is the `--hci-port` it was started with). Without the variable
//! the tests pass vacuously and say so, because CI has no emulator. They send only read-only
//! informational commands and a Reset of the connection's own virtual controller.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use hci_wire::{Decoder, Event, Packet, opcodes};

fn connect() -> Option<TcpStream> {
    let Ok(addr) = std::env::var("GATTMIRROR_HCI") else {
        eprintln!("GATTMIRROR_HCI not set: live controller test skipped");
        return None;
    };
    let s = TcpStream::connect(addr).expect("connect to the controller");
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    Some(s)
}

fn command_complete(
    s: &mut TcpStream,
    d: &mut Decoder,
    opcode: hci_wire::Opcode,
    params: Vec<u8>,
) -> Vec<u8> {
    s.write_all(&Packet::Command { opcode, params }.encode())
        .unwrap();
    let mut chunk = [0u8; 512];
    loop {
        if let Some(Packet::Event { code, params }) = d.next_packet().expect("framing") {
            if let Event::CommandComplete {
                opcode: got,
                return_params,
                ..
            } = Event::parse(code, &params)
            {
                assert_eq!(got, opcode, "answer is for the command sent");
                return return_params;
            }
            continue;
        }
        let n = s.read(&mut chunk).expect("controller answered");
        assert!(n > 0, "controller closed the connection");
        d.push(&chunk[..n]);
    }
}

#[test]
fn reset_and_informational_commands_succeed() {
    let Some(mut s) = connect() else { return };
    let mut d = Decoder::new();
    for op in [
        opcodes::RESET,
        opcodes::READ_LOCAL_VERSION,
        opcodes::READ_BD_ADDR,
        opcodes::LE_READ_BUFFER_SIZE,
    ] {
        let ret = command_complete(&mut s, &mut d, op, vec![]);
        assert_eq!(ret[0], 0, "status of {op:?}");
    }
}

#[test]
fn le_read_buffer_size_reports_an_acl_length_and_count() {
    let Some(mut s) = connect() else { return };
    let mut d = Decoder::new();
    command_complete(&mut s, &mut d, opcodes::RESET, vec![]);
    let ret = command_complete(&mut s, &mut d, opcodes::LE_READ_BUFFER_SIZE, vec![]);
    assert_eq!(ret.len(), 4, "status, 16-bit length, 8-bit count");
    assert_eq!(ret[0], 0);
}

/// Observed, not specified: a controller that has not been reset answers any command with an
/// event `0x10` (one parameter, `0x42`) instead of a Command Complete. A host must Reset first.
#[test]
fn a_command_before_reset_is_answered_with_event_0x10() {
    let Some(mut s) = connect() else { return };
    s.write_all(
        &Packet::Command {
            opcode: opcodes::LE_READ_BUFFER_SIZE,
            params: vec![],
        }
        .encode(),
    )
    .unwrap();
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).expect("an answer");
    assert_eq!(&buf[..n], [0x04, 0x10, 0x01, 0x42]);
}
