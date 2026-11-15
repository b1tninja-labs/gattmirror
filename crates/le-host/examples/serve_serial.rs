//! Serve the serial shim to a guest: a stub adapter (`gattmirror::sim`) behind `SerialBackend`.
//!
//! `cargo run -p le-host --example serve_serial -- 127.0.0.1:PORT [NAME] [SECONDS]`
//!
//! A guest write of `ATI\r` to the write characteristic goes to the stub, whose reply comes back
//! as notifications cut to the negotiated MTU. Every ATT frame is printed as `ATT > hex` (from the
//! guest) or `ATT < hex` (to the guest), a transcript that holds no addresses.

use std::net::TcpStream;
use std::time::Duration;

use gattmirror::serial::{FFF0, SerialBackend, WriteBuffering};
use gattmirror::sim::Session;
use gattmirror::{Mirror, NotifyPolicy};
use le_host::{AdvParams, Advertisement, Controller, Direction, Peripheral, ServiceUuid};
use serial_stream::ByteStream;
use serial_stream::mem;

/// The far end of the stream: split commands at `\r` and answer each with the stub's reply.
fn adapter(mut end: mem::MemStream) {
    let rx = end.receiver().expect("receiver");
    std::thread::spawn(move || {
        let mut session = Session::default();
        let mut line = Vec::new();
        while let Ok(chunk) = rx.recv() {
            for b in chunk {
                if b == b'\r' {
                    let reply = session.respond(&line);
                    line.clear();
                    if end.write(&reply).is_err() {
                        return;
                    }
                } else {
                    line.push(b);
                }
            }
        }
    });
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for x in b {
        write!(s, "{x:02x}").unwrap();
    }
    s
}

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args
        .next()
        .expect("usage: serve_serial HOST:PORT [NAME] [SECONDS]");
    let name = args.next().unwrap_or_else(|| "GM-TEST-E3".to_owned());
    let secs: u64 = args.next().map_or(120, |s| s.parse().expect("seconds"));

    let mut profile = gatt_profile::parse(FFF0).expect("profile");
    gattmirror::apply_name(&mut profile, &name);
    let (ours, theirs) = mem::pair();
    adapter(theirs);
    let backend =
        SerialBackend::new(profile, ours, WriteBuffering::UntilByte(b'\r')).expect("serial");
    let mut mirror = Mirror::new(backend, 247).expect("mirror");
    mirror.set_policy(NotifyPolicy::Always);
    let (server, mut pump) = mirror.into_parts();

    let stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut ctl = Controller::new(stream);
    let info = ctl.init().expect("init");
    let ad = Advertisement::new(&name, Some(ServiceUuid::Uuid16(0xFFF0))).expect("advertisement");
    let (mut p, handle) = Peripheral::new(ctl, info, ad, AdvParams::default(), server);
    p.set_source(Box::new(move |srv| pump.drain(srv)));
    p.set_link_hook(Box::new(|e, srv| {
        println!("link {e:?}");
        srv.backend().reset_connection();
    }));
    p.set_trace(Box::new(|d, cid, b| {
        if cid == le_host::CID_ATT {
            let arrow = if d == Direction::In { '>' } else { '<' };
            println!("ATT {arrow} {}", hex(b));
        }
    }));
    let h = handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        h.stop();
    });
    println!("serving {name:?} for {secs} s");
    let r = p.serve();
    println!("done: {r:?}; connections {}", p.connections());
}
