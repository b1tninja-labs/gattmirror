//! Serve on a live controller, if one is named (`GATTMIRROR_HCI=host:port`). Without it the test
//! passes vacuously and says so. No peer connects: this checks that the loop runs against the real
//! controller's event stream and shuts down cleanly.

use std::net::TcpStream;
use std::time::Duration;

use att_server::{Database, NoBackend, Props, Server, Uuid, Value};
use le_host::{AdvParams, Advertisement, Controller, Peripheral};

#[test]
fn serve_idles_and_stops_on_a_live_controller() {
    let Ok(addr) = std::env::var("GATTMIRROR_HCI") else {
        eprintln!("GATTMIRROR_HCI not set: live serve test skipped");
        return;
    };
    let s = TcpStream::connect(addr).expect("connect to the controller");
    s.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut ctl = Controller::new(s);
    let info = ctl.init().expect("init");
    let mut b = Database::builder();
    b.service(Uuid::from_u16(0x1800)).unwrap();
    b.characteristic(
        Uuid::from_u16(0x2A00),
        Props::READ,
        Value::Static(b"t".to_vec()),
    )
    .unwrap();
    let (mut p, h) = Peripheral::new(
        ctl,
        info,
        Advertisement::new("GM-TEST-E2", None).unwrap(),
        AdvParams::default(),
        Server::new(b.build(), NoBackend, 247),
    );
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        h.stop();
    });
    p.serve().expect("serve");
    stopper.join().unwrap();
    assert_eq!(p.connections(), 0);
}
