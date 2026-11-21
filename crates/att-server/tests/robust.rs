//! Malformed and hostile input: nothing panics, every response fits the MTU and decodes, errors
//! name the request that caused them, and per-connection queues stay bounded.
//!
//! No external fuzzing crate is used (first-party code only): a fixed-seed xorshift generator
//! drives the cases, so a failure reproduces from the printed seed and iteration.

#![allow(clippy::many_single_char_names, clippy::cast_possible_truncation)]

use att_server::{
    Access, Backend, Database, ErrorCode, Level, Pdu, Props, SendError, Server, Subscription, Uuid,
    Value, opcode,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }
    fn u16(&mut self) -> u16 {
        u16::try_from(self.next() & 0xFFFF).unwrap()
    }
    fn byte(&mut self) -> u8 {
        u8::try_from(self.next() & 0xFF).unwrap()
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.byte()).collect()
    }
}

/// A backend with values of every awkward length.
struct Wild {
    calls: u64,
}

impl Backend for Wild {
    fn read(&mut self, handle: u16) -> Result<Vec<u8>, ErrorCode> {
        self.calls += 1;
        match handle % 5 {
            0 => Ok(vec![]),
            1 => Ok(vec![1; 1 + usize::from(handle % 7)]),
            2 => Ok(vec![2; 512]),
            3 => Err(ErrorCode::Unlikely),
            _ => Ok(vec![3; usize::from(handle % 200)]),
        }
    }
    fn write(&mut self, handle: u16, _v: &[u8], _r: bool) -> Result<(), ErrorCode> {
        if handle % 11 == 0 {
            Err(ErrorCode::Other(0x80))
        } else {
            Ok(())
        }
    }
    fn subscription_changed(&mut self, _h: u16, _s: Subscription) {}
}

/// Many services, 16- and 128-bit UUIDs, notify/indicate/write mixes, long static values and some
/// secured attributes.
fn big_db() -> Database {
    let mut b = Database::builder();
    for s in 0..12_u16 {
        b.service(if s % 3 == 0 {
            Uuid::from_u128(0x6E40_0000_B5A3_F393_E0A9_E50E_24DC_0000 | u128::from(s))
        } else {
            Uuid::from_u16(0xA000 + s)
        })
        .unwrap();
        for c in 0..4_u16 {
            let props = [
                Props::READ,
                Props::READ | Props::NOTIFY,
                Props::WRITE | Props::WRITE_NO_RSP | Props::INDICATE,
                Props::READ | Props::WRITE,
            ][usize::from(c)];
            let uuid = if c == 2 {
                Uuid::from_u128(0x6E40_1000_B5A3_F393_E0A9_E50E_24DC_0000 | u128::from(s * 8 + c))
            } else {
                Uuid::from_u16(0xB000 + s * 8 + c)
            };
            let value = if c % 2 == 0 {
                Value::Dynamic
            } else {
                Value::Static(vec![c as u8; 300 + usize::from(s)])
            };
            let vh = b.characteristic(uuid, props, value).unwrap();
            if c == 3 {
                b.descriptor(
                    Uuid::from_u16(0x2901),
                    Access::READ_WRITE,
                    Value::Static(b"x".to_vec()),
                )
                .unwrap();
                b.require(vh, Level::Encrypted, Level::Authenticated)
                    .unwrap();
            }
        }
    }
    b.build()
}

const REQUESTS: [u8; 11] = [
    0x02, 0x04, 0x06, 0x08, 0x0A, 0x0C, 0x0E, 0x10, 0x12, 0x16, 0x18,
];

fn handle_like(r: &mut Rng, max: u16) -> u16 {
    match r.below(6) {
        0 => 0,
        1 => 0xFFFF,
        2 => 1,
        3 => max,
        _ => u16::try_from(1 + r.below(usize::from(max) + 3)).unwrap_or(0xFFFF),
    }
}

