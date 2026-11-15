//! An in-memory ATT client talks to the mirror over its public API, with the simulated adapter
//! behind it.

use att_server::{ErrorCode, Pdu, Uuid};
use gattmirror::sim::SimAdapter;
use gattmirror::{Backend, CharId, Mirror};

struct Guest {
    mirror: Mirror<SimAdapter>,
}

impl Guest {
    fn new() -> Self {
        Guest {
            mirror: Mirror::new(SimAdapter::new(), 247).unwrap(),
        }
    }

    /// Send a request as wire bytes and decode the answer.
    fn req(&mut self, p: &Pdu) -> Pdu {
        let out = self
            .mirror
            .handle_bytes(&p.encode())
            .expect("a response to a request");
        Pdu::decode(&out).unwrap()
    }

    fn handle_of(&self, ch: usize) -> u16 {
        self.mirror.value_handle(CharId(ch)).unwrap()
    }

    /// Everything queued for the guest, concatenated.
    fn notified(&mut self, handle: u16) -> Vec<u8> {
        let mut out = Vec::new();
        for p in self.mirror.drain_outgoing() {
            match p {
                Pdu::Notification { handle: h, value } if h == handle => out.extend(value),
                other => panic!("unexpected {other:?}"),
            }
        }
        out
    }

    fn subscribe(&mut self, value_handle: u16) {
        let cccd = value_handle + 1;
        assert_eq!(
            self.req(&Pdu::WriteReq {
                handle: cccd,
                value: vec![1, 0]
            }),
            Pdu::WriteRsp
        );
    }
}

#[test]
fn discovery_shows_the_mirrored_table() {
    let mut g = Guest::new();
    let Pdu::ReadByGroupTypeRsp { entries } = g.req(&Pdu::ReadByGroupTypeReq {
        start: 1,
        end: 0xFFFF,
        group_type: Uuid::from_u16(0x2800),
    }) else {
        panic!("not a group response");
    };
    let services: Vec<Uuid> = entries
        .iter()
        .map(|e| Uuid::from_le_bytes(&e.value).unwrap())
        .collect();
    assert_eq!(services, [Uuid::from_u16(0x1800), Uuid::from_u16(0xFFF0)]);

    let Pdu::ReadByTypeRsp { entries } = g.req(&Pdu::ReadByTypeReq {
        start: entries[1].start,
        end: entries[1].end,
        attr_type: Uuid::from_u16(0x2803),
    }) else {
        panic!("not a type response");
    };
    // Each declaration value: properties, value handle, 16-bit UUID.
    let chars: Vec<(u8, u16, u16)> = entries
        .iter()
        .map(|(_, v)| {
            (
                v[0],
                u16::from_le_bytes([v[1], v[2]]),
                u16::from_le_bytes([v[3], v[4]]),
            )
        })
        .collect();
    assert_eq!(
        chars,
        [
            (0x10, g.handle_of(1), 0xFFF1),
            (0x0C, g.handle_of(2), 0xFFF2)
        ]
    );

    let Pdu::FindInfoRsp { entries } = g.req(&Pdu::FindInfoReq {
        start: g.handle_of(1) + 1,
        end: g.handle_of(1) + 1,
    }) else {
        panic!("not a find-info response");
    };
    assert_eq!(entries, [(g.handle_of(1) + 1, Uuid::from_u16(0x2902))]);
}

#[test]
fn the_captured_device_name_is_served_without_touching_the_adapter() {
    let mut g = Guest::new();
    let h = g.handle_of(0);
    let Pdu::ReadRsp { value } = g.req(&Pdu::ReadReq { handle: h }) else {
        panic!("not a read response");
    };
    assert_eq!(value, b"Example Serial Adapter");
    assert_eq!(g.mirror.backend().reads, 0);
}

