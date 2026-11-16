//! Read Multiple, security levels, indication tracking and the Service Changed decision.

#![allow(clippy::unnecessary_wraps)]

use att_server::{
    Access, Backend, Database, ErrorCode, Level, Pdu, Props, Server, Subscription, Uuid, Value,
};

#[derive(Default)]
struct Rec {
    confirmed: Vec<u16>,
    wrote: Vec<u16>,
}

impl Backend for Rec {
    fn read(&mut self, _h: u16) -> Result<Vec<u8>, ErrorCode> {
        Ok(vec![0xEE; 3])
    }
    fn write(&mut self, h: u16, _v: &[u8], _r: bool) -> Result<(), ErrorCode> {
        self.wrote.push(h);
        Ok(())
    }
    fn indication_confirmed(&mut self, h: u16) {
        self.confirmed.push(h);
    }
    fn subscription_changed(&mut self, _h: u16, _s: Subscription) {}
}

struct H {
    open: u16,
    secret: u16,
    signed: u16,
    ind: u16,
    ind_cccd: u16,
}

fn table() -> (Database, H) {
    let mut b = Database::builder();
    b.service(Uuid::from_u16(0xFFF0)).unwrap();
    let open = b
        .characteristic(
            Uuid::from_u16(0xFFF1),
            Props::READ | Props::WRITE,
            Value::Static(vec![1, 2]),
        )
        .unwrap();
    let secret = b
        .characteristic(
            Uuid::from_u16(0xFFF2),
            Props::READ | Props::WRITE,
            Value::Dynamic,
        )
        .unwrap();
    b.require(secret, Level::Encrypted, Level::Encrypted)
        .unwrap();
    let signed = b
        .characteristic(
            Uuid::from_u16(0xFFF3),
            Props::READ | Props::WRITE,
            Value::Dynamic,
        )
        .unwrap();
    b.require(signed, Level::Authenticated, Level::Authenticated)
        .unwrap();
    let ind = b
        .characteristic(Uuid::from_u16(0xFFF4), Props::INDICATE, Value::Dynamic)
        .unwrap();
    b.descriptor(Uuid::from_u16(0x2901), Access::READ, Value::Static(vec![7]))
        .unwrap();
    (
        b.build(),
        H {
            open,
            secret,
            signed,
            ind,
            ind_cccd: ind + 1,
        },
    )
}

fn server() -> (Server<Rec>, H) {
    let (db, h) = table();
    (Server::new(db, Rec::default(), 247), h)
}

fn err(req: u8, handle: u16, code: ErrorCode) -> Option<Pdu> {
    Some(Pdu::ErrorRsp {
        request: req,
        handle,
        code,
    })
}

#[test]
fn read_multiple_concatenates_and_names_the_failing_handle() {
    let (mut s, h) = server();
    assert_eq!(
        s.handle(&Pdu::ReadMultipleReq {
            handles: vec![h.open, h.secret, h.open]
        }),
        err(0x0E, h.secret, ErrorCode::InsufficientEncryption)
    );
    s.set_link_level(Level::Encrypted);
    assert_eq!(
        s.handle(&Pdu::ReadMultipleReq {
            handles: vec![h.open, h.secret, h.open]
        }),
        Some(Pdu::ReadMultipleRsp {
            values: vec![1, 2, 0xEE, 0xEE, 0xEE, 1, 2]
        })
    );
    assert_eq!(
        s.handle(&Pdu::ReadMultipleReq {
            handles: vec![h.open, 999]
        }),
        err(0x0E, 999, ErrorCode::InvalidHandle)
    );
    // The response is cut to MTU - 1 (22 bytes at the default MTU).
    let (db, h2) = table();
    let mut s = Server::new(db, Rec::default(), 23);
    s.set_link_level(Level::Encrypted);
    let many = vec![h2.secret; 20];
    let Some(Pdu::ReadMultipleRsp { values }) = s.handle(&Pdu::ReadMultipleReq { handles: many })
    else {
        panic!()
    };
    assert_eq!(values.len(), 22);
}