/// A request of a known opcode with plausible but unchecked fields, so the handlers run deep.
fn structured(r: &mut Rng, op: u8, max_handle: u16, mtu: usize) -> Vec<u8> {
    let mut v = vec![op];
    let h = |r: &mut Rng| handle_like(r, max_handle);
    let uuid = |r: &mut Rng| -> Vec<u8> {
        match r.below(6) {
            0 => 0x2800_u16.to_le_bytes().to_vec(),
            1 => 0x2803_u16.to_le_bytes().to_vec(),
            2 => 0x2902_u16.to_le_bytes().to_vec(),
            3 => 0xB000_u16
                .wrapping_add(u16::try_from(r.below(96)).unwrap())
                .to_le_bytes()
                .to_vec(),
            4 => r.bytes(16),
            _ => r.bytes(2),
        }
    };
    let blob = |r: &mut Rng| {
        let n = r.below(mtu + 40);
        r.bytes(n)
    };
    match op {
        0x02 => v.extend(r.u16().to_le_bytes()),
        0x04 | 0x06 | 0x08 | 0x10 => {
            let (a, b) = (h(r), h(r));
            v.extend(a.to_le_bytes());
            v.extend(b.to_le_bytes());
            match op {
                0x06 => {
                    v.extend(uuid(r)[..2].to_vec());
                    v.extend(blob(r));
                }
                0x08 | 0x10 => v.extend(uuid(r)),
                _ => {}
            }
        }
        0x0A => v.extend(h(r).to_le_bytes()),
        0x0C => {
            v.extend(h(r).to_le_bytes());
            v.extend(r.u16().to_le_bytes());
        }
        0x0E => {
            for _ in 0..2 + r.below(12) {
                v.extend(h(r).to_le_bytes());
            }
        }
        0x12 | 0x52 => {
            v.extend(h(r).to_le_bytes());
            v.extend(blob(r));
        }
        0x16 => {
            v.extend(h(r).to_le_bytes());
            v.extend(r.u16().to_le_bytes());
            v.extend(blob(r));
        }
        0x18 => v.push(r.byte() % 3),
        _ => {}
    }
    v
}

fn check_response(server: &Server<Wild>, req: &[u8], resp: &[u8], ctx: &str) {
    assert!(
        resp.len() <= usize::from(server.mtu()),
        "{ctx}: response {} > mtu {}",
        resp.len(),
        server.mtu()
    );
    let decoded =
        Pdu::decode(resp).unwrap_or_else(|e| panic!("{ctx}: response does not decode: {e:?}"));
    match decoded {
        Pdu::ErrorRsp { request, .. } => {
            assert_eq!(request, req[0], "{ctx}: error names another request");
        }
        _ => assert_eq!(
            resp[0],
            req[0] + 1,
            "{ctx}: response opcode is not request + 1"
        ),
    }
}

#[test]
fn random_bytes_never_panic_the_decoder_and_survivors_round_trip() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let mut decoded = 0;
    for i in 0..60_000 {
        let n = r.below(40);
        let mut b = r.bytes(n);
        if !b.is_empty() && r.below(4) != 0 {
            b[0] = r.byte() % 0x20; // bias toward real opcodes
        }
        if let Ok(p) = Pdu::decode(&b) {
            decoded += 1;
            assert_eq!(Pdu::decode(&p.encode()), Ok(p), "iteration {i}: {b:02x?}");
        }
    }
    assert!(
        decoded > 1000,
        "generator never reached the decoder: {decoded}"
    );
}

#[test]
fn every_opcode_truncated_at_every_length_is_handled() {
    let mut r = Rng(7);
    for op in 0..=255_u8 {
        for len in 0..40 {
            let mut b = vec![op];
            b.extend(r.bytes(len));
            let mut s = Server::new(big_db(), Wild { calls: 0 }, 517);
            let _ = s.handle_bytes(&b);
            let _ = Pdu::decode(&b);
        }
    }
}

