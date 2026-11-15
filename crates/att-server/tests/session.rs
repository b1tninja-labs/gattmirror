//! Drives a `Server` the way a GATT client would, through the public API only.

use att_server::{
    Access, Backend, Database, ErrorCode, GroupEntry, NoBackend, Pdu, Props, SendError, Server,
    Subscription, Uuid, Value,
};

const SVC: u16 = 0xFFF0;
const NOTIFY_CHAR: u16 = 0xFFF1;
const WRITE_CHAR: u16 = 0xFFF2;

/// Handles of the fixture table below.
struct Handles {
    notify_value: u16,
    notify_cccd: u16,
    write_value: u16,
    name_value: u16,
    live_value: u16,
}

fn fixture() -> (Database, Handles) {
    let mut b = Database::builder();
    b.service(Uuid::from_u16(0x1800)).unwrap();
    let name_value = b
        .characteristic(
            Uuid::from_u16(0x2A00),
            Props::READ,
            Value::Static(b"mirror".to_vec()),
        )
        .unwrap();
    b.service(Uuid::from_u16(SVC)).unwrap();
    let notify_value = b
        .characteristic(
            Uuid::from_u16(NOTIFY_CHAR),
            Props::NOTIFY | Props::INDICATE,
            Value::Dynamic,
        )
        .unwrap();
    let write_value = b
        .characteristic(
            Uuid::from_u16(WRITE_CHAR),
            Props::WRITE | Props::WRITE_NO_RSP,
            Value::Dynamic,
        )
        .unwrap();
    let live_value = b
        .characteristic(
            Uuid::from_u128(0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E),
            Props::READ,
            Value::Dynamic,
        )
        .unwrap();
    b.descriptor(
        Uuid::from_u16(0x2901),
        Access::READ,
        Value::Static(b"d".to_vec()),
    )
    .unwrap();
    let db = b.build();
    let notify_cccd = notify_value + 1;
    (
        db,
        Handles {
            notify_value,
            notify_cccd,
            write_value,
            name_value,
            live_value,
        },
    )
}

#[derive(Default)]
struct Mem {
    live: Vec<u8>,
    writes: Vec<(u16, Vec<u8>, bool)>,
    subs: Vec<(u16, Subscription)>,
    refuse_writes: bool,
}

impl Backend for Mem {
    fn read(&mut self, _handle: u16) -> Result<Vec<u8>, ErrorCode> {
        Ok(self.live.clone())
    }
    fn write(&mut self, handle: u16, value: &[u8], with_response: bool) -> Result<(), ErrorCode> {
        if self.refuse_writes {
            return Err(ErrorCode::Unlikely);
        }
        self.writes.push((handle, value.to_vec(), with_response));
        Ok(())
    }
    fn subscription_changed(&mut self, h: u16, s: Subscription) {
        self.subs.push((h, s));
    }
}

fn server() -> (Server<Mem>, Handles) {
    let (db, h) = fixture();
    (Server::new(db, Mem::default(), 247), h)
}

#[allow(clippy::needless_pass_by_value)] // call sites build the request inline
fn rsp(s: &mut Server<Mem>, p: Pdu) -> Pdu {
    s.handle(&p).expect("a response")
}

fn error(req: u8, handle: u16, code: ErrorCode) -> Pdu {
    Pdu::ErrorRsp {
        request: req,
        handle,
        code,
    }
}

#[test]
fn mtu_exchange_takes_the_smaller_and_a_repeat_recomputes_it() {
    let (mut s, _) = server();
    assert_eq!(s.mtu(), 23);
    assert_eq!(
        rsp(&mut s, Pdu::MtuReq { mtu: 185 }),
        Pdu::MtuRsp { mtu: 247 }
    );
    assert_eq!(s.mtu(), 185);
    // Android and Bumble answer a repeat and recompute; so does this server.
    assert_eq!(
        rsp(&mut s, Pdu::MtuReq { mtu: 100 }),
        Pdu::MtuRsp { mtu: 247 }
    );
    assert_eq!(s.mtu(), 100);
}

