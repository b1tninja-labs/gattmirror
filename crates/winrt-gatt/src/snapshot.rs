//! Mapping a discovered [`GattTable`] onto a [`gatt_profile::Profile`], and back to the handle to relay to.
//!
//! Platform independent: it takes plain data, so it is tested with synthetic tables. A profile carries
//! no attribute handles, so the mapping also returns a [`HandleMap`] that records, for every
//! characteristic and descriptor in the profile, the handle the *peripheral* assigned. A mirror serves the
//! profile and uses the map to relay traffic to the right real attribute.

use gatt_profile as gp;

use crate::properties::Properties;
use crate::table::GattTable;
use crate::uuid::Uuid;

/// The Client Characteristic Configuration descriptor type. The value is the 16-bit short form the
/// Windows stack reports; `[unread]` against the specification.
pub const CCCD: Uuid = Uuid::from_u16(0x2902);

/// How to build a profile from a table.
#[derive(Clone, Debug, Default)]
pub struct SnapshotOptions {
    /// Drop the Client Characteristic Configuration descriptors. A server that adds its own (as
    /// `att-server` does for notify and indicate) must not receive a second one.
    pub drop_cccd: bool,
    /// The GAP Device Name as read from the peripheral, if the caller read it.
    pub device_name: Option<String>,
    /// The advertised local name.
    pub advertised_name: Option<String>,
    /// Service UUIDs seen in advertising data.
    pub advertised_services: Vec<Uuid>,
}

/// Where a profile characteristic or descriptor came from on the peripheral.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HandleMap {
    /// `(service index, characteristic index, peripheral handle)` for every characteristic.
    pub characteristics: Vec<(usize, usize, u16)>,
    /// `(service index, characteristic index, descriptor index, peripheral handle)` for every
    /// descriptor kept in the profile (indices are positions in the *profile*).
    pub descriptors: Vec<(usize, usize, usize, u16)>,
}

impl HandleMap {
    /// The peripheral handle behind characteristic `ch` of service `svc` in the profile.
    #[must_use]
    pub fn characteristic(&self, svc: usize, ch: usize) -> Option<u16> {
        self.characteristics
            .iter()
            .find(|(s, c, _)| *s == svc && *c == ch)
            .map(|(_, _, h)| *h)
    }

    /// The peripheral handle behind descriptor `d` of characteristic `ch` of service `svc`.
    #[must_use]
    pub fn descriptor(&self, svc: usize, ch: usize, d: usize) -> Option<u16> {
        self.descriptors
            .iter()
            .find(|(s, c, i, _)| *s == svc && *c == ch && *i == d)
            .map(|(_, _, _, h)| *h)
    }
}

/// Convert a UUID. Both are 128-bit values on the Bluetooth base, so this is a copy.
#[must_use]
pub const fn profile_uuid(u: Uuid) -> gp::Uuid {
    gp::Uuid::from_u128(u.0)
}

/// Convert a profile UUID back.
#[must_use]
pub const fn table_uuid(u: gp::Uuid) -> Uuid {
    Uuid(u.as_u128())
}

/// Translate property bits. The low byte carries the declaration bits unchanged; bits above it
/// (reliable write, writable auxiliaries in the Windows flag set) have no place in a declaration byte
/// and are dropped, with the Extended bit set to say that extended properties exist.
#[must_use]
pub const fn profile_properties(p: Properties) -> gp::Properties {
    let low = (p.0 & 0xFF) as u8;
    let extended = if p.0 > 0xFF {
        gp::Properties::EXTENDED
    } else {
        0
    };
    gp::Properties(low | extended)
}

