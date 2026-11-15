//! Advertise on a live controller, if one is named.
//!
//! Set `GATTMIRROR_HCI` to the `host:port` of the emulator's H4-over-TCP socket. Without it the
//! tests pass vacuously and say so. Each connection is its own virtual controller, so the
//! advertisement vanishes when the test ends.

use std::net::TcpStream;
use std::time::Duration;

use le_host::{AdvParams, Advertisement, Controller, ServiceUuid};

/// Not a real device name; the test name every live run advertises.
const TEST_NAME: &str = "GM-TEST-E1";

fn connect() -> Option<Controller<TcpStream>> {
    let Ok(addr) = std::env::var("GATTMIRROR_HCI") else {
        eprintln!("GATTMIRROR_HCI not set: live advertising test skipped");
        return None;
    };
    let s = TcpStream::connect(addr).expect("connect to the controller");
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    Some(Controller::new(s))
}

#[test]
fn init_and_advertise_a_fixed_name() {
    let Some(mut c) = connect() else { return };
    let info = c.init().expect("controller init");
    assert!(info.le_acl_len > 0 && info.le_acl_count > 0);
    let ad = Advertisement::new(TEST_NAME, Some(ServiceUuid::Uuid16(0xFFF0))).unwrap();
    c.start_advertising(&ad, &AdvParams::default())
        .expect("advertising accepted");
    c.stop_advertising().expect("advertising disabled");
}

#[test]
fn a_command_before_reset_is_not_reset() {
    let Some(mut c) = connect() else { return };
    let e = c
        .command(hci_wire::opcodes::LE_READ_BUFFER_SIZE, &[])
        .unwrap_err();
    assert!(matches!(e, le_host::Error::NotReset), "{e}");
}