#[test]
fn mtu_never_goes_below_the_default_or_above_the_servers() {
    let (mut s, _) = server();
    s.handle(&Pdu::MtuReq { mtu: 5 });
    assert_eq!(s.mtu(), 23);
    s.reset();
    s.handle(&Pdu::MtuReq { mtu: 517 });
    assert_eq!(s.mtu(), 247);
    s.reset();
    assert_eq!(s.mtu(), 23);
}

#[test]
fn primary_service_discovery() {
    let (mut s, _) = server();
    let r = rsp(
        &mut s,
        Pdu::ReadByGroupTypeReq {
            start: 1,
            end: 0xFFFF,
            group_type: Uuid::from_u16(0x2800),
        },
    );
    assert_eq!(
        r,
        Pdu::ReadByGroupTypeRsp {
            entries: vec![
                GroupEntry {
                    start: 1,
                    end: 3,
                    value: vec![0x00, 0x18]
                },
                GroupEntry {
                    start: 4,
                    end: 12,
                    value: vec![0xF0, 0xFF]
                },
            ]
        }
    );
    // Past the last service.
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadByGroupTypeReq {
                start: 15,
                end: 0xFFFF,
                group_type: Uuid::from_u16(0x2800)
            }
        ),
        error(0x10, 15, ErrorCode::AttributeNotFound)
    );
    // Only services group.
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadByGroupTypeReq {
                start: 1,
                end: 0xFFFF,
                group_type: Uuid::from_u16(0x2803)
            }
        ),
        error(0x10, 1, ErrorCode::UnsupportedGroupType)
    );
}

#[test]
fn bad_handle_ranges_are_invalid_handle() {
    let (mut s, _) = server();
    assert_eq!(
        rsp(&mut s, Pdu::FindInfoReq { start: 0, end: 5 }),
        error(0x04, 0, ErrorCode::InvalidHandle)
    );
    assert_eq!(
        rsp(&mut s, Pdu::FindInfoReq { start: 9, end: 5 }),
        error(0x04, 9, ErrorCode::InvalidHandle)
    );
}

#[test]
fn characteristic_discovery_by_type() {
    let (mut s, h) = server();
    let r = rsp(
        &mut s,
        Pdu::ReadByTypeReq {
            start: 4,
            end: 14,
            attr_type: Uuid::from_u16(0x2803),
        },
    );
    let Pdu::ReadByTypeRsp { entries } = r else {
        panic!("{r:?}")
    };
    // Three characteristics in service 2; the third has a 128-bit UUID so its declaration is longer
    // and must not be mixed into the same response.
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, h.notify_value - 1);
    assert_eq!(
        entries[0].1,
        vec![0x30, u8::try_from(h.notify_value).unwrap(), 0, 0xF1, 0xFF]
    );
    // Continue after the last one returned: the 128-bit one comes alone.
    let next = entries[1].0 + 1;
    let r = rsp(
        &mut s,
        Pdu::ReadByTypeReq {
            start: next,
            end: 0xFFFF,
            attr_type: Uuid::from_u16(0x2803),
        },
    );
    let Pdu::ReadByTypeRsp { entries } = r else {
        panic!("{r:?}")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.len(), 1 + 2 + 16);
}

#[test]
fn find_information_lists_uuids_of_one_width() {
    let (mut s, h) = server();
    let r = rsp(
        &mut s,
        Pdu::FindInfoReq {
            start: h.notify_value,
            end: h.notify_cccd,
        },
    );
    assert_eq!(
        r,
        Pdu::FindInfoRsp {
            entries: vec![
                (h.notify_value, Uuid::from_u16(NOTIFY_CHAR)),
                (h.notify_cccd, Uuid::from_u16(0x2902))
            ]
        }
    );
    // Across the 128-bit characteristic: narrow entries stop where the wide one begins.
    let r = rsp(
        &mut s,
        Pdu::FindInfoReq {
            start: h.write_value,
            end: 0xFFFF,
        },
    );
    let Pdu::FindInfoRsp { entries } = r else {
        panic!("{r:?}")
    };
    assert_eq!(entries.len(), 2);
    let r = rsp(
        &mut s,
        Pdu::FindInfoReq {
            start: h.live_value,
            end: 0xFFFF,
        },
    );
    let Pdu::FindInfoRsp { entries } = r else {
        panic!("{r:?}")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0],
        (
            h.live_value,
            Uuid::from_u128(0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E)
        )
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::FindInfoReq {
                start: 15,
                end: 0xFFFF
            }
        ),
        error(0x04, 15, ErrorCode::AttributeNotFound)
    );
}