#[test]
fn hostile_requests_at_every_mtu_get_bounded_valid_responses() {
    let db = big_db();
    let max_handle = db.attributes().last().unwrap().handle();
    for (seed, mtu_req) in [0_u16, 1, 22, 23, 24, 64, 247, 512, 517, 65_535]
        .into_iter()
        .enumerate()
    {
        let mut r = Rng(0xC0FF_EE00 + seed as u64);
        let mut s = Server::new(db.clone(), Wild { calls: 0 }, 65_535);
        if mtu_req != 0 {
            let rsp = s.handle_bytes(&[0x02, mtu_req.to_le_bytes()[0], mtu_req.to_le_bytes()[1]]);
            assert!(rsp.is_some());
        }
        assert!(s.mtu() >= 23);
        for i in 0..4000 {
            let op = REQUESTS[r.below(REQUESTS.len())];
            if op == 0x02 {
                continue; // one Exchange MTU per connection, done above
            }
            let mut req = structured(&mut r, op, max_handle, usize::from(s.mtu()));
            // Sometimes corrupt it: cut it short or flip bytes.
            match r.below(5) {
                0 => req.truncate(r.below(req.len() + 1)),
                1 if req.len() > 1 => {
                    let at = 1 + r.below(req.len() - 1);
                    req[at] ^= r.byte();
                }
                _ => {}
            }
            if req.is_empty() {
                continue;
            }
            let ctx = format!(
                "mtu_req {mtu_req} iter {i} req {:02x?}",
                &req[..req.len().min(24)]
            );
            if let Some(resp) = s.handle_bytes(&req) {
                check_response(&s, &req, &resp, &ctx);
            }
            // Updates the transport would send must fit the MTU too.
            while let Some(p) = s.poll_outgoing() {
                assert!(
                    p.encode().len() <= usize::from(s.mtu()),
                    "{ctx}: update over mtu"
                );
            }
        }
    }
}

#[test]
fn commands_and_stray_pdus_never_answer() {
    let db = big_db();
    let mut r = Rng(99);
    let mut s = Server::new(db, Wild { calls: 0 }, 247);
    for _ in 0..2000 {
        let mut req = vec![[0x52_u8, 0xD2, 0x1B, 0x1D, 0x1E, 0x13, 0x0B, 0x19][r.below(8)]];
        let n = r.below(30);
        req.extend(r.bytes(n));
        assert_eq!(s.handle_bytes(&req), None, "{req:02x?}");
    }
}

#[test]
fn the_prepare_queue_is_bounded_in_parts_and_in_bytes() {
    let mut s = Server::new(big_db(), Wild { calls: 0 }, 517);
    s.handle_bytes(&[0x02, 0x05, 0x02]);
    // Any attribute that accepts a prepare write.
    let handle = (1..=s.database().attributes().last().unwrap().handle())
        .find(|&h| {
            let mut probe = Server::new(big_db(), Wild { calls: 0 }, 517);
            probe.handle_bytes(&[0x02, 0x05, 0x02]);
            matches!(
                probe.handle(&Pdu::PrepareWriteReq {
                    handle: h,
                    offset: 0,
                    value: vec![0]
                }),
                Some(Pdu::PrepareWriteRsp { .. })
            )
        })
        .unwrap_or(0);
    assert!(handle != 0, "the fixture has a prepare-writable attribute");
    let mut accepted_parts = 0;
    let mut accepted_bytes = 0;
    for _ in 0..10_000 {
        let part = vec![0xAB; 400];
        match s
            .handle(&Pdu::PrepareWriteReq {
                handle,
                offset: 0,
                value: part,
            })
            .unwrap()
        {
            Pdu::PrepareWriteRsp { value, .. } => {
                accepted_parts += 1;
                accepted_bytes += value.len();
            }
            Pdu::ErrorRsp { code, .. } => assert_eq!(code, ErrorCode::PrepareQueueFull),
            other => panic!("{other:?}"),
        }
    }
    assert!(accepted_parts <= 64, "parts {accepted_parts}");
    assert!(accepted_bytes <= 2048, "bytes {accepted_bytes}");
    // Executing frees the budget.
    s.handle(&Pdu::ExecuteWriteReq { flags: 0 });
    assert!(matches!(
        s.handle(&Pdu::PrepareWriteReq {
            handle,
            offset: 0,
            value: vec![1]
        }),
        Some(Pdu::PrepareWriteRsp { .. })
    ));
}

