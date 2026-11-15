//! Fixtures, lint, serial detection, diff, versioning and annotated output.

use std::fs;
use std::path::Path;

use gatt_profile::{
    Difference, FORMAT_VERSION, IssueKind, ParseErrorKind, Profile, Severity, Side, Uuid, parse,
    write, write_annotated,
};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    fs::read_to_string(path).unwrap()
}

fn fixtures() -> Vec<(String, Profile)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "gatt") {
            let text = fs::read_to_string(&p).unwrap();
            let prof = parse(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            out.push((p.file_name().unwrap().to_string_lossy().into_owned(), prof));
        }
    }
    assert!(out.len() >= 5);
    out
}

fn p(src: &str) -> Profile {
    parse(src).unwrap()
}

#[test]
fn every_fixture_round_trips_lints_without_errors_and_diffs_clean() {
    for (name, prof) in fixtures() {
        assert_eq!(parse(&write(&prof)).unwrap(), prof, "{name}");
        assert_eq!(parse(&write_annotated(&prof)).unwrap(), prof, "{name}");
        let errors: Vec<_> = prof
            .lint()
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.describe(&prof))
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        assert!(prof.diff(&prof).is_empty(), "{name}");
        assert!(prof.attributes().is_ok(), "{name}");
    }
}

#[test]
fn every_serial_fixture_names_its_roles() {
    for (name, prof) in fixtures() {
        if name == "gap_gatt_baseline.gatt" {
            assert!(prof.serial().is_none());
        } else {
            assert!(prof.serial().is_some(), "{name}");
        }
    }
}

#[test]
fn annotated_output_names_known_uuids() {
    let prof = p("service 1800\ncharacteristic 2a00 read\nservice fff0\n");
    let text = write_annotated(&prof);
    assert!(text.contains("# GAP\nservice 1800"));
    assert!(text.contains("  # Device Name\n"));
    assert!(!text.contains("# fff0"));
}

#[test]
fn serial_roles_are_chosen_by_property() {
    let prof = p(&fixture("fff0_serial.gatt"));
    let s = prof.serial().unwrap();
    assert_eq!(s.service, Uuid::from_u16(0xfff0));
    assert_eq!(s.notify, Uuid::from_u16(0xfff1));
    assert_eq!(s.write, Uuid::from_u16(0xfff2));
    assert!(s.write_without_response);
    assert!(!s.indicate_only);
    // swap the numbers: roles follow the properties
    let swapped = p("service fff0\ncharacteristic fff1 write\ncharacteristic fff2 indicate\n");
    let s = swapped.serial().unwrap();
    assert_eq!(
        (s.write, s.notify),
        (Uuid::from_u16(0xfff1), Uuid::from_u16(0xfff2))
    );
    assert!(!s.write_without_response);
    assert!(s.indicate_only);
}

#[test]
fn a_single_characteristic_can_hold_both_roles() {
    let prof = p(&fixture("hm10_ffe0.gatt"));
    let s = prof.serial().unwrap();
    assert_eq!(s.write, s.notify);
    assert_eq!(s.write, Uuid::from_u16(0xffe1));
}

fn kinds(src: &str) -> Vec<(Severity, IssueKind)> {
    p(src)
        .lint()
        .into_iter()
        .map(|i| (i.severity, i.kind))
        .collect()
}

#[test]
fn lint_rules_fire() {
    use IssueKind::*;
    use Severity::*;
    let u = Uuid::from_u16;
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 notify\n"),
        vec![(Error, NotifyWithoutCccd)]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 read\n  descriptor 2902\n"),
        vec![(Error, CccdWithoutNotify)]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 indicate\n  descriptor 2902 value=000000\n"),
        vec![(Error, CccdBadValue(3))]
    );
    assert_eq!(
        kinds(
            "service 1800\ncharacteristic 2a00 read\n  descriptor 2901 value=41\n  descriptor 2901 value=42\n"
        ),
        vec![(Error, DuplicateDescriptor(u(0x2901)))]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 read\ncharacteristic 2a00 read\n"),
        vec![(Warning, DuplicateCharacteristic(u(0x2a00)))]
    );
    assert_eq!(
        kinds("service 1800\nservice 1800\n"),
        vec![
            (Warning, DuplicateService(u(0x1800))),
            (Warning, EmptyService),
            (Warning, EmptyService)
        ]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 none\n"),
        vec![(Warning, NoProperties)]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 write value=00\n"),
        vec![(Warning, ValueWithoutRead)]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 read,extended\n"),
        vec![(Warning, ExtendedWithoutDescriptor)]
    );
    assert_eq!(
        kinds("service 1800\ncharacteristic 2a00 read\n  descriptor 2900\n"),
        vec![(Warning, ExtendedDescriptorWithoutBit)]
    );
    assert_eq!(
        kinds("adv-service fff0\nadv-service fff0\nservice 1800\ncharacteristic 2a00 read\n"),
        vec![
            (Warning, DuplicateAdvertisedService(u(0xfff0))),
            (Warning, AdvertisedServiceNotInTable(u(0xfff0))),
            (Warning, AdvertisedServiceNotInTable(u(0xfff0))),
        ]
    );
}