#[test]
fn find_by_type_value_finds_a_service_and_its_extent() {
    let (mut s, _) = server();
    let r = rsp(
        &mut s,
        Pdu::FindByTypeValueReq {
            start: 1,
            end: 0xFFFF,
            attr_type: 0x2800,
            value: vec![0xF0, 0xFF],
        },
    );
    assert_eq!(
        r,
        Pdu::FindByTypeValueRsp {
            ranges: vec![(4, 12)]
        }
    );
    let r = rsp(
        &mut s,
        Pdu::FindByTypeValueReq {
            start: 1,
            end: 0xFFFF,
            attr_type: 0x2800,
            value: vec![0x12, 0x34],
        },
    );
    assert_eq!(r, error(0x06, 1, ErrorCode::AttributeNotFound));
}

#[test]
fn reads_come_from_the_table_the_backend_and_the_cccd() {
    let (mut s, h) = server();
    s.backend().live = vec![1, 2, 3];
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadReq {
                handle: h.name_value
            }
        ),
        Pdu::ReadRsp {
            value: b"mirror".to_vec()
        }
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadReq {
                handle: h.live_value
            }
        ),
        Pdu::ReadRsp {
            value: vec![1, 2, 3]
        }
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadReq {
                handle: h.notify_cccd
            }
        ),
        Pdu::ReadRsp { value: vec![0, 0] }
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadReq {
                handle: h.write_value
            }
        ),
        error(0x0A, h.write_value, ErrorCode::ReadNotPermitted)
    );
    assert_eq!(
        rsp(&mut s, Pdu::ReadReq { handle: 999 }),
        error(0x0A, 999, ErrorCode::InvalidHandle)
    );
}

#[test]
fn long_values_are_cut_to_the_mtu_and_read_blob_continues() {
    let (mut s, h) = server();
    s.backend().live = (0..50).collect();
    // MTU 23: a read carries at most 22 value bytes.
    let Pdu::ReadRsp { value } = rsp(
        &mut s,
        Pdu::ReadReq {
            handle: h.live_value,
        },
    ) else {
        panic!()
    };
    assert_eq!(value, (0..22).collect::<Vec<u8>>());
    let Pdu::ReadBlobRsp { value } = rsp(
        &mut s,
        Pdu::ReadBlobReq {
            handle: h.live_value,
            offset: 22,
        },
    ) else {
        panic!()
    };
    assert_eq!(value, (22..44).collect::<Vec<u8>>());
    let Pdu::ReadBlobRsp { value } = rsp(
        &mut s,
        Pdu::ReadBlobReq {
            handle: h.live_value,
            offset: 44,
        },
    ) else {
        panic!()
    };
    assert_eq!(value, (44..50).collect::<Vec<u8>>());
    // Offset exactly at the end is an empty part; past it is an error.
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadBlobReq {
                handle: h.live_value,
                offset: 50
            }
        ),
        Pdu::ReadBlobRsp { value: vec![] }
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadBlobReq {
                handle: h.live_value,
                offset: 51
            }
        ),
        error(0x0C, h.live_value, ErrorCode::InvalidOffset)
    );
    // A bigger MTU lets one read carry it all.
    s.handle(&Pdu::MtuReq { mtu: 100 });
    let Pdu::ReadRsp { value } = rsp(
        &mut s,
        Pdu::ReadReq {
            handle: h.live_value,
        },
    ) else {
        panic!()
    };
    assert_eq!(value.len(), 50);
}

#[test]
fn write_request_reaches_the_backend_and_write_command_gets_no_answer() {
    let (mut s, h) = server();
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: h.write_value,
                value: vec![9, 8]
            }
        ),
        Pdu::WriteRsp
    );
    assert_eq!(
        s.handle(&Pdu::WriteCmd {
            handle: h.write_value,
            value: vec![7]
        }),
        None
    );
    assert_eq!(
        s.backend().writes,
        vec![
            (h.write_value, vec![9, 8], true),
            (h.write_value, vec![7], false)
        ]
    );
}

