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
