//! A guest ATT client talks through `Mirror` and `SerialBackend` to a device behind a `ByteStream`,
//! for both serial profile layouts, and over a TCP-served stream.

use std::thread;
use std::time::{Duration, Instant};

use att_server::{Pdu, Uuid};
use gattmirror::Mirror;
use gattmirror::serial::{FFF0, NORDIC_UART, SerialBackend, WriteBuffering};
use serial_stream::mem;
use serial_stream::tcp::{Server, TcpClient, loopback};
use serial_stream::{ByteStream, StreamError};

/// A device: answers each `\r`-terminated line with the line, a status word and a `>` prompt,
/// in two chunks so chunk boundaries are visibly meaningless.
fn device(mut s: impl ByteStream + 'static) {
    let rx = s.receiver().unwrap();
    thread::spawn(move || {
        let mut line = Vec::new();
        while let Ok(chunk) = rx.recv() {
            for b in chunk {
                line.push(b);
                if b == b'\r' {
                    let l = std::mem::take(&mut line);
                    let half = l.len() / 2;
                    if s.write(&l[..half]).is_err() {
                        return;
                    }
                    let mut tail = l[half..].to_vec();
                    tail.extend_from_slice(b"\nOK\r\n>");
                    if s.write(&tail).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

fn guest_for(
    text: &str,
    buffering: WriteBuffering,
) -> (Mirror<SerialBackend<mem::MemStream>>, u16, u16) {
    let (ours, theirs) = mem::pair();
    device(theirs);
    let p = gatt_profile::parse(text).unwrap();
    let backend = SerialBackend::new(p, ours, buffering).unwrap();
    let (w, n) = (backend.write_char(), backend.notify_char());
    let m = Mirror::new(backend, 247).unwrap();
    let (wh, nh) = (m.value_handle(w).unwrap(), m.value_handle(n).unwrap());
    (m, wh, nh)
}

fn write(m: &mut Mirror<SerialBackend<mem::MemStream>>, handle: u16, value: &[u8]) {
    let r = m.handle_bytes(
        &Pdu::WriteReq {
            handle,
            value: value.to_vec(),
        }
        .encode(),
    );
    assert_eq!(Pdu::decode(&r.unwrap()).unwrap(), Pdu::WriteRsp);
}

fn collect<B: gattmirror::Backend>(m: &mut Mirror<B>, handle: u16, until: &[u8]) -> Vec<u8> {
    let mut got = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while !got.ends_with(until) && Instant::now() < end {
        for p in m.drain_outgoing() {
            match p {
                Pdu::Notification { handle: h, value } if h == handle => got.extend(value),
                other => panic!("unexpected {other:?}"),
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    got
}

#[test]
fn fff0_round_trip_with_a_command_split_over_writes() {
    let (mut m, wh, nh) = guest_for(FFF0, WriteBuffering::UntilByte(b'\r'));
    write(&mut m, nh + 1, &[1, 0]);
    write(&mut m, wh, b"AT");
    write(&mut m, wh, b"I\r");
    assert_eq!(collect(&mut m, nh, b">"), b"ATI\r\nOK\r\n>");
}

#[test]
fn nordic_uart_round_trip() {
    let (mut m, wh, nh) = guest_for(NORDIC_UART, WriteBuffering::Passthrough);
    write(&mut m, nh + 1, &[1, 0]);
    write(&mut m, wh, b"ATRV\r");
    assert_eq!(collect(&mut m, nh, b">"), b"ATRV\r\nOK\r\n>");
}

#[test]
fn a_reply_longer_than_the_default_mtu_arrives_in_order_in_pieces() {
    let (mut m, wh, nh) = guest_for(FFF0, WriteBuffering::Passthrough);
    write(&mut m, nh + 1, &[1, 0]);
    // 40 command bytes: the echo alone exceeds one 20-byte notification at the default MTU.
    let cmd: Vec<u8> = b"A"
        .iter()
        .copied()
        .cycle()
        .take(39)
        .chain(*b"\r")
        .collect();
    write(&mut m, wh, &cmd);
    let mut want = cmd.clone();
    want.extend_from_slice(b"\nOK\r\n>");
    let mut pieces = 0;
    let mut got = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while !got.ends_with(b">") && Instant::now() < end {
        for p in m.drain_outgoing() {
            let Pdu::Notification { value, .. } = p else {
                panic!("not a notification")
            };
            assert!(value.len() <= 20);
            pieces += 1;
            got.extend(value);
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(got, want);
    assert!(pieces >= 3, "{pieces}");
}

#[test]
fn nothing_is_sent_to_the_device_for_a_write_the_guest_never_finishes() {
    let (mut m, wh, nh) = guest_for(FFF0, WriteBuffering::UntilByte(b'\r'));
    write(&mut m, nh + 1, &[1, 0]);
    write(&mut m, wh, b"ATI");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(m.drain_outgoing(), Vec::<Pdu>::new());
    assert_eq!(m.backend().pending(), b"ATI");
}

#[test]
fn the_serial_profile_discovery_names_the_serial_service() {
    let (mut m, _, _) = guest_for(FFF0, WriteBuffering::Passthrough);
    let r = m.handle_bytes(
        &Pdu::ReadByGroupTypeReq {
            start: 1,
            end: 0xFFFF,
            group_type: Uuid::from_u16(0x2800),
        }
        .encode(),
    );
    let Pdu::ReadByGroupTypeRsp { entries } = Pdu::decode(&r.unwrap()).unwrap() else {
        panic!()
    };
    assert_eq!(entries.len(), 2);
}

#[test]
fn a_tcp_served_stream_backs_the_mirror() {
    // The device is served over TCP; the mirror's backend is a TCP client of it.
    let srv = Server::start(loopback(0), || {
        let (ours, theirs) = mem::pair();
        device(theirs);
        Some(Box::new(ours) as Box<dyn ByteStream>)
    })
    .unwrap();
    let client = TcpClient::connect(srv.local_addr()).unwrap();
    let profile = gatt_profile::parse(FFF0).unwrap();
    let backend = SerialBackend::new(profile, client, WriteBuffering::UntilByte(b'\r')).unwrap();
    let (wc, nc) = (backend.write_char(), backend.notify_char());
    let mut mirror = Mirror::new(backend, 247).unwrap();
    let (wh, nh) = (
        mirror.value_handle(wc).unwrap(),
        mirror.value_handle(nc).unwrap(),
    );
    let r = mirror.handle_bytes(
        &Pdu::WriteReq {
            handle: nh + 1,
            value: vec![1, 0],
        }
        .encode(),
    );
    assert!(r.is_some());
    let r = mirror.handle_bytes(
        &Pdu::WriteReq {
            handle: wh,
            value: b"ATZ\r".to_vec(),
        }
        .encode(),
    );
    assert!(r.is_some());
    assert_eq!(collect(&mut mirror, nh, b">"), b"ATZ\r\nOK\r\n>");
}

#[test]
fn stream_error_is_a_plain_value() {
    assert_eq!(StreamError::Closed, StreamError::Closed);
}

#[test]
fn a_new_connection_does_not_inherit_a_partial_command() {
    let (m, wh, nh) = guest_for(FFF0, WriteBuffering::UntilByte(b'\r'));
    let (mut server, mut pump) = m.into_parts();
    server.handle(&Pdu::WriteReq {
        handle: nh + 1,
        value: vec![1, 0],
    });
    server.handle(&Pdu::WriteReq {
        handle: wh,
        value: b"AT".to_vec(),
    });
    // What a host loop's link hook does after `Server::reset`.
    server.reset();
    server.backend().reset_connection();
    server.handle(&Pdu::WriteReq {
        handle: nh + 1,
        value: vec![1, 0],
    });
    server.handle(&Pdu::WriteReq {
        handle: wh,
        value: b"ATI\r".to_vec(),
    });
    let mut got = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while !got.ends_with(b">") && Instant::now() < end {
        for p in pump.drain(&mut server) {
            if let Pdu::Notification { value, .. } = p {
                got.extend(value);
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(got, b"ATI\r\nOK\r\n>");
}