#[test]
fn encryption_and_authentication_levels_gate_reads_and_writes() {
    let (mut s, h) = server();
    let read = |s: &mut Server<Rec>, h: u16| s.handle(&Pdu::ReadReq { handle: h });
    let write = |s: &mut Server<Rec>, h: u16| {
        s.handle(&Pdu::WriteReq {
            handle: h,
            value: vec![1],
        })
    };

    assert!(matches!(read(&mut s, h.open), Some(Pdu::ReadRsp { .. })));
    assert_eq!(
        read(&mut s, h.secret),
        err(0x0A, h.secret, ErrorCode::InsufficientEncryption)
    );
    assert_eq!(
        write(&mut s, h.secret),
        err(0x12, h.secret, ErrorCode::InsufficientEncryption)
    );
    assert_eq!(
        read(&mut s, h.signed),
        err(0x0A, h.signed, ErrorCode::InsufficientAuthentication)
    );
    assert_eq!(
        write(&mut s, h.signed),
        err(0x12, h.signed, ErrorCode::InsufficientAuthentication)
    );
    assert_eq!(
        s.backend().wrote,
        Vec::<u16>::new(),
        "a refused write never reaches the backend"
    );

    s.set_link_level(Level::Encrypted);
    assert!(matches!(read(&mut s, h.secret), Some(Pdu::ReadRsp { .. })));
    assert_eq!(write(&mut s, h.secret), Some(Pdu::WriteRsp));
    assert_eq!(
        read(&mut s, h.signed),
        err(0x0A, h.signed, ErrorCode::InsufficientAuthentication)
    );

    s.set_link_level(Level::Authenticated);
    assert_eq!(write(&mut s, h.signed), Some(Pdu::WriteRsp));

    // Disconnect drops the level.
    s.reset();
    assert_eq!(
        read(&mut s, h.secret),
        err(0x0A, h.secret, ErrorCode::InsufficientEncryption)
    );
}

#[test]
fn security_also_gates_prepare_write_and_read_by_type() {
    let (mut s, h) = server();
    assert_eq!(
        s.handle(&Pdu::PrepareWriteReq {
            handle: h.secret,
            offset: 0,
            value: vec![1]
        }),
        err(0x16, h.secret, ErrorCode::InsufficientEncryption)
    );
    assert_eq!(
        s.handle(&Pdu::ReadByTypeReq {
            start: 1,
            end: 0xFFFF,
            attr_type: Uuid::from_u16(0xFFF2)
        }),
        err(0x08, h.secret, ErrorCode::InsufficientEncryption)
    );
}

#[test]
fn require_names_an_existing_handle() {
    let mut b = Database::builder();
    assert!(b.require(5, Level::Open, Level::Open).is_err());
}

#[test]
fn indication_confirmation_is_tracked_and_reported() {
    let (mut s, h) = server();
    s.handle(&Pdu::WriteReq {
        handle: h.ind_cccd,
        value: vec![2, 0],
    })
    .unwrap();
    // A confirmation with nothing outstanding changes nothing and tells nobody.
    assert_eq!(s.handle(&Pdu::Confirmation), None);
    assert_eq!(s.backend().confirmed, Vec::<u16>::new());

    s.indicate(h.ind, &[1]).unwrap();
    s.indicate(h.ind, &[2]).unwrap();
    assert_eq!(s.pending_indication(), None);
    assert!(matches!(s.poll_outgoing(), Some(Pdu::Indication { .. })));
    assert_eq!(s.pending_indication(), Some(h.ind));
    assert_eq!(s.poll_outgoing(), None);
    s.handle(&Pdu::Confirmation);
    assert_eq!(s.backend().confirmed, vec![h.ind]);
    assert_eq!(s.pending_indication(), None);
    // The second goes out only now, and a duplicate confirmation does not release a third.
    assert!(matches!(s.poll_outgoing(), Some(Pdu::Indication { value, .. }) if value == [2]));
    s.handle(&Pdu::Confirmation);
    s.handle(&Pdu::Confirmation);
    assert_eq!(s.backend().confirmed, vec![h.ind, h.ind]);
}

#[test]
fn reset_forgets_an_outstanding_indication() {
    let (mut s, h) = server();
    s.handle(&Pdu::WriteReq {
        handle: h.ind_cccd,
        value: vec![2, 0],
    })
    .unwrap();
    s.indicate(h.ind, &[1]).unwrap();
    s.poll_outgoing();
    s.reset();
    assert_eq!(s.pending_indication(), None);
    assert!(!s.indication_pending());
}

#[test]
fn the_table_never_contains_service_changed() {
    // Decision: the table is fixed per connection, so 0x2A05 is never added and never indicated.
    let (db, _) = table();
    assert!(
        db.attributes()
            .iter()
            .all(|a| a.uuid() != Uuid::from_u16(0x2A05))
    );
    assert!(
        db.attributes()
            .iter()
            .all(|a| a.uuid() != Uuid::from_u16(0x1801))
    );
}

