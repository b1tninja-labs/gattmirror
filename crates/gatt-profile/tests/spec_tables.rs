//! Values pinned to the Bluetooth Core Specification 7.0, Vol 3 Parts B, F and G. Identifiers only.

use gatt_profile::{BASE_UUID, IssueKind, Properties, Uuid, cccd, parse, type_uuids};

#[test]
fn base_uuid_and_short_expansion_follow_part_b_2_5_1() {
    // 128_bit_value = 16_bit_value * 2^96 + Bluetooth_Base_UUID
    assert_eq!(
        BASE_UUID.as_u128(),
        0x0000_0000_0000_1000_8000_0080_5f9b_34fb
    );
    assert_eq!(
        Uuid::from_u16(0x2902).as_u128(),
        0x0000_2902_0000_1000_8000_0080_5f9b_34fb
    );
    assert_eq!(
        "00002902-0000-1000-8000-00805f9b34fb".parse::<Uuid>(),
        Ok(Uuid::from_u16(0x2902))
    );
}

#[test]
fn property_bits_follow_table_3_5() {
    assert_eq!(Properties::BROADCAST, 0x01);
    assert_eq!(Properties::READ, 0x02);
    assert_eq!(Properties::WRITE_NO_RESPONSE, 0x04);
    assert_eq!(Properties::WRITE, 0x08);
    assert_eq!(Properties::NOTIFY, 0x10);
    assert_eq!(Properties::INDICATE, 0x20);
    assert_eq!(Properties::SIGNED_WRITE, 0x40);
    assert_eq!(Properties::EXTENDED, 0x80);
}

#[test]
fn attribute_type_and_descriptor_uuids_follow_part_g_section_3() {
    let pairs = [
        (type_uuids::PRIMARY_SERVICE, 0x2800),
        (type_uuids::SECONDARY_SERVICE, 0x2801),
        (type_uuids::INCLUDE, 0x2802),
        (type_uuids::CHARACTERISTIC, 0x2803),
        (type_uuids::CHARACTERISTIC_EXTENDED_PROPERTIES, 0x2900),
        (type_uuids::CHARACTERISTIC_USER_DESCRIPTION, 0x2901),
        (type_uuids::CLIENT_CHARACTERISTIC_CONFIGURATION, 0x2902),
        (type_uuids::SERVER_CHARACTERISTIC_CONFIGURATION, 0x2903),
        (type_uuids::CHARACTERISTIC_PRESENTATION_FORMAT, 0x2904),
    ];
    for (uuid, short) in pairs {
        assert_eq!(uuid, Uuid::from_u16(short));
    }
}

#[test]
fn cccd_bits_follow_table_3_11() {
    assert_eq!(cccd::NOTIFICATION, 1);
    assert_eq!(cccd::INDICATION, 2);
}

#[test]
fn broadcast_needs_a_server_configuration_descriptor() {
    let bad = parse("service 1800\ncharacteristic 2a00 broadcast,read\n").unwrap();
    assert!(
        bad.lint()
            .iter()
            .any(|i| i.kind == IssueKind::BroadcastWithoutSccd)
    );
    let good =
        parse("service 1800\ncharacteristic 2a00 broadcast,read\n  descriptor 2903\n").unwrap();
    assert_eq!(good.lint().len(), 0);
}

#[test]
fn declarations_carry_two_octets_for_a_16_bit_uuid_and_sixteen_otherwise() {
    // Part G 2.5.4.
    let p = parse("service 1800\ncharacteristic 2a00 read\n").unwrap();
    assert_eq!(p.attributes().unwrap()[0].value.len(), 2);
    let q = parse("service 00001234-0000-1000-8001-00805f9b34fb\n").unwrap();
    assert_eq!(q.attributes().unwrap()[0].value.len(), 16);
}