/// Build a profile and its handle map from a discovered table. Services become primary (the table does
/// not record secondary services; see the roadmap).
#[must_use]
pub fn to_profile(table: &GattTable, opts: &SnapshotOptions) -> (gp::Profile, HandleMap) {
    let mut map = HandleMap::default();
    let mut services = Vec::with_capacity(table.services.len());
    for (si, s) in table.services.iter().enumerate() {
        let mut characteristics = Vec::with_capacity(s.characteristics.len());
        for (ci, c) in s.characteristics.iter().enumerate() {
            let mut descriptors = Vec::new();
            for d in &c.descriptors {
                if opts.drop_cccd && d.uuid == CCCD {
                    continue;
                }
                map.descriptors.push((si, ci, descriptors.len(), d.handle));
                // A CCCD holds per-connection state, so its snapshot value is the off state.
                let value = if d.uuid == CCCD {
                    vec![0, 0]
                } else {
                    Vec::new()
                };
                descriptors.push(gp::Descriptor {
                    uuid: profile_uuid(d.uuid),
                    value,
                });
            }
            map.characteristics.push((si, ci, c.handle));
            characteristics.push(gp::Characteristic {
                uuid: profile_uuid(c.uuid),
                properties: profile_properties(c.properties),
                value: None,
                descriptors,
            });
        }
        services.push(gp::Service {
            uuid: profile_uuid(s.uuid),
            kind: gp::ServiceKind::Primary,
            characteristics,
        });
    }
    let profile = gp::Profile {
        device_name: opts.device_name.clone(),
        advertised_name: opts.advertised_name.clone(),
        advertised_services: opts
            .advertised_services
            .iter()
            .copied()
            .map(profile_uuid)
            .collect(),
        services,
    };
    (profile, map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::{Characteristic, Descriptor, Service};

    fn table() -> GattTable {
        GattTable {
            services: vec![
                Service {
                    handle: 1,
                    uuid: Uuid::from_u16(0x1800),
                    characteristics: vec![Characteristic {
                        handle: 3,
                        uuid: Uuid::from_u16(0x2A00),
                        properties: Properties(Properties::READ),
                        descriptors: vec![],
                    }],
                },
                Service {
                    handle: 10,
                    uuid: Uuid::from_u16(0xFFF0),
                    characteristics: vec![
                        Characteristic {
                            handle: 12,
                            uuid: Uuid::from_u16(0xFFF1),
                            properties: Properties(Properties::NOTIFY),
                            descriptors: vec![
                                Descriptor {
                                    handle: 13,
                                    uuid: Uuid::from_u16(0x2901),
                                },
                                Descriptor {
                                    handle: 14,
                                    uuid: CCCD,
                                },
                            ],
                        },
                        Characteristic {
                            handle: 16,
                            uuid: Uuid(0x6E40_0002_B5A3_F393_E0A9_E50E_24DC_CA9E),
                            properties: Properties(
                                Properties::WRITE | Properties::WRITE_WITHOUT_RESPONSE,
                            ),
                            descriptors: vec![],
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn properties_translate_bit_for_bit() {
        assert_eq!(profile_properties(Properties(0xFF)), gp::Properties(0xFF));
        assert_eq!(
            profile_properties(Properties(Properties::NOTIFY)).0,
            gp::Properties::NOTIFY
        );
        assert_eq!(
            profile_properties(Properties(Properties::WRITE_WITHOUT_RESPONSE)).0,
            gp::Properties::WRITE_NO_RESPONSE
        );
        // A bit above the declaration byte becomes the Extended bit rather than vanishing.
        assert_eq!(
            profile_properties(Properties(0x0100 | Properties::READ)).0,
            gp::Properties::READ | gp::Properties::EXTENDED
        );
    }

    #[test]
    fn uuids_round_trip_including_long_ones() {
        for u in [
            Uuid::from_u16(0x2902),
            Uuid(0x6E40_0002_B5A3_F393_E0A9_E50E_24DC_CA9E),
        ] {
            assert_eq!(table_uuid(profile_uuid(u)), u);
        }
        assert_eq!(
            profile_uuid(Uuid::from_u16(0xFFF0)),
            gp::Uuid::from_u16(0xFFF0)
        );
        assert_eq!(profile_uuid(Uuid::from_u16(0xFFF0)).as_u16(), Some(0xFFF0));
    }

    #[test]
    fn structure_and_cccd_are_kept_by_default() {
        let (p, _) = to_profile(&table(), &SnapshotOptions::default());
        assert_eq!(p.services.len(), 2);
        let notify = &p.services[1].characteristics[0];
        assert_eq!(notify.descriptors.len(), 2);
        let cccd = &notify.descriptors[1];
        assert_eq!(cccd.uuid, gp::Uuid::from_u16(0x2902));
        assert_eq!(cccd.value, vec![0, 0]);
        assert_eq!(notify.descriptors[0].value.len(), 0);
    }

    #[test]
    fn dropping_the_cccd_keeps_the_map_aligned() {
        let opts = SnapshotOptions {
            drop_cccd: true,
            ..Default::default()
        };
        let (p, m) = to_profile(&table(), &opts);
        let notify = &p.services[1].characteristics[0];
        assert_eq!(notify.descriptors.len(), 1);
        assert_eq!(m.descriptor(1, 0, 0), Some(13));
        assert_eq!(m.descriptor(1, 0, 1), None);
    }

    #[test]
    fn the_map_names_the_peripheral_handles() {
        let (_, m) = to_profile(&table(), &SnapshotOptions::default());
        assert_eq!(m.characteristic(0, 0), Some(3));
        assert_eq!(m.characteristic(1, 0), Some(12));
        assert_eq!(m.characteristic(1, 1), Some(16));
        assert_eq!(m.characteristic(2, 0), None);
        assert_eq!(m.descriptor(1, 0, 1), Some(14));
    }

    #[test]
    fn names_and_advertised_services_are_carried() {
        let opts = SnapshotOptions {
            device_name: Some("Example".into()),
            advertised_name: Some("Example Adv".into()),
            advertised_services: vec![Uuid::from_u16(0xFFF0)],
            ..Default::default()
        };
        let (p, _) = to_profile(&table(), &opts);
        assert_eq!(p.device_name.as_deref(), Some("Example"));
        assert_eq!(p.advertised_name.as_deref(), Some("Example Adv"));
        assert_eq!(p.advertised_services, vec![gp::Uuid::from_u16(0xFFF0)]);
    }

    #[test]
    fn the_written_text_parses_back_to_the_same_profile() {
        let opts = SnapshotOptions {
            advertised_name: Some("Example Adv".into()),
            ..Default::default()
        };
        let (p, _) = to_profile(&table(), &opts);
        let text = gp::write(&p);
        assert_eq!(gp::parse(&text).unwrap(), p);
    }
}
