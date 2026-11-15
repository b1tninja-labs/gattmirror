//! One LE connection at a time: connect, ACL, the fixed channels, flow control, re-advertise.
//!
//! Event codes, subevent codes and field layouts here are from memory and `[unread]`; each was
//! checked against a live controller where `ROADMAP.md` says so.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use att_server::{Backend, Server};
use hci_wire::{EventCode, Packet, opcodes};

use crate::advertising::{AdvParams, Advertisement};
use crate::controller::{Controller, ControllerInfo, Error};
use crate::l2cap::{self, CID_ATT, CID_SIGNALLING, CID_SMP, Reassembler};

/// LE Meta subevent: LE Connection Complete.
const SUB_CONN_COMPLETE: u8 = 0x01;
/// LE Meta subevent: LE Enhanced Connection Complete.
const SUB_ENH_CONN_COMPLETE: u8 = 0x0A;
/// Disconnect reason: remote user terminated connection.
const REASON_USER_TERMINATED: u8 = 0x13;

enum Cmd {
    Update {
        handle: u16,
        value: Vec<u8>,
        indicate: bool,
    },
    Disconnect,
    Stop,
}

/// A cloneable, thread-safe handle on a running [`Peripheral`].
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Cmd>,
}

// The booleans only say whether the peripheral still exists; callers may ignore them.
#[allow(clippy::must_use_candidate)]
impl Handle {
    /// Queue a notification of `value_handle`. It is sent if a client is connected and has enabled
    /// notifications by the time the peripheral loop sees it; otherwise it is counted in
    /// [`Peripheral::dropped_updates`]. Returns false if the peripheral has gone.
    pub fn notify(&self, value_handle: u16, value: &[u8]) -> bool {
        self.send(value_handle, value, false)
    }

    /// As [`Handle::notify`], for an indication.
    pub fn indicate(&self, value_handle: u16, value: &[u8]) -> bool {
        self.send(value_handle, value, true)
    }

    fn send(&self, handle: u16, value: &[u8], indicate: bool) -> bool {
        self.tx
            .send(Cmd::Update {
                handle,
                value: value.to_vec(),
                indicate,
            })
            .is_ok()
    }

    /// Ask the peripheral to drop the current connection (it then advertises again).
    pub fn disconnect(&self) -> bool {
        self.tx.send(Cmd::Disconnect).is_ok()
    }

    /// Ask [`Peripheral::serve`] to return: it disconnects and stops advertising first.
    pub fn stop(&self) -> bool {
        self.tx.send(Cmd::Stop).is_ok()
    }
}

struct Conn {
    handle: u16,
    reasm: Reassembler,
}

/// A connectable LE peripheral serving one ATT [`Server`].
pub struct Peripheral<T: Read + Write, B: Backend> {
    ctl: Controller<T>,
    info: ControllerInfo,
    ad: Advertisement,
    params: AdvParams,
    server: Server<B>,
    rx: Receiver<Cmd>,
    conn: Option<Conn>,
    tx_queue: VecDeque<Packet>,
    in_flight: u16,
    connections: usize,
    dropped: usize,
    stopping: bool,
}

