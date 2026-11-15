//! Display-only names for well-known 16-bit Bluetooth UUIDs, so dumps are readable.
//!
//! Names are taken verbatim from the Bluetooth SIG's public Assigned Numbers YAML files
//! (`service_uuids.yaml`, `characteristic_uuids.yaml`, `descriptors.yaml`, `declarations.yaml` under
//! <https://bitbucket.org/bluetooth-SIG/public/src/main/assigned_numbers/uuids/>). Only a small
//! subset is kept; a UUID that is missing here is simply unnamed. The files carry a SIG copyright
//! notice and no licence grant; the table copies identifiers only, and the owner should confirm that
//! is acceptable before this crate is published.

use crate::uuid::Uuid;

const SERVICES: &[(u16, &str)] = &[
    (0x1800, "GAP"),
    (0x1801, "GATT"),
    (0x1802, "Immediate Alert"),
    (0x1803, "Link Loss"),
    (0x1804, "Tx Power"),
    (0x1805, "Current Time"),
    (0x180a, "Device Information"),
    (0x180d, "Heart Rate"),
    (0x180f, "Battery"),
    (0x1812, "Human Interface Device"),
    (0x1816, "Cycling Speed and Cadence"),
    (0x181c, "User Data"),
];

const CHARACTERISTICS: &[(u16, &str)] = &[
    (0x2a00, "Device Name"),
    (0x2a01, "Appearance"),
    (0x2a02, "Peripheral Privacy Flag"),
    (0x2a03, "Reconnection Address"),
    (0x2a04, "Peripheral Preferred Connection Parameters"),
    (0x2a05, "Service Changed"),
    (0x2a19, "Battery Level"),
    (0x2a23, "System ID"),
    (0x2a24, "Model Number String"),
    (0x2a25, "Serial Number String"),
    (0x2a26, "Firmware Revision String"),
    (0x2a27, "Hardware Revision String"),
    (0x2a28, "Software Revision String"),
    (0x2a29, "Manufacturer Name String"),
    (
        0x2a2a,
        "IEEE 11073-20601 Regulatory Certification Data List",
    ),
    (0x2a37, "Heart Rate Measurement"),
    (0x2a38, "Body Sensor Location"),
    (0x2a50, "PnP ID"),
    (0x2aa6, "Central Address Resolution"),
    (0x2b29, "Client Supported Features"),
    (0x2b2a, "Database Hash"),
];

const DESCRIPTORS: &[(u16, &str)] = &[
    (0x2900, "Characteristic Extended Properties"),
    (0x2901, "Characteristic User Description"),
    (0x2902, "Client Characteristic Configuration"),
    (0x2903, "Server Characteristic Configuration"),
    (0x2904, "Characteristic Presentation Format"),
    (0x2905, "Characteristic Aggregate Format"),
    (0x2906, "Valid Range"),
    (0x2907, "External Report Reference"),
    (0x2908, "Report Reference"),
    (0x2909, "Number of Digitals"),
    (0x290a, "Value Trigger Setting"),
    (0x290b, "Environmental Sensing Configuration"),
    (0x290c, "Environmental Sensing Measurement"),
    (0x290d, "Environmental Sensing Trigger Setting"),
    (0x290e, "Time Trigger Setting"),
    (0x290f, "Complete BR-EDR Transport Block Data"),
    (0x2910, "Observation Schedule"),
    (0x2911, "Valid Range and Accuracy"),
    (0x2912, "Measurement Description"),
    (0x2913, "Manufacturer Limits"),
    (0x2914, "Process Tolerances"),
    (0x2915, "IMD Trigger Setting"),
    (0x2916, "Cooking Sensor Info"),
    (0x2917, "Cooking Trigger Setting"),
];

const DECLARATIONS: &[(u16, &str)] = &[
    (0x2800, "Primary Service"),
    (0x2801, "Secondary Service"),
    (0x2802, "Include"),
    (0x2803, "Characteristic"),
];

fn lookup(table: &[(u16, &'static str)], uuid: Uuid) -> Option<&'static str> {
    let short = uuid.as_u16()?;
    table
        .binary_search_by_key(&short, |(k, _)| *k)
        .ok()
        .map(|i| table[i].1)
}

/// The SIG name of a service UUID, if it is in the table.
#[must_use]
pub fn service_name(uuid: Uuid) -> Option<&'static str> {
    lookup(SERVICES, uuid)
}

/// The SIG name of a characteristic UUID, if it is in the table.
#[must_use]
pub fn characteristic_name(uuid: Uuid) -> Option<&'static str> {
    lookup(CHARACTERISTICS, uuid)
}

/// The SIG name of a descriptor UUID, if it is in the table.
#[must_use]
pub fn descriptor_name(uuid: Uuid) -> Option<&'static str> {
    lookup(DESCRIPTORS, uuid)
}

/// The SIG name of an attribute-declaration UUID (`2800`..`2803`), if it is in the table.
#[must_use]
pub fn declaration_name(uuid: Uuid) -> Option<&'static str> {
    lookup(DECLARATIONS, uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_unique() {
        for t in [SERVICES, CHARACTERISTICS, DESCRIPTORS, DECLARATIONS] {
            assert!(t.windows(2).all(|w| w[0].0 < w[1].0));
        }
    }

    #[test]
    fn well_known_names_resolve() {
        assert_eq!(service_name(Uuid::from_u16(0x1800)), Some("GAP"));
        assert_eq!(
            characteristic_name(Uuid::from_u16(0x2a00)),
            Some("Device Name")
        );
        assert_eq!(
            descriptor_name(Uuid::from_u16(0x2902)),
            Some("Client Characteristic Configuration")
        );
        assert_eq!(
            declaration_name(Uuid::from_u16(0x2803)),
            Some("Characteristic")
        );
    }

    #[test]
    fn unknown_and_long_uuids_have_no_name() {
        assert_eq!(service_name(Uuid::from_u16(0xfff0)), None);
        let long = Uuid::from_u128(0x6e40_0001_b5a3_f393_e0a9_e50e_24dc_ca9e);
        assert_eq!(service_name(long), None);
    }
}