#[test]
fn per_connection_state_is_not_shared_between_servers() {
    let (db, h) = table();
    let mut a = Server::new(db.clone(), Rec::default(), 247);
    let mut b = Server::new(db, Rec::default(), 247);
    a.handle(&Pdu::WriteReq {
        handle: h.ind_cccd,
        value: vec![2, 0],
    })
    .unwrap();
    a.handle(&Pdu::MtuReq { mtu: 200 });
    a.set_link_level(Level::Authenticated);
    assert_eq!(
        a.subscription(h.ind),
        Subscription {
            notify: false,
            indicate: true
        }
    );
    assert_eq!(b.subscription(h.ind), Subscription::default());
    assert_eq!(b.mtu(), 23);
    assert_eq!(
        b.handle(&Pdu::ReadReq { handle: h.signed }),
        err(0x0A, h.signed, ErrorCode::InsufficientAuthentication)
    );
}

fn one_char_server(props: Props, value: Value, level: Level) -> (Server<Rec>, u16) {
    let mut b = Database::builder();
    b.service(Uuid::from_u16(0xFFE0)).unwrap();
    let h = b
        .characteristic(Uuid::from_u16(0xFFE1), props, value)
        .unwrap();
    b.require(h, level, level).unwrap();
    (Server::new(b.build(), Rec::default(), 247), h)
}

/// Core 6.3 Vol 3 Part F 4: authentication and authorization are checked before any other check,
/// so a protected attribute that is also not readable or not writable reports the security error.
#[test]
fn security_is_checked_before_access_permission() {
    let (mut s, h) = one_char_server(Props::WRITE, Value::Dynamic, Level::Authenticated);
    assert_eq!(
        s.handle(&Pdu::ReadReq { handle: h }),
        err(0x0A, h, ErrorCode::InsufficientAuthentication)
    );
    let (mut s, h) = one_char_server(Props::READ, Value::Dynamic, Level::Encrypted);
    assert_eq!(
        s.handle(&Pdu::WriteReq {
            handle: h,
            value: vec![1]
        }),
        err(0x12, h, ErrorCode::InsufficientEncryption)
    );
    // With the link raised the plain permission error is reported.
    s.set_link_level(Level::Encrypted);
    assert_eq!(
        s.handle(&Pdu::WriteReq {
            handle: h,
            value: vec![1]
        }),
        err(0x12, h, ErrorCode::WriteNotPermitted)
    );
}

/// Core 6.3 Vol 3 Part F 3.4.3.3: only attributes the client may read are matched, and the request
/// never answers with a security error.
#[test]
fn find_by_type_value_matches_only_readable_attributes() {
    let (mut s, _h) = one_char_server(Props::READ, Value::Static(vec![9]), Level::Encrypted);
    let find = |s: &mut Server<Rec>| {
        s.handle(&Pdu::FindByTypeValueReq {
            start: 1,
            end: 0xFFFF,
            attr_type: 0xFFE1,
            value: vec![9],
        })
    };
    assert_eq!(find(&mut s), err(0x06, 1, ErrorCode::AttributeNotFound));
    s.set_link_level(Level::Encrypted);
    assert!(matches!(
        find(&mut s),
        Some(Pdu::FindByTypeValueRsp { ranges }) if ranges.len() == 1
    ));
    // A dynamic value is compared too.
    let (mut s, _h) = one_char_server(Props::READ, Value::Dynamic, Level::Open);
    assert!(matches!(
        s.handle(&Pdu::FindByTypeValueReq {
            start: 1,
            end: 0xFFFF,
            attr_type: 0xFFE1,
            value: vec![0xEE; 3],
        }),
        Some(Pdu::FindByTypeValueRsp { .. })
    ));
}

/// Core 6.3 Vol 3 Part F 3.4.4.7: the response needs every handle valid and readable, even those
/// whose values would be cut off by the MTU.
#[test]
fn read_multiple_checks_handles_past_the_mtu_cut() {
    let (mut s, h) = one_char_server(Props::READ, Value::Static(vec![5; 40]), Level::Open);
    assert_eq!(
        s.handle(&Pdu::ReadMultipleReq {
            handles: vec![h, 0x0F00]
        }),
        err(0x0E, 0x0F00, ErrorCode::InvalidHandle)
    );
    // Both valid: the concatenation is cut to MTU - 1 (22 at the default MTU).
    let Some(Pdu::ReadMultipleRsp { values }) = s.handle(&Pdu::ReadMultipleReq {
        handles: vec![h, h],
    }) else {
        panic!("expected a Read Multiple Response");
    };
    assert_eq!(values.len(), 22);
}