impl<T: Read + Write, B: Backend> Peripheral<T, B> {
    /// Build a peripheral over an initialised controller (`info` from [`Controller::init`]). The
    /// transport should have a read timeout, which sets how often queued work is noticed.
    pub fn new(
        ctl: Controller<T>,
        info: ControllerInfo,
        ad: Advertisement,
        params: AdvParams,
        server: Server<B>,
    ) -> (Self, Handle) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                ctl,
                info,
                ad,
                params,
                server,
                rx,
                conn: None,
                tx_queue: VecDeque::new(),
                in_flight: 0,
                connections: 0,
                dropped: 0,
                stopping: false,
            },
            Handle { tx },
        )
    }

    /// Advertise, then serve connections until [`Handle::stop`] (returns `Ok`) or a transport
    /// error.
    ///
    /// # Errors
    ///
    /// Any [`Error`] from the transport or the controller.
    pub fn serve(&mut self) -> Result<(), Error> {
        self.ctl.start_advertising(&self.ad, &self.params)?;
        loop {
            // Take what has arrived, but not so much that a busy controller starves the queue.
            let mut read = 0;
            for _ in 0..64 {
                match self.ctl.recv()? {
                    Some(p) => {
                        read += 1;
                        self.on_packet(p)?;
                    }
                    None => break,
                }
            }
            let stop = self.drain_commands()?;
            let sent = self.pump()?;
            // A stop waits for a quiet pass, so what the controller has already said is handled.
            if stop {
                self.stopping = true;
            }
            if self.stopping && read == 0 && sent == 0 {
                if let Some(h) = self.connection_handle() {
                    self.disconnect(h)?;
                }
                return self.ctl.stop_advertising();
            }
        }
    }

    /// The server, for inspection.
    pub fn server(&mut self) -> &mut Server<B> {
        &mut self.server
    }

    /// The connection handle, while connected.
    #[must_use]
    pub fn connection_handle(&self) -> Option<u16> {
        self.conn.as_ref().map(|c| c.handle)
    }

    /// How many connections have been accepted so far.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.connections
    }

    /// Notifications and indications that were not sent (no client, or not subscribed).
    pub fn dropped_updates(&self) -> usize {
        self.dropped
    }

    /// Take the controller back.
    pub fn into_controller(self) -> Controller<T> {
        self.ctl
    }

    fn on_packet(&mut self, packet: Packet) -> Result<(), Error> {
        match packet {
            Packet::Event { code, params } => self.on_event(code, &params),
            Packet::Acl {
                handle,
                boundary,
                data,
                ..
            } => {
                let Some(conn) = self.conn.as_mut().filter(|c| c.handle == handle) else {
                    return Ok(());
                };
                if let Some((cid, payload)) = conn.reasm.push(boundary, &data) {
                    self.on_frame(cid, &payload);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn on_event(&mut self, code: u8, params: &[u8]) -> Result<(), Error> {
        match EventCode(code) {
            EventCode::LE_META => {
                let Some((&sub, rest)) = params.split_first() else {
                    return Ok(());
                };
                if matches!(sub, SUB_CONN_COMPLETE | SUB_ENH_CONN_COMPLETE) {
                    // status, handle (2), role: the same prefix in both layouts.
                    if let [status, h0, h1, _role, ..] = rest {
                        return self.on_connect(*status, u16::from_le_bytes([*h0, *h1]));
                    }
                }
                Ok(())
            }
            EventCode::DISCONNECTION_COMPLETE => {
                if let [status, h0, h1, _reason] = params {
                    let h = u16::from_le_bytes([*h0, *h1]);
                    if *status == 0 && self.connection_handle() == Some(h) {
                        return self.on_disconnect();
                    }
                }
                Ok(())
            }
            EventCode::NUMBER_OF_COMPLETED_PACKETS => {
                let Some((&n, rest)) = params.split_first() else {
                    return Ok(());
                };
                let n = usize::from(n);
                if rest.len() < n * 4 {
                    return Ok(());
                }
                let (handles, counts) = rest.split_at(n * 2);
                for (h, c) in handles.chunks_exact(2).zip(counts.chunks_exact(2)) {
                    if self.connection_handle() == Some(u16::from_le_bytes([h[0], h[1]])) {
                        self.in_flight = self
                            .in_flight
                            .saturating_sub(u16::from_le_bytes([c[0], c[1]]));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn on_connect(&mut self, status: u8, handle: u16) -> Result<(), Error> {
        if status != 0 {
            // A failed attempt: the advertising set is gone with it, so start again.
            return self.ctl.start_advertising(&self.ad, &self.params);
        }
        self.server.reset();
        self.tx_queue.clear();
        self.in_flight = 0;
        self.connections += 1;
        self.conn = Some(Conn {
            handle,
            reasm: Reassembler::default(),
        });
        Ok(())
    }

    fn on_disconnect(&mut self) -> Result<(), Error> {
        self.conn = None;
        self.server.reset();
        self.tx_queue.clear();
        self.in_flight = 0;
        self.ctl.start_advertising(&self.ad, &self.params)
    }

    fn on_frame(&mut self, cid: u16, payload: &[u8]) {
        let reply = match cid {
            CID_ATT => self.server.handle_bytes(payload),
            CID_SIGNALLING => l2cap::signalling_reply(payload),
            CID_SMP => l2cap::smp_reply(payload),
            _ => None,
        };
        if let Some(r) = reply {
            self.enqueue(cid, &r);
        }
    }

    fn enqueue(&mut self, cid: u16, payload: &[u8]) {
        let Some(conn) = &self.conn else { return };
        let max = usize::from(self.info.le_acl_len.max(1));
        self.tx_queue
            .extend(l2cap::fragment(conn.handle, cid, payload, max));
    }

    /// Returns true once a stop was requested.
    fn drain_commands(&mut self) -> Result<bool, Error> {
        loop {
            match self.rx.try_recv() {
                Ok(Cmd::Update {
                    handle,
                    value,
                    indicate,
                }) => {
                    // With no client the subscriptions are empty, so the server refuses these.
                    let queued = if indicate {
                        self.server.indicate(handle, &value).is_ok()
                    } else {
                        self.server.notify(handle, &value).is_ok()
                    };
                    if !queued {
                        self.dropped += 1;
                    }
                }
                Ok(Cmd::Disconnect) => {
                    if let Some(h) = self.connection_handle() {
                        self.disconnect(h)?;
                    }
                }
                Ok(Cmd::Stop) => return Ok(true),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(false),
            }
        }
    }

    fn disconnect(&mut self, handle: u16) -> Result<(), Error> {
        let mut p = handle.to_le_bytes().to_vec();
        p.push(REASON_USER_TERMINATED);
        self.ctl.command(opcodes::DISCONNECT, &p).map(drop)
    }

    /// Move server output into the ACL queue and write as many packets as the controller has
    /// buffers for. Returns how many ACL packets were written.
    fn pump(&mut self) -> Result<usize, Error> {
        let mut sent = 0;
        while let Some(pdu) = self.server.poll_outgoing() {
            self.enqueue(CID_ATT, &pdu.encode());
        }
        while self.in_flight < u16::from(self.info.le_acl_count) {
            let Some(p) = self.tx_queue.pop_front() else {
                break;
            };
            self.ctl.send(&p)?;
            self.in_flight += 1;
            sent += 1;
        }
        Ok(sent)
    }
}

#[cfg(test)]
#[allow(clippy::assert_is_empty)]
mod tests {
    use att_server::{Database, NoBackend, Pdu, Props, Server, Subscription, Uuid, Value};
    use hci_wire::{Packet, PacketBoundary, opcodes};

    use super::{Handle, Peripheral};
    use crate::advertising::{AdvParams, Advertisement};
    use crate::controller::{Controller, ControllerInfo};
    use crate::fake::Fake;
    use crate::l2cap::{self, CID_ATT, CID_SIGNALLING, CID_SMP, Reassembler};

    const H: u16 = 0x0040;

    struct Table {
        name: u16,
        long: u16,
        notify: u16,
    }

    fn table() -> (Database, Table) {
        let mut b = Database::builder();
        b.service(Uuid::from_u16(0x1800)).unwrap();
        let name = b
            .characteristic(
                Uuid::from_u16(0x2A00),
                Props::READ,
                Value::Static(b"mirror".to_vec()),
            )
            .unwrap();
        let long = b
            .characteristic(
                Uuid::from_u16(0x2A01),
                Props::READ,
                Value::Static((0..100).collect()),
            )
            .unwrap();
        let notify = b
            .characteristic(Uuid::from_u16(0x2A02), Props::NOTIFY, Value::Dynamic)
            .unwrap();
        (b.build(), Table { name, long, notify })
    }

    type Rig = (Peripheral<Fake, NoBackend>, Handle, Table);

    /// A peripheral over an initialised fake controller with `count` ACL buffers. `setup` queues
    /// what the controller will send.
    fn rig(count: u8, setup: impl FnOnce(&mut Fake, &Table)) -> Rig {
        let mut ctl = Controller::new(Fake::default());
        ctl.init().unwrap();
        let mut fake = ctl.into_inner();
        fake.block_when_idle();
        let (db, t) = table();
        setup(&mut fake, &t);
        let info = ControllerInfo {
            bd_addr: [0; 6],
            le_acl_len: 27,
            le_acl_count: count,
        };
        let (p, h) = Peripheral::new(
            Controller::new(fake),
            info,
            Advertisement::new("TEST", None).unwrap(),
            AdvParams::default(),
            Server::new(db, NoBackend, 247),
        );
        (p, h, t)
    }

    /// Stop after everything queued is handled, and take the fake back.
    fn run(p: &mut Peripheral<Fake, NoBackend>, h: &Handle) -> Fake {
        h.stop();
        p.serve().unwrap();
        std::mem::replace(&mut p.ctl, Controller::new(Fake::default())).into_inner()
    }

    fn conn_event(status: u8, handle: u16) -> Vec<u8> {
        let mut params = vec![1, status];
        params.extend_from_slice(&handle.to_le_bytes());
        params.extend_from_slice(&[1, 0, 1, 2, 3, 4, 5, 6, 24, 0, 0, 0, 200, 0, 0]);
        Packet::Event { code: 0x3E, params }.encode()
    }

    fn disc_event(handle: u16) -> Vec<u8> {
        let mut params = vec![0];
        params.extend_from_slice(&handle.to_le_bytes());
        params.push(0x13);
        Packet::Event { code: 0x05, params }.encode()
    }

    fn acl(handle: u16, cid: u16, payload: &[u8]) -> Vec<u8> {
        l2cap::fragment(handle, cid, payload, 255)
            .iter()
            .flat_map(Packet::encode)
            .collect()
    }

    /// Rebuild the frames the host wrote, as `(cid, payload)`.
    fn frames(fake: &Fake) -> Vec<(u16, Vec<u8>)> {
        let mut r = Reassembler::default();
        fake.acl_out()
            .iter()
            .filter_map(|p| {
                let Packet::Acl { boundary, data, .. } = p else {
                    unreachable!()
                };
                r.push(*boundary, data)
            })
            .collect()
    }

    fn att(fake: &Fake) -> Vec<Pdu> {
        frames(fake)
            .iter()
            .filter(|(c, _)| *c == CID_ATT)
            .map(|(_, p)| Pdu::decode(p).unwrap())
            .collect()
    }

    fn adv_enables(fake: &Fake) -> usize {
        fake.commands()
            .iter()
            .filter(|(o, d)| *o == opcodes::LE_SET_ADVERTISING_ENABLE && d == &[1])
            .count()
    }

    #[test]
    fn a_read_over_a_new_connection_is_answered_as_acl() {
        let (mut p, h, _) = rig(20, |f, t| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(H, CID_ATT, &Pdu::ReadReq { handle: t.name }.encode()));
        });
        let fake = run(&mut p, &h);
        assert_eq!(p.connections(), 1);
        assert_eq!(
            att(&fake),
            [Pdu::ReadRsp {
                value: b"mirror".to_vec()
            }]
        );
        let Packet::Acl {
            handle, boundary, ..
        } = &fake.acl_out()[0]
        else {
            unreachable!()
        };
        assert_eq!((*handle, *boundary), (H, PacketBoundary::FirstNonFlushable));
    }

    #[test]
    fn acl_for_another_handle_is_ignored() {
        let (mut p, h, _) = rig(20, |f, t| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(
                H + 1,
                CID_ATT,
                &Pdu::ReadReq { handle: t.name }.encode(),
            ));
        });
        let fake = run(&mut p, &h);
        assert!(fake.acl_out().is_empty());
    }

    #[test]
    fn acl_before_any_connection_is_ignored() {
        let (mut p, h, _) = rig(20, |f, t| {
            f.push_inbound(acl(H, CID_ATT, &Pdu::ReadReq { handle: t.name }.encode()));
        });
        let fake = run(&mut p, &h);
        assert!(fake.acl_out().is_empty());
    }

    #[test]
    fn a_request_split_over_two_fragments_is_reassembled() {
        let (mut p, h, _) = rig(20, |f, t| {
            f.push_inbound(conn_event(0, H));
            let req = Pdu::ReadReq { handle: t.name }.encode();
            let frame = l2cap::fragment(H, CID_ATT, &req, 255).remove(0);
            let Packet::Acl { data, .. } = frame else {
                unreachable!()
            };
            for (i, part) in [&data[..3], &data[3..]].into_iter().enumerate() {
                let boundary = if i == 0 {
                    PacketBoundary::FirstNonFlushable
                } else {
                    PacketBoundary::Continuing
                };
                f.push_inbound(
                    Packet::Acl {
                        handle: H,
                        boundary,
                        broadcast: 0,
                        data: part.to_vec(),
                    }
                    .encode(),
                );
            }
        });
        let fake = run(&mut p, &h);
        assert_eq!(att(&fake).len(), 1);
    }

    #[test]
    fn a_long_response_is_fragmented_to_the_controllers_buffer_size() {
        let (mut p, h, _) = rig(20, |f, t| {
            f.complete_acl_automatically();
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(H, CID_ATT, &Pdu::MtuReq { mtu: 247 }.encode()));
            f.push_inbound(acl(H, CID_ATT, &Pdu::ReadReq { handle: t.long }.encode()));
        });
        let fake = run(&mut p, &h);
        let sizes: Vec<usize> = fake
            .acl_out()
            .iter()
            .map(|p| match p {
                Packet::Acl { data, .. } => data.len(),
                _ => unreachable!(),
            })
            .collect();
        assert!(sizes.iter().all(|s| *s <= 27), "{sizes:?}");
        // The MTU response (3 bytes plus the 4-byte header) in one packet, then 105 bytes in four.
        assert_eq!(sizes, [7, 27, 27, 27, 24]);
        assert_eq!(
            att(&fake)[1],
            Pdu::ReadRsp {
                value: (0..100).collect()
            }
        );
    }

    /// Two buffers and no Number Of Completed Packets: only two packets may be in flight.
    #[test]
    fn no_more_packets_are_sent_than_the_controller_has_buffers() {
        let (mut p, h, _) = rig(2, |f, t| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(H, CID_ATT, &Pdu::MtuReq { mtu: 247 }.encode()));
            f.push_inbound(acl(H, CID_ATT, &Pdu::ReadReq { handle: t.long }.encode()));
        });
        let fake = run(&mut p, &h);
        assert_eq!(fake.acl_out().len(), 2);
    }

    /// The same load with each packet acknowledged: everything gets out.
    #[test]
    fn completed_packets_release_the_queue() {
        let (mut p, h, _) = rig(2, |f, t| {
            f.complete_acl_automatically();
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(H, CID_ATT, &Pdu::MtuReq { mtu: 247 }.encode()));
            f.push_inbound(acl(H, CID_ATT, &Pdu::ReadReq { handle: t.long }.encode()));
        });
        let fake = run(&mut p, &h);
        assert_eq!(fake.acl_out().len(), 5);
    }

    fn subscribe(f: &mut Fake, t: &Table) {
        f.push_inbound(conn_event(0, H));
        f.push_inbound(acl(
            H,
            CID_ATT,
            &Pdu::WriteReq {
                handle: t.notify + 1,
                value: vec![1, 0],
            }
            .encode(),
        ));
    }

    #[test]
    fn a_notification_from_another_thread_reaches_a_subscribed_client() {
        let (mut p, h, t) = rig(20, subscribe);
        let h2 = h.clone();
        let nv = t.notify;
        std::thread::spawn(move || assert!(h2.notify(nv, b"hi")))
            .join()
            .unwrap();
        let fake = run(&mut p, &h);
        let a = att(&fake);
        assert_eq!(a.len(), 2);
        assert_eq!(
            a[1],
            Pdu::Notification {
                handle: t.notify,
                value: b"hi".to_vec()
            }
        );
        assert_eq!(p.dropped_updates(), 0);
    }

    #[test]
    fn a_notification_without_a_subscription_is_dropped_and_counted() {
        let (mut p, h, t) = rig(20, |f, _| f.push_inbound(conn_event(0, H)));
        h.notify(t.notify, b"hi");
        let fake = run(&mut p, &h);
        assert!(fake.acl_out().is_empty());
        assert_eq!(p.dropped_updates(), 1);
    }

    #[test]
    fn a_notification_with_no_connection_is_dropped_and_counted() {
        let (mut p, h, t) = rig(20, |_, _| {});
        h.notify(t.notify, b"hi");
        let fake = run(&mut p, &h);
        assert!(fake.acl_out().is_empty());
        assert_eq!(p.dropped_updates(), 1);
    }

    #[test]
    fn disconnection_forgets_the_client_and_advertises_again() {
        let (mut p, h, t) = rig(20, |f, t| {
            subscribe(f, t);
            f.push_inbound(disc_event(H));
        });
        let fake = run(&mut p, &h);
        assert_eq!(p.connection_handle(), None);
        assert_eq!(p.server().subscription(t.notify), Subscription::default());
        // Once at the start, once after the link dropped.
        assert_eq!(adv_enables(&fake), 2);
    }

    #[test]
    fn a_disconnection_of_another_handle_changes_nothing() {
        let (mut p, h, _) = rig(20, |f, _| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(disc_event(H + 1));
        });
        let fake = run(&mut p, &h);
        assert_eq!(p.connection_handle(), Some(H));
        assert_eq!(adv_enables(&fake), 1);
    }

    #[test]
    fn a_failed_connection_attempt_advertises_again() {
        let (mut p, h, _) = rig(20, |f, _| f.push_inbound(conn_event(0x3E, 0)));
        let fake = run(&mut p, &h);
        assert_eq!(p.connections(), 0);
        assert_eq!(adv_enables(&fake), 2);
    }

    #[test]
    fn a_reconnection_starts_with_a_fresh_server() {
        let (mut p, h, t) = rig(20, |f, t| {
            subscribe(f, t);
            f.push_inbound(disc_event(H));
            f.push_inbound(conn_event(0, H + 2));
        });
        run(&mut p, &h);
        assert_eq!(p.connections(), 2);
        assert!(!p.server().subscription(t.notify).notify);
    }

    #[test]
    fn signalling_requests_are_rejected_over_acl() {
        let (mut p, h, _) = rig(20, |f, _| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(
                H,
                CID_SIGNALLING,
                &[0x12, 9, 8, 0, 6, 0, 6, 0, 0, 0, 0, 0],
            ));
        });
        let fake = run(&mut p, &h);
        assert_eq!(frames(&fake), [(CID_SIGNALLING, vec![1, 9, 2, 0, 0, 0])]);
    }

    #[test]
    fn pairing_is_refused_over_acl() {
        let (mut p, h, _) = rig(20, |f, _| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(H, CID_SMP, &[1, 3, 0, 1, 16, 0, 0]));
        });
        let fake = run(&mut p, &h);
        assert_eq!(frames(&fake), [(CID_SMP, vec![5, 5])]);
    }

    #[test]
    fn an_unknown_channel_gets_no_reply() {
        let (mut p, h, _) = rig(20, |f, _| {
            f.push_inbound(conn_event(0, H));
            f.push_inbound(acl(H, 0x0040, &[1, 2, 3]));
        });
        let fake = run(&mut p, &h);
        assert!(fake.acl_out().is_empty());
    }

    #[test]
    fn disconnect_sends_the_command_for_the_current_handle() {
        let (mut p, h, _) = rig(20, |f, _| f.push_inbound(conn_event(0, H)));
        h.disconnect();
        let fake = run(&mut p, &h);
        let log = fake.commands();
        let d: Vec<_> = log
            .iter()
            .filter(|(o, _)| *o == opcodes::DISCONNECT)
            .collect();
        assert!(!d.is_empty());
        assert_eq!(d[0].1, [0x40, 0x00, 0x13]);
    }

    #[test]
    fn stop_disables_advertising_last() {
        let (mut p, h, _) = rig(20, |_, _| {});
        let fake = run(&mut p, &h);
        let log = fake.commands();
        let last = log.last().unwrap();
        assert_eq!(last.0, opcodes::LE_SET_ADVERTISING_ENABLE);
        assert_eq!(last.1, [0]);
    }
}
