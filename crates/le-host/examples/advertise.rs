//! Advertise a name on the controller at `host:port` and hold the connection open.
//!
//! `cargo run -p le-host --example advertise -- 127.0.0.1:PORT [NAME] [SECONDS]`
//!
//! The advertisement lives as long as the TCP connection does.

use std::net::TcpStream;
use std::time::Duration;

use le_host::{AdvParams, Advertisement, Controller, ServiceUuid};

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args
        .next()
        .expect("usage: advertise HOST:PORT [NAME] [SECONDS]");
    let name = args.next().unwrap_or_else(|| "GM-TEST-E1".to_owned());
    let secs: u64 = args.next().map_or(120, |s| s.parse().expect("seconds"));
    let stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut c = Controller::new(stream);
    let info = c.init().expect("init");
    println!(
        "controller up: LE ACL {} x {}",
        info.le_acl_len, info.le_acl_count
    );
    let ad = Advertisement::new(&name, Some(ServiceUuid::Uuid16(0xFFF0))).expect("advertisement");
    c.start_advertising(&ad, &AdvParams::default())
        .expect("start advertising");
    println!("advertising {name:?} for {secs} s");
    std::thread::sleep(Duration::from_secs(secs));
    c.stop_advertising().expect("stop");
}
