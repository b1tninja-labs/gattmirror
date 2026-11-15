use std::fmt::Write as _;

use crate::assigned;
use crate::model::{Profile, Properties, ServiceKind};
use crate::parse::FORMAT_VERSION;

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
    emit(profile, false)
}

/// Like [`write()`], with a full-line `# Name` comment above each service, characteristic and
/// descriptor whose UUID is in the [`crate::assigned`] table. The comments are ignored on parsing, so
/// the output reads back to the same profile.
#[must_use]
pub fn write_annotated(profile: &Profile) -> String {
    emit(profile, true)
}

fn note(out: &mut String, indent: &str, name: Option<&str>, on: bool) {
    if let (true, Some(n)) = (on, name) {
        let _ = writeln!(out, "{indent}# {n}");
    }
}

fn emit(profile: &Profile, annotate: bool) -> String {
    let mut out = format!(
        "gatt-profile {FORMAT_VERSION}
"
    );
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
        note(&mut out, "", assigned::service_name(s.uuid), annotate);
        let _ = writeln!(out, "service {} {kind}", s.uuid);
        for c in &s.characteristics {
            note(
                &mut out,
                "  ",
                assigned::characteristic_name(c.uuid),
                annotate,
            );
            let _ = write!(out, "  characteristic {} {}", c.uuid, props(c.properties));
            if let Some(v) = &c.value {
                let _ = write!(out, " value={}", hex(v));
            }
            out.push('\n');
            for d in &c.descriptors {
                note(
                    &mut out,
                    "    ",
                    assigned::descriptor_name(d.uuid),
                    annotate,
                );
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