/// Core 6.3 Vol 3 Part G 2.5.3: Primary and Secondary Service are both grouping types for Read By
/// Group Type; Characteristic is not.
#[test]
fn group_type_secondary_service_is_supported_characteristic_is_not() {
    let (mut s, _h) = one_char_server(Props::READ, Value::Static(vec![1]), Level::Open);
    let group = |s: &mut Server<Rec>, t: u16| {
        s.handle(&Pdu::ReadByGroupTypeReq {
            start: 1,
            end: 0xFFFF,
            group_type: Uuid::from_u16(t),
        })
    };
    assert_eq!(
        group(&mut s, 0x2801),
        err(0x10, 1, ErrorCode::AttributeNotFound)
    );
    assert_eq!(
        group(&mut s, 0x2803),
        err(0x10, 1, ErrorCode::UnsupportedGroupType)
    );
}

/// Core 6.3 Vol 3 Part F 3.4.4.1: an attribute that would cause an error ends the list, and only
/// the attributes before it are returned; when the first one fails its error is the response.
#[test]
fn read_by_type_stops_at_the_first_unreadable_match() {
    let mut b = Database::builder();
    b.service(Uuid::from_u16(0xFFE0)).unwrap();
    let a = b
        .characteristic(
            Uuid::from_u16(0xFFE1),
            Props::READ,
            Value::Static(vec![1, 1]),
        )
        .unwrap();
    let hidden = b
        .characteristic(
            Uuid::from_u16(0xFFE1),
            Props::READ,
            Value::Static(vec![2, 2]),
        )
        .unwrap();
    let c = b
        .characteristic(
            Uuid::from_u16(0xFFE1),
            Props::READ,
            Value::Static(vec![3, 3]),
        )
        .unwrap();
    b.require(hidden, Level::Encrypted, Level::Open).unwrap();
    let mut s = Server::new(b.build(), Rec::default(), 247);
    let req = |s: &mut Server<Rec>, start: u16| {
        s.handle(&Pdu::ReadByTypeReq {
            start,
            end: 0xFFFF,
            attr_type: Uuid::from_u16(0xFFE1),
        })
    };
    // The first match is returned; the unreadable second ends the list before the third.
    assert_eq!(
        req(&mut s, 1),
        Some(Pdu::ReadByTypeRsp {
            entries: vec![(a, vec![1, 1])]
        })
    );
    // Starting at the unreadable one, its error is the response, with its handle.
    assert_eq!(
        req(&mut s, hidden),
        err(0x08, hidden, ErrorCode::InsufficientEncryption)
    );
    s.set_link_level(Level::Encrypted);
    let Some(Pdu::ReadByTypeRsp { entries }) = req(&mut s, 1) else {
        panic!("expected a response");
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[2].0, c);
}

/// Core 6.3 Vol 3 Part F 3.4.4.10: every Attribute Data in one response has the length the Length
/// field gives, so a 16-bit service and a 128-bit service never share a response.
#[test]
fn read_by_group_type_does_not_mix_16_and_128_bit_services() {
    let mut b = Database::builder();
    let first = b.service(Uuid::from_u16(0xFFE0)).unwrap();
    let second = b
        .service(Uuid::from_u128(0x1234_5678_9ABC_DEF0_1122_3344_5566_7788))
        .unwrap();
    let mut s = Server::new(b.build(), Rec::default(), 247);
    s.handle(&Pdu::MtuReq { mtu: 247 });
    let group = |s: &mut Server<Rec>, start: u16| {
        let Some(Pdu::ReadByGroupTypeRsp { entries }) = s.handle(&Pdu::ReadByGroupTypeReq {
            start,
            end: 0xFFFF,
            group_type: Uuid::from_u16(0x2800),
        }) else {
            panic!("expected a response");
        };
        entries.iter().map(|e| e.start).collect::<Vec<_>>()
    };
    assert_eq!(group(&mut s, 1), vec![first]);
    assert_eq!(group(&mut s, first + 1), vec![second]);
}

/// Bookkeeping of this crate (the specification is silent): turning indications off drops the
/// indications still queued.
#[test]
fn disabling_indications_drops_queued_indications() {
    let (mut s, h) = server();
    let write = |s: &mut Server<Rec>, bits: u8| {
        s.handle(&Pdu::WriteReq {
            handle: h.ind_cccd,
            value: vec![bits, 0],
        })
    };
    assert_eq!(write(&mut s, 2), Some(Pdu::WriteRsp));
    s.indicate(h.ind, &[1]).unwrap();
    assert_eq!(write(&mut s, 0), Some(Pdu::WriteRsp));
    assert_eq!(s.poll_outgoing(), None);
}
