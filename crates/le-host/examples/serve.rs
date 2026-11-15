//! Advertise and serve a small GATT table on the controller at `host:port`.
//!
//! `cargo run -p le-host --example serve -- 127.0.0.1:PORT [NAME] [SECONDS]`
//!
//! The table has a device name, a readable counter-free value, a notifying characteristic fed once a
//! second from another thread, and a writable one. It logs connections and what clients write.

use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use att_server::{Backend, Database, ErrorCode, Props, Server, Subscription, Uuid, Value};
use le_host::{
    AdvParams, Advertisement, Controller, Handle, NotifyPolicy, Peripheral, ServiceUuid,
};

/// Answers every write on the serial characteristic with an ELM327-style reply notification.
struct Log {
    notify_handle: u16,
    to_peer: Arc<Mutex<Option<Handle>>>,
}

impl Backend for Log {
    fn read(&mut self, handle: u16) -> Result<Vec<u8>, ErrorCode> {
        println!("read of dynamic handle {handle:#06x}");
        Ok(b"live".to_vec())
    }
    fn write(&mut self, handle: u16, value: &[u8], with_response: bool) -> Result<(), ErrorCode> {
        println!("write {handle:#06x} rsp={with_response} {value:02x?}");
        let reply: &[u8] = if value.starts_with(b"ATZ") {
            b"\r\rELM327 v1.5\r\r>"
        } else {
            b"OK\r\r>"
        };
        if let Some(h) = self.to_peer.lock().unwrap().as_ref() {
            h.notify(self.notify_handle, reply);
        }
        Ok(())
    }
    fn subscription_changed(&mut self, handle: u16, s: Subscription) {
        println!("subscription {handle:#06x}: {s:?}");
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args
        .next()
        .expect("usage: serve HOST:PORT [NAME] [SECONDS]");
    let name = args.next().unwrap_or_else(|| "GM-TEST-E2".to_owned());
    let secs: u64 = args.next().map_or(120, |s| s.parse().expect("seconds"));

    let mut b = Database::builder();
    b.service(Uuid::from_u16(0x1800)).unwrap();
    b.characteristic(
        Uuid::from_u16(0x2A00),
        Props::READ,
        Value::Static(name.as_bytes().to_vec()),
    )
    .unwrap();
    b.service(Uuid::from_u16(0xFFF0)).unwrap();
    let notify = b
        .characteristic(Uuid::from_u16(0xFFF1), Props::NOTIFY, Value::Dynamic)
        .unwrap();
    b.characteristic(
        Uuid::from_u16(0xFFF2),
        Props::WRITE | Props::WRITE_NO_RSP,
        Value::Dynamic,
    )
    .unwrap();
    let to_peer = Arc::new(Mutex::new(None));
    let server = Server::new(
        b.build(),
        Log {
            notify_handle: notify,
            to_peer: Arc::clone(&to_peer),
        },
        247,
    );

    let stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut ctl = Controller::new(stream);
    let info = ctl.init().expect("init");
    let ad = Advertisement::new(&name, Some(ServiceUuid::Uuid16(0xFFF0))).expect("advertisement");
    let (mut p, handle) = Peripheral::new(ctl, info, ad, AdvParams::default(), server);

    *to_peer.lock().unwrap() = Some(handle.clone());
    p.set_notify_policy(NotifyPolicy::Always);
    p.set_trace(Box::new(|d, cid, b| println!("{d:?} cid {cid}: {b:02x?}")));
    let h = handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        h.stop();
    });
    println!("serving {name:?} for {secs} s");
    let r = p.serve();
    println!(
        "done: {r:?}; connections {}, updates dropped {}",
        p.connections(),
        p.dropped_updates()
    );
}