#[test]
fn writes_respect_properties() {
    let (mut s, h) = server();
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: h.live_value,
                value: vec![1]
            }
        ),
        error(0x12, h.live_value, ErrorCode::WriteNotPermitted)
    );
    // A command to a characteristic without write-without-response is dropped, not executed.
    assert_eq!(
        s.handle(&Pdu::WriteCmd {
            handle: h.notify_value,
            value: vec![1]
        }),
        None
    );
    assert_eq!(s.backend().writes, []);
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: 500,
                value: vec![1]
            }
        ),
        error(0x12, 500, ErrorCode::InvalidHandle)
    );
}

#[test]
fn a_backend_refusal_becomes_the_error_response() {
    let (mut s, h) = server();
    s.backend().refuse_writes = true;
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: h.write_value,
                value: vec![1]
            }
        ),
        error(0x12, h.write_value, ErrorCode::Unlikely)
    );
}

#[test]
fn a_write_to_a_static_attribute_updates_the_table_only_if_the_backend_accepts() {
    let mut b = Database::builder();
    b.service(Uuid::from_u16(1)).unwrap();
    let v = b
        .characteristic(
            Uuid::from_u16(2),
            Props::READ | Props::WRITE,
            Value::Static(vec![0]),
        )
        .unwrap();
    let mut s = Server::new(b.build(), Mem::default(), 23);
    assert_eq!(
        s.handle(&Pdu::WriteReq {
            handle: v,
            value: vec![5, 6]
        }),
        Some(Pdu::WriteRsp)
    );
    assert_eq!(
        s.handle(&Pdu::ReadReq { handle: v }),
        Some(Pdu::ReadRsp { value: vec![5, 6] })
    );
    s.backend().refuse_writes = true;
    s.handle(&Pdu::WriteReq {
        handle: v,
        value: vec![7],
    });
    assert_eq!(
        s.handle(&Pdu::ReadReq { handle: v }),
        Some(Pdu::ReadRsp { value: vec![5, 6] })
    );
}

#[test]
fn cccd_subscription_gates_notifications() {
    let (mut s, h) = server();
    assert_eq!(
        s.notify(h.notify_value, &[1]),
        Err(SendError::NotSubscribed)
    );
    assert_eq!(
        s.notify(h.name_value, &[1]),
        Err(SendError::NotSubscribable)
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: h.notify_cccd,
                value: vec![1, 0]
            }
        ),
        Pdu::WriteRsp
    );
    assert_eq!(
        s.subscription(h.notify_value),
        Subscription {
            notify: true,
            indicate: false
        }
    );
    assert_eq!(
        s.backend().subs,
        vec![(
            h.notify_value,
            Subscription {
                notify: true,
                indicate: false
            }
        )]
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::ReadReq {
                handle: h.notify_cccd
            }
        ),
        Pdu::ReadRsp { value: vec![1, 0] }
    );
    s.notify(h.notify_value, &[0xAA, 0xBB]).unwrap();
    assert_eq!(
        s.indicate(h.notify_value, &[1]),
        Err(SendError::NotSubscribed)
    );
    assert_eq!(
        s.poll_outgoing(),
        Some(Pdu::Notification {
            handle: h.notify_value,
            value: vec![0xAA, 0xBB]
        })
    );
    assert_eq!(s.poll_outgoing(), None);
}

#[test]
fn cccd_writes_are_validated() {
    let (mut s, h) = server();
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: h.notify_cccd,
                value: vec![1]
            }
        ),
        error(0x12, h.notify_cccd, ErrorCode::InvalidAttributeValueLength)
    );
    assert_eq!(
        rsp(
            &mut s,
            Pdu::WriteReq {
                handle: h.notify_cccd,
                value: vec![4, 0]
            }
        ),
        error(0x12, h.notify_cccd, ErrorCode::Other(0xFD))
    );
}