#[test]
fn a_write_reaches_the_adapter_and_its_reply_comes_back_as_notifications() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    g.subscribe(tx);
    // Enabling notifications connects the session: the adapter prints its banner.
    assert_eq!(g.notified(tx), b"\r\nELM327 v1.5\r\n\r\n>");

    assert_eq!(
        g.req(&Pdu::WriteReq {
            handle: rx,
            value: b"ATI\r".to_vec()
        }),
        Pdu::WriteRsp
    );
    assert_eq!(g.mirror.backend().writes.len(), 1);
    assert_eq!(g.notified(tx), b"ATI\rELM327 v1.5\r\n\r\n>");
}

#[test]
fn replies_longer_than_the_mtu_are_split_in_order() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    g.subscribe(tx);
    g.notified(tx);
    g.req(&Pdu::WriteReq {
        handle: rx,
        value: b"ATZ\r".to_vec(),
    });
    let pieces = g.mirror.drain_outgoing();
    let lens: Vec<usize> = pieces
        .iter()
        .map(|p| match p {
            Pdu::Notification { value, .. } => value.len(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    // 22 bytes at the default MTU of 23 (20-byte payloads).
    assert_eq!(lens, [20, 2]);
}

#[test]
fn a_write_command_is_relayed_without_a_response() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    g.subscribe(tx);
    g.notified(tx);
    let r = g.mirror.handle_bytes(
        &Pdu::WriteCmd {
            handle: rx,
            value: b"ATRV\r".to_vec(),
        }
        .encode(),
    );
    assert_eq!(r, None);
    assert!(!g.mirror.backend().writes[0].2);
    assert_eq!(g.notified(tx), b"ATRV\r12.4V\r\n\r\n>");
}

#[test]
fn a_command_split_across_writes_is_one_command() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    g.subscribe(tx);
    g.notified(tx);
    for part in [&b"AT"[..], b"I", b"\r"] {
        g.req(&Pdu::WriteReq {
            handle: rx,
            value: part.to_vec(),
        });
    }
    assert_eq!(g.notified(tx), b"ATI\rELM327 v1.5\r\n\r\n>");
}

#[test]
fn nothing_is_delivered_to_a_guest_that_has_not_subscribed() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    g.req(&Pdu::WriteReq {
        handle: rx,
        value: b"ATI\r".to_vec(),
    });
    assert_eq!(g.mirror.drain_outgoing(), Vec::<Pdu>::new());
    assert_eq!(g.mirror.dropped(), 1);
    // Subscribing later does not replay the reply.
    g.subscribe(tx);
    assert_eq!(g.notified(tx), b"\r\nELM327 v1.5\r\n\r\n>");
}

#[test]
fn unsubscribing_is_passed_to_the_adapter_and_stops_delivery() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    g.subscribe(tx);
    g.notified(tx);
    g.req(&Pdu::WriteReq {
        handle: tx + 1,
        value: vec![0, 0],
    });
    g.req(&Pdu::WriteReq {
        handle: rx,
        value: b"ATI\r".to_vec(),
    });
    assert_eq!(g.mirror.drain_outgoing(), Vec::<Pdu>::new());
}

#[test]
fn guest_access_the_properties_forbid_is_refused() {
    let mut g = Guest::new();
    let tx = g.handle_of(1);
    let rx = g.handle_of(2);
    assert_eq!(
        g.req(&Pdu::WriteReq {
            handle: tx,
            value: vec![1]
        }),
        Pdu::ErrorRsp {
            request: 0x12,
            handle: tx,
            code: ErrorCode::WriteNotPermitted
        }
    );
    assert_eq!(
        g.req(&Pdu::ReadReq { handle: rx }),
        Pdu::ErrorRsp {
            request: 0x0A,
            handle: rx,
            code: ErrorCode::ReadNotPermitted
        }
    );
}

#[test]
fn the_notification_channel_can_be_taken_once() {
    let mut a = SimAdapter::new();
    assert!(a.notifications().is_some());
    assert!(Mirror::new(a, 23).is_err());
}
