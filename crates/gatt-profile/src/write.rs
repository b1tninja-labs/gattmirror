use std::fmt::Write as _;

use crate::model::{Profile, Properties, ServiceKind};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn props(p: Properties) -> String {
    let names: Vec<&str> = Properties::NAMES
        .iter()
        .filter(|(_, bit)| p.0 & bit != 0)
        .map(|(n, _)| *n)
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(",")
    }
}

/// Emit the canonical text form of a profile; [`crate::parse`] reads it back to an equal profile.
///
/// Names are written to end of line, so a name containing a line break, or with leading or trailing
/// whitespace, does not survive a round trip; a parsed profile never has one.
#[must_use]
pub fn write(profile: &Profile) -> String {
    let mut out = String::new();
    if let Some(n) = &profile.device_name {
        let _ = writeln!(out, "device-name {n}");
    }
    if let Some(n) = &profile.advertised_name {
        let _ = writeln!(out, "adv-name {n}");
    }
    for u in &profile.advertised_services {
        let _ = writeln!(out, "adv-service {u}");
    }
    for s in &profile.services {
        if !out.is_empty() {
            out.push('\n');
        }
        let kind = match s.kind {
            ServiceKind::Primary => "primary",
            ServiceKind::Secondary => "secondary",
        };
        let _ = writeln!(out, "service {} {kind}", s.uuid);
        for c in &s.characteristics {
            let _ = write!(out, "  characteristic {} {}", c.uuid, props(c.properties));
            if let Some(v) = &c.value {
                let _ = write!(out, " value={}", hex(v));
            }
            out.push('\n');
            for d in &c.descriptors {
                let _ = write!(out, "    descriptor {}", d.uuid);
                if !d.value.is_empty() {
                    let _ = write!(out, " value={}", hex(&d.value));
                }
                out.push('\n');
            }
        }
    }
    out
}
