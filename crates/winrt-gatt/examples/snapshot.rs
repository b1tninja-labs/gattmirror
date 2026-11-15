//! Discover a peripheral and write its GATT table in the `gatt-profile` text format.
//!
//! Usage: `snapshot <name-substring | address> [output-file]` (text goes to stdout without a file).
//! This only reads the table and the GAP Device Name; it writes nothing to the peripheral.
//! Compile-checked only: it has never been run against a device.

#[cfg(windows)]
fn main() {
    if let Err(e) = run() {
        eprintln!("snapshot: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("snapshot: this example needs Windows");
    std::process::exit(2);
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Duration;
    use winrt_gatt::snapshot::{SnapshotOptions, to_profile};
    use winrt_gatt::{ConnectOptions, ScanFilter, Uuid, WinrtClient, address, scan};

    let mut args = std::env::args().skip(1);
    let target = args
        .next()
        .ok_or("usage: snapshot <name-substring | address> [output-file]")?;
    let out = args.next();

    let filter = match address::parse_address(&target) {
        Ok(a) => ScanFilter::by_address(a),
        Err(_) => ScanFilter::by_name(target),
    };
    let found = scan(&filter, Duration::from_secs(30))?;
    eprintln!("found {}", address::format_address(found.address));

    let (client, _notifications) = WinrtClient::connect(found.address, &ConnectOptions::default())?;
    let table = client.table();

    // Read the GAP Device Name (2A00) if the table offers it readable; a failure only costs the field.
    let device_name = table
        .find(Some(Uuid::from_u16(0x1800)), Uuid::from_u16(0x2A00))
        .filter(|c| c.properties.has(winrt_gatt::Properties::READ))
        .and_then(|c| client.read(c.handle).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned());

    let opts = SnapshotOptions {
        device_name,
        advertised_name: (!found.name.is_empty()).then(|| found.name.clone()),
        advertised_services: found.services.clone(),
        ..SnapshotOptions::default()
    };
    let (profile, _map) = to_profile(table, &opts);
    let text = gatt_profile::write(&profile);
    match out {
        Some(path) => std::fs::write(path, text)?,
        None => print!("{text}"),
    }
    Ok(())
}