#[test]
fn lint_messages_say_where() {
    let prof = p("service fff0\ncharacteristic fff1 notify\n");
    let issue = &prof.lint()[0];
    assert_eq!(
        issue.describe(&prof),
        "error: service fff0, characteristic fff1: notify or indicate is set but there is no 2902 descriptor"
    );
}

#[test]
fn diff_finds_each_kind_of_difference() {
    let a = p(&fixture("fff0_serial.gatt"));
    assert_eq!(a.diff(&a.clone()), vec![]);

    let mut b = a.clone();
    b.device_name = Some("Other".into());
    b.services[1].characteristics[1].properties.0 = 0x08;
    b.services[1].characteristics[0].descriptors[0].value = vec![1, 0];
    b.services[0].characteristics[0].value = Some(b"x".to_vec());
    b.services.push(p("service 180f\n").services.remove(0));
    let extra =
        p("service 1800\ncharacteristic fff3 read\n").services[0].characteristics[0].clone();
    b.services[1].characteristics.push(extra);

    let d = a.diff(&b);
    let f0 = Uuid::from_u16(0xfff0);
    assert!(d.contains(&Difference::DeviceName(
        a.device_name.clone(),
        b.device_name.clone()
    )));
    assert!(d.contains(&Difference::Properties {
        service: f0,
        uuid: Uuid::from_u16(0xfff2),
        left: 0x0c,
        right: 0x08
    }));
    assert!(d.contains(&Difference::DescriptorValue {
        service: f0,
        characteristic: Uuid::from_u16(0xfff1),
        uuid: Uuid::from_u16(0x2902)
    }));
    assert!(d.contains(&Difference::Value {
        service: Uuid::from_u16(0x1800),
        uuid: Uuid::from_u16(0x2a00)
    }));
    assert!(d.contains(&Difference::Service(Uuid::from_u16(0x180f), Side::Right)));
    assert!(d.contains(&Difference::Characteristic {
        service: f0,
        uuid: Uuid::from_u16(0xfff3),
        only_in: Side::Right
    }));
    assert_eq!(d.len(), 6);
    // the other way round flips the sides
    assert!(
        b.diff(&a)
            .contains(&Difference::Service(Uuid::from_u16(0x180f), Side::Left))
    );
}

#[test]
fn diff_ignores_order_and_missing_live_values() {
    let a = p(
        "service 1800\ncharacteristic 2a00 read value=41\ncharacteristic 2a01 read\nservice fff0\n",
    );
    let b = p(
        "service fff0\nservice 1800\ncharacteristic 2a01 read value=00\ncharacteristic 2a00 read\n",
    );
    assert_eq!(a.diff(&b), vec![]);
}

#[test]
fn diff_pairs_repeated_uuids_by_occurrence() {
    let a = p("service 1800\ncharacteristic 2a00 read\ncharacteristic 2a00 write\n");
    let b = p("service 1800\ncharacteristic 2a00 read\ncharacteristic 2a00 read\n");
    assert_eq!(a.diff(&b).len(), 1);
    assert!(matches!(a.diff(&b)[0], Difference::Properties { .. }));
}

#[test]
fn diff_display_is_readable() {
    let a = p("service 1800\n");
    let b = p("service 1800 secondary\n");
    assert_eq!(
        a.diff(&b)[0].to_string(),
        "service 1800 differs in primary/secondary"
    );
}

#[test]
fn version_header_rules() {
    assert!(write(&Profile::default()).starts_with(&format!("gatt-profile {FORMAT_VERSION}\n")));
    assert!(parse("gatt-profile 1\n").is_ok());
    // a missing header is version 1
    assert!(parse("service 1800\n").is_ok());
    // a header may follow comments and blank lines
    assert!(parse("# c\n\n  gatt-profile 1\n").is_ok());
    assert_eq!(
        parse("gatt-profile 2\n").unwrap_err().kind,
        ParseErrorKind::UnsupportedVersion(2)
    );
}

#[test]
fn extensions_are_ignored_but_unknown_names_are_not() {
    let prof = p(
        "gatt-profile 1\nx-note anything at all\nservice 1800\ncharacteristic 2a00 read x-seen=3 value=41\n",
    );
    assert_eq!(prof.services[0].characteristics[0].value, Some(vec![0x41]));
    assert!(parse("note anything\n").is_err());
    assert!(parse("service 1800\ncharacteristic 2a00 read seen=3\n").is_err());
}

#[test]
fn diff_skips_a_descriptor_value_one_side_lacks() {
    let a = p("service 1800\ncharacteristic 2a00 read\n  descriptor 2901 value=41\n");
    let b = p("service 1800\ncharacteristic 2a00 read\n  descriptor 2901\n");
    assert_eq!(a.diff(&b), vec![]);
    let c = p("service 1800\ncharacteristic 2a00 read\n  descriptor 2901 value=42\n");
    assert_eq!(a.diff(&c).len(), 1);
}
