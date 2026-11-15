//! Fixture, round-trip, attribute-layout and malformed-input tests.

use gatt_profile::{
    AttributeRole, ParseErrorKind, Profile, Properties, ServiceKind, Uuid, parse, type_uuids, write,
};

const FFF0: &str = include_str!("../fixtures/fff0_serial.gatt");
const NUART: &str = include_str!("../fixtures/nordic_uart.gatt");

#[test]
fn fff0_fixture_has_the_expected_shape() {
    let p = parse(FFF0).unwrap();
    assert_eq!(p.device_name.as_deref(), Some("Example Serial Adapter"));
    assert_eq!(p.advertised_services, vec![Uuid::from_u16(0xfff0)]);
    let svc = &p.services[1];
    assert_eq!(svc.uuid, Uuid::from_u16(0xfff0));
    assert_eq!(svc.kind, ServiceKind::Primary);
    let notify = &svc.characteristics[0];
    assert_eq!(notify.uuid, Uuid::from_u16(0xfff1));
    assert_eq!(notify.properties, Properties(Properties::NOTIFY));
    assert_eq!(notify.descriptors[0].uuid, Uuid::from_u16(0x2902));
    assert_eq!(notify.descriptors[0].value, vec![0, 0]);
    let write_ch = &svc.characteristics[1];
    assert!(write_ch.properties.contains(Properties::WRITE));
    assert!(write_ch.properties.contains(Properties::WRITE_NO_RESPONSE));
    assert!(!write_ch.properties.contains(Properties::NOTIFY));
}

#[test]
fn fixtures_round_trip_and_the_writer_is_idempotent() {
    for src in [FFF0, NUART] {
        let p = parse(src).unwrap();
        let text = write(&p);
        assert_eq!(parse(&text).unwrap(), p);
        assert_eq!(write(&parse(&text).unwrap()), text);
    }
}

#[test]
fn long_uuids_and_secondary_services_survive() {
    let p = parse(NUART).unwrap();
    assert_eq!(p.services[0].uuid.as_u16(), None);
    assert_eq!(p.services[1].kind, ServiceKind::Secondary);
    assert!(write(&p).contains("6e400001-b5a3-f393-e0a9-e50e24dcca9e"));
}

#[test]
fn attributes_have_sequential_handles_and_encoded_declarations() {
    let attrs = parse(FFF0).unwrap().attributes().unwrap();
    let handles: Vec<u16> = attrs.iter().map(|a| a.handle).collect();
    let expected: Vec<u16> = (1..=u16::try_from(attrs.len()).unwrap()).collect();
    assert_eq!(handles, expected);
    // service 1800: decl, char decl, value; then fff0: decl, fff1 decl, value, cccd, fff2 decl, value
    assert_eq!(attrs.len(), 3 + 6);
    assert_eq!(attrs[0].role, AttributeRole::Service { end_handle: 3 });
    assert_eq!(attrs[3].role, AttributeRole::Service { end_handle: 9 });
    assert_eq!(attrs[3].attr_type, type_uuids::PRIMARY_SERVICE);
    assert_eq!(attrs[3].value, vec![0xf0, 0xff]);
    // fff1 declaration: properties notify, value handle 6, uuid fff1 little-endian
    assert_eq!(attrs[4].attr_type, type_uuids::CHARACTERISTIC);
    assert_eq!(attrs[4].value, vec![0x10, 0x06, 0x00, 0xf1, 0xff]);
    assert_eq!(attrs[6].attr_type, Uuid::from_u16(0x2902));
    assert_eq!(attrs[6].value, vec![0, 0]);
    // the 1800 name carries its static value; fff2 does not
    assert_eq!(
        attrs[2].role,
        AttributeRole::Value {
            properties: 0x02,
            has_static_value: true
        }
    );
    assert_eq!(
        attrs[8].role,
        AttributeRole::Value {
            properties: 0x0c,
            has_static_value: false
        }
    );
}

#[test]
fn a_long_uuid_declaration_carries_sixteen_bytes() {
    let attrs = parse(NUART).unwrap().attributes().unwrap();
    assert_eq!(attrs[0].value.len(), 16);
    assert_eq!(attrs[1].value.len(), 1 + 2 + 16);
}

#[test]
fn too_many_attributes_is_an_error() {
    let mut p = parse("service 1800\ncharacteristic 2a00 read\n").unwrap();
    let ch = p.services[0].characteristics[0].clone();
    p.services[0].characteristics = vec![ch; 33_000];
    assert!(p.attributes().is_err());
}

fn err(src: &str) -> (usize, ParseErrorKind) {
    let e = parse(src).unwrap_err();
    (e.line, e.kind)
}

#[test]
fn errors_carry_line_numbers() {
    assert_eq!(
        err("# c\n\nservice 1800\n  frobnicate x\n"),
        (4, ParseErrorKind::UnknownDirective("frobnicate".into()))
    );
    assert_eq!(
        err("characteristic 2a00 read\n"),
        (1, ParseErrorKind::NoService)
    );
    assert_eq!(
        err("service 1800\ndescriptor 2902\n"),
        (2, ParseErrorKind::NoCharacteristic)
    );
    assert_eq!(
        err("service 1800\ncharacteristic 2a00\n"),
        (2, ParseErrorKind::MissingArgument("properties"))
    );
    assert_eq!(
        err("service 1800\ncharacteristic 2a00 shout\n"),
        (2, ParseErrorKind::BadProperty("shout".into()))
    );
    assert_eq!(
        err("service 1800\ncharacteristic 2a00 read value=abc\n"),
        (2, ParseErrorKind::BadHex("abc".into()))
    );
    assert_eq!(
        err("service 1800\ncharacteristic 2a00 read value=00 value=11\n"),
        (2, ParseErrorKind::BadOption("value=11".into()))
    );
    assert_eq!(
        err("service 1800 tertiary\n"),
        (1, ParseErrorKind::BadServiceKind("tertiary".into()))
    );
    assert_eq!(
        err("service 1800 primary extra\n"),
        (1, ParseErrorKind::TrailingText("extra".into()))
    );
    assert_eq!(
        err("device-name\n"),
        (1, ParseErrorKind::EmptyText("device-name"))
    );
    assert_eq!(
        err("adv-name a\nadv-name b\n"),
        (2, ParseErrorKind::Duplicate("adv-name"))
    );
    assert_eq!(
        err("service\n"),
        (1, ParseErrorKind::MissingArgument("UUID"))
    );
    assert!(matches!(
        err("service zzzz\n"),
        (1, ParseErrorKind::BadUuid(_))
    ));
}

#[test]
fn error_text_names_the_line() {
    let e = parse("service 1800\nbogus\n").unwrap_err();
    assert_eq!(e.to_string(), "line 2: unknown directive `bogus`");
}

#[test]
fn raw_property_bytes_and_none_parse() {
    let p = parse("service 1800\ncharacteristic 2a00 0x12\ncharacteristic 2a01 none\n").unwrap();
    assert_eq!(
        p.services[0].characteristics[0].properties,
        Properties(0x12)
    );
    assert_eq!(p.services[0].characteristics[1].properties, Properties(0));
    assert!(write(&p).contains("read,notify"));
    assert!(write(&p).contains("2a01 none"));
}

#[test]
fn empty_input_is_an_empty_profile() {
    assert_eq!(parse("").unwrap(), Profile::default());
    assert_eq!(
        write(&Profile::default()),
        "gatt-profile 1
"
    );
}