#[test]
fn unsubscribing_drops_queued_updates() {
    let (mut s, h) = server();
    s.handle(&Pdu::WriteReq {
        handle: h.notify_cccd,
        value: vec![1, 0],
    });
    s.notify(h.notify_value, &[1]).unwrap();
    s.handle(&Pdu::WriteReq {
        handle: h.notify_cccd,
        value: vec![0, 0],
    });
    assert_eq!(s.poll_outgoing(), None);
}

#[test]
fn only_one_indication_is_outstanding_until_confirmed() {
    let (mut s, h) = server();
    s.handle(&Pdu::WriteReq {
        handle: h.notify_cccd,
        value: vec![3, 0],
    });
    s.indicate(h.notify_value, &[1]).unwrap();
    s.indicate(h.notify_value, &[2]).unwrap();
    s.notify(h.notify_value, &[3]).unwrap();
    assert_eq!(
        s.poll_outgoing(),
        Some(Pdu::Indication {
            handle: h.notify_value,
            value: vec![1]
        })
    );
    assert!(s.indication_pending());
    // The second indication waits; the notification behind it is not blocked.
    assert_eq!(
        s.poll_outgoing(),
        Some(Pdu::Notification {
            handle: h.notify_value,
            value: vec![3]
        })
    );
    assert_eq!(s.poll_outgoing(), None);
    assert_eq!(s.handle(&Pdu::Confirmation), None);
    assert!(!s.indication_pending());
    assert_eq!(
        s.poll_outgoing(),
        Some(Pdu::Indication {
            handle: h.notify_value,
            value: vec![2]
        })
    );
}

#[test]
fn outgoing_values_are_cut_to_the_mtu() {
    let (mut s, h) = server();
    s.handle(&Pdu::WriteReq {
        handle: h.notify_cccd,
        value: vec![1, 0],
    });
    s.notify(h.notify_value, &[7; 100]).unwrap();
    let Some(Pdu::Notification { value, .. }) = s.poll_outgoing() else {
        panic!()
    };
    assert_eq!(value.len(), 20);
    assert_eq!(s.notify(h.notify_value, &[0; 513]), Err(SendError::TooLong));
}

#[test]
fn prepared_writes_are_assembled_on_execute() {
    let (mut s, h) = server();
    let w = h.write_value;
    for (off, part) in [(0_u16, vec![1, 2]), (2, vec![3, 4])] {
        assert_eq!(
            rsp(
                &mut s,
                Pdu::PrepareWriteReq {
                    handle: w,
                    offset: off,
                    value: part.clone()
                }
            ),
            Pdu::PrepareWriteRsp {
                handle: w,
                offset: off,
                value: part
            }
        );
    }
    assert_eq!(s.backend().writes, []);
    assert_eq!(
        rsp(&mut s, Pdu::ExecuteWriteReq { flags: 1 }),
        Pdu::ExecuteWriteRsp
    );
    assert_eq!(s.backend().writes, vec![(w, vec![1, 2, 3, 4], true)]);
}

#[test]
fn cancelled_prepared_writes_never_reach_the_backend() {
    let (mut s, h) = server();
    s.handle(&Pdu::PrepareWriteReq {
        handle: h.write_value,
        offset: 0,
        value: vec![1],
    });
    assert_eq!(
        rsp(&mut s, Pdu::ExecuteWriteReq { flags: 0 }),
        Pdu::ExecuteWriteRsp
    );
    assert_eq!(
        rsp(&mut s, Pdu::ExecuteWriteReq { flags: 1 }),
        Pdu::ExecuteWriteRsp
    );
    assert_eq!(s.backend().writes, []);
}

#[test]
fn a_gap_in_prepared_writes_fails_the_execute() {
    let (mut s, h) = server();
    s.handle(&Pdu::PrepareWriteReq {
        handle: h.write_value,
        offset: 5,
        value: vec![1],
    });
    assert_eq!(
        rsp(&mut s, Pdu::ExecuteWriteReq { flags: 1 }),
        error(0x18, h.write_value, ErrorCode::InvalidOffset)
    );
    assert_eq!(s.backend().writes, []);
}