#[test]
fn prepare_offsets_and_sizes_are_checked() {
    let mut s = Server::new(big_db(), Wild { calls: 0 }, 23);
    let h = (1..=400_u16)
        .find(|&h| {
            let mut p = Server::new(big_db(), Wild { calls: 0 }, 23);
            matches!(
                p.handle(&Pdu::PrepareWriteReq {
                    handle: h,
                    offset: 0,
                    value: vec![0]
                }),
                Some(Pdu::PrepareWriteRsp { .. })
            )
        })
        .unwrap();
    let code = |s: &mut Server<Wild>, off: u16, n: usize| match s
        .handle(&Pdu::PrepareWriteReq {
            handle: h,
            offset: off,
            value: vec![0; n],
        })
        .unwrap()
    {
        Pdu::ErrorRsp { code, .. } => Some(code),
        _ => None,
    };
    // Offsets and total length are validated at Execute Write, not at Prepare Write
    // (Core 6.3 Vol 3 Part F 3.4.6.1): a wild offset is queued, then refused when executed.
    let execute = |s: &mut Server<Wild>| match s.handle(&Pdu::ExecuteWriteReq { flags: 1 }).unwrap()
    {
        Pdu::ErrorRsp { code, .. } => Some(code),
        _ => None,
    };
    assert_eq!(code(&mut s, 513, 1), None);
    assert_eq!(execute(&mut s), Some(ErrorCode::InvalidOffset));
    assert_eq!(code(&mut s, 0xFFFF, 1), None);
    assert_eq!(execute(&mut s), Some(ErrorCode::InvalidOffset));
    // Parts that add up to more than the longest attribute value are refused at Execute, and the
    // queue is cleared by the refusal.
    for _ in 0..29 {
        assert_eq!(code(&mut s, 0, 18), None);
    }
    assert_eq!(
        execute(&mut s),
        Some(ErrorCode::InvalidAttributeValueLength)
    );
    assert_eq!(execute(&mut s), None);
    // A part that would make the echo longer than the MTU is refused rather than echoed oversize.
    assert_eq!(
        code(&mut s, 0, 19),
        Some(ErrorCode::InvalidAttributeValueLength)
    );
    assert_eq!(code(&mut s, 0, 18), None);
}

#[test]
fn the_outgoing_queue_is_bounded() {
    let mut s = Server::new(big_db(), Wild { calls: 0 }, 23);
    let cccd = s
        .database()
        .attributes()
        .iter()
        .find(|a| a.uuid() == Uuid::from_u16(0x2902))
        .unwrap()
        .handle();
    s.handle(&Pdu::WriteReq {
        handle: cccd,
        value: vec![1, 0],
    })
    .unwrap();
    let mut accepted = 0;
    let mut full = false;
    for _ in 0..10_000 {
        match s.notify(cccd - 1, &[1; 20]) {
            Ok(()) => accepted += 1,
            Err(SendError::QueueFull) => {
                full = true;
                break;
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    assert!(full && accepted == 256, "accepted {accepted}");
    // Draining makes room again.
    assert!(s.poll_outgoing().is_some());
    assert_eq!(s.notify(cccd - 1, &[1]), Ok(()));
}

#[test]
fn execute_write_with_a_bad_flag_and_an_empty_queue() {
    let mut s = Server::new(big_db(), Wild { calls: 0 }, 23);
    assert_eq!(
        s.handle(&Pdu::ExecuteWriteReq { flags: 1 }),
        Some(Pdu::ExecuteWriteRsp)
    );
    assert_eq!(
        s.handle(&Pdu::ExecuteWriteReq { flags: 7 }),
        Some(Pdu::ErrorRsp {
            request: opcode::EXECUTE_WRITE_REQ,
            handle: 0,
            code: ErrorCode::InvalidPdu
        })
    );
}
