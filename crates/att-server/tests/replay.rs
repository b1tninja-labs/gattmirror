//! Replays a transcript of a typical Android client against a fixed table. The expected server
//! bytes in `data/android_discovery.txt` were worked out by hand, independently of this crate.
//! The transcript is synthesized, not captured; see its header.

use att_server::{Backend, Database, ErrorCode, Props, Server, Uuid, Value};

struct Echo;

impl Backend for Echo {
    fn read(&mut self, _h: u16) -> Result<Vec<u8>, ErrorCode> {
        Ok(vec![])
    }
    fn write(&mut self, _h: u16, _v: &[u8], _r: bool) -> Result<(), ErrorCode> {
        Ok(())
    }
}

/// Handles: 1 GAP service; 2 and 3 device name; 4 vendor service; 5, 6, 7 notify characteristic with
/// its CCCD; 8 and 9 write characteristic.
fn table() -> Database {
    let mut b = Database::builder();
    b.service(Uuid::from_u16(0x1800)).unwrap();
    b.characteristic(
        Uuid::from_u16(0x2A00),
        Props::READ,
        Value::Static(b"Dev".to_vec()),
    )
    .unwrap();
    b.service(Uuid::from_u16(0xFFF0)).unwrap();
    b.characteristic(Uuid::from_u16(0xFFF1), Props::NOTIFY, Value::Dynamic)
        .unwrap();
    b.characteristic(
        Uuid::from_u16(0xFFF2),
        Props::WRITE | Props::WRITE_NO_RSP,
        Value::Dynamic,
    )
    .unwrap();
    b.build()
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

#[test]
fn android_discovery_transcript() {
    let mut server = Server::new(table(), Echo, 517);
    let script = include_str!("data/android_discovery.txt");
    let mut pending: Option<Vec<u8>> = None;
    let mut steps = 0;
    for (n, line) in script.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (tag, rest) = line.split_once(':').unwrap();
        match tag {
            "C" => {
                assert!(
                    pending.is_none(),
                    "line {}: client sent twice without an expectation",
                    n + 1
                );
                pending = Some(hex(rest));
            }
            "S" | "N" => {
                let req = pending.take().expect("an expectation needs a request");
                let got = server.handle_bytes(&req);
                let want = (tag == "S").then(|| hex(rest));
                assert_eq!(got, want, "line {}: request {req:02x?}", n + 1);
                steps += 1;
            }
            other => panic!("line {}: unknown tag {other}", n + 1),
        }
    }
    assert!(pending.is_none());
    assert_eq!(steps, 13);
}