#[test]
fn prepare_checks_permission_and_queue_space() {
    let (mut s, h) = server();
    assert_eq!(
        rsp(
            &mut s,
            Pdu::PrepareWriteReq {
                handle: h.live_value,
                offset: 0,
                value: vec![1]
            }
        ),
        error(0x16, h.live_value, ErrorCode::WriteNotPermitted)
    );
    for _ in 0..64 {
        s.handle(&Pdu::PrepareWriteReq {
            handle: h.write_value,
            offset: 0,
            value: vec![1],
        });
    }
    assert_eq!(
        rsp(
            &mut s,
            Pdu::PrepareWriteReq {
                handle: h.write_value,
                offset: 0,
                value: vec![1]
            }
        ),
        error(0x16, h.write_value, ErrorCode::PrepareQueueFull)
    );
}

#[test]
fn raw_bytes_in_and_out() {
    let (mut s, h) = server();
    assert_eq!(
        s.handle_bytes(&[0x02, 0xB9, 0x00]),
        Some(vec![0x03, 0xF7, 0x00])
    );
    assert_eq!(
        s.handle_bytes(&[0x12, u8::try_from(h.write_value).unwrap(), 0, 0x55]),
        Some(vec![0x13])
    );
    // Truncated read request: Invalid PDU naming the opcode.
    assert_eq!(
        s.handle_bytes(&[0x0A, 1]),
        Some(vec![0x01, 0x0A, 0, 0, 0x04])
    );
    // Unknown request opcode: Request Not Supported; unknown command: silence; empty: silence.
    assert_eq!(
        s.handle_bytes(&[0x20, 1, 0]),
        Some(vec![0x01, 0x20, 0, 0, 0x06])
    );
    assert_eq!(s.handle_bytes(&[0x52 | 0x80, 1, 0, 1]), None);
    assert_eq!(s.handle_bytes(&[]), None);
    // A truncated command has no response either.
    assert_eq!(s.handle_bytes(&[0x52, 1]), None);
}

#[test]
fn a_server_ignores_pdus_only_a_server_sends() {
    let (mut s, _) = server();
    assert_eq!(s.handle(&Pdu::ReadRsp { value: vec![] }), None);
    assert_eq!(
        s.handle(&Pdu::Notification {
            handle: 1,
            value: vec![]
        }),
        None
    );
    assert_eq!(s.handle(&Pdu::WriteRsp), None);
}

#[test]
fn no_backend_refuses_dynamic_values() {
    let (db, h) = fixture();
    let mut s = Server::new(db, NoBackend, 23);
    assert_eq!(
        s.handle(&Pdu::ReadReq {
            handle: h.live_value
        }),
        Some(error(0x0A, h.live_value, ErrorCode::ReadNotPermitted))
    );
    assert_eq!(
        s.handle(&Pdu::WriteReq {
            handle: h.write_value,
            value: vec![1]
        }),
        Some(error(0x12, h.write_value, ErrorCode::WriteNotPermitted))
    );
}

#[test]
fn find_by_type_value_matches_the_type_as_well_as_the_value() {
    let (mut s, _) = server();
    // The user-description descriptor holds b"d" but is not a service declaration.
    let r = s
        .handle(&Pdu::FindByTypeValueReq {
            start: 1,
            end: 0xFFFF,
            attr_type: 0x2800,
            value: b"d".to_vec(),
        })
        .unwrap();
    assert_eq!(r, error(0x06, 1, ErrorCode::AttributeNotFound));
}

#[test]
fn read_by_type_returns_only_entries_of_one_length() {
    let (mut s, h) = server();
    s.handle(&Pdu::MtuReq { mtu: 247 });
    // Declarations in service 2 are 5, 5 and 19 bytes; with room for all three the long one must
    // still be left for the next request.
    let r = s
        .handle(&Pdu::ReadByTypeReq {
            start: h.notify_value - 1,
            end: 0xFFFF,
            attr_type: Uuid::from_u16(0x2803),
        })
        .unwrap();
    let Pdu::ReadByTypeRsp { entries } = r else {
        panic!("{r:?}")
    };
    assert_eq!(entries.len(), 2);
}
