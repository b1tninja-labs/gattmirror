//! A peripheral's GATT table as plain data, independent of any other crate.
//!
//! Handles are the attribute handles the peripheral assigned; they are the keys for read, write and
//! subscribe. Included-service declarations are not captured (`[open]`).

use crate::properties::Properties;
use crate::uuid::Uuid;

/// A descriptor of a characteristic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// Attribute handle.
    pub handle: u16,
    /// Descriptor type.
    pub uuid: Uuid,
}

/// A characteristic and its descriptors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Characteristic {
    /// Attribute handle of the characteristic as the platform reports it.
    pub handle: u16,
    /// Characteristic type.
    pub uuid: Uuid,
    /// Declared properties.
    pub properties: Properties,
    /// Descriptors, in discovery order.
    pub descriptors: Vec<Descriptor>,
}

/// A service and its characteristics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    /// Attribute handle of the service.
    pub handle: u16,
    /// Service type.
    pub uuid: Uuid,
    /// Characteristics, in discovery order.
    pub characteristics: Vec<Characteristic>,
}

/// A complete discovered table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GattTable {
    /// Services, in discovery order.
    pub services: Vec<Service>,
}

impl GattTable {
    /// Every characteristic with the service that holds it, in table order.
    pub fn characteristics(&self) -> impl Iterator<Item = (&Service, &Characteristic)> {
        self.services
            .iter()
            .flat_map(|s| s.characteristics.iter().map(move |c| (s, c)))
    }

    /// Find a characteristic by handle.
    #[must_use]
    pub fn characteristic(&self, handle: u16) -> Option<&Characteristic> {
        self.characteristics()
            .map(|(_, c)| c)
            .find(|c| c.handle == handle)
    }

    /// Find the first characteristic of a type, optionally inside a given service.
    #[must_use]
    pub fn find(&self, service: Option<Uuid>, uuid: Uuid) -> Option<&Characteristic> {
        self.characteristics()
            .filter(|(s, _)| service.is_none_or(|u| s.uuid == u))
            .map(|(_, c)| c)
            .find(|c| c.uuid == uuid)
    }

    /// Find a descriptor by handle, with the characteristic that owns it.
    #[must_use]
    pub fn descriptor(&self, handle: u16) -> Option<(&Characteristic, &Descriptor)> {
        self.characteristics()
            .flat_map(|(_, c)| c.descriptors.iter().map(move |d| (c, d)))
            .find(|(_, d)| d.handle == handle)
    }

    /// Check that handles are unique across the table, which a mirror depends on.
    ///
    /// # Errors
    /// Returns the first duplicated handle.
    pub fn check_unique_handles(&self) -> Result<(), crate::Error> {
        let mut seen = std::collections::HashSet::new();
        let mut handles = Vec::new();
        for s in &self.services {
            handles.push(s.handle);
            for c in &s.characteristics {
                handles.push(c.handle);
                handles.extend(c.descriptors.iter().map(|d| d.handle));
            }
        }
        for h in handles {
            if !seen.insert(h) {
                return Err(crate::Error::InvalidArgument(format!(
                    "duplicate attribute handle {h:#06x}"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GattTable {
        GattTable {
            services: vec![
                Service {
                    handle: 1,
                    uuid: Uuid::from_u16(0xFFF0),
                    characteristics: vec![
                        Characteristic {
                            handle: 3,
                            uuid: Uuid::from_u16(0xFFF1),
                            properties: Properties(Properties::NOTIFY),
                            descriptors: vec![Descriptor {
                                handle: 4,
                                uuid: Uuid::from_u16(0x2902),
                            }],
                        },
                        Characteristic {
                            handle: 6,
                            uuid: Uuid::from_u16(0xFFF2),
                            properties: Properties(Properties::WRITE),
                            descriptors: vec![],
                        },
                    ],
                },
                Service {
                    handle: 8,
                    uuid: Uuid::from_u16(0x180A),
                    characteristics: vec![Characteristic {
                        handle: 10,
                        uuid: Uuid::from_u16(0xFFF1),
                        properties: Properties(Properties::READ),
                        descriptors: vec![],
                    }],
                },
            ],
        }
    }

    #[test]
    fn lookup_by_handle_and_type() {
        let t = sample();
        assert_eq!(t.characteristic(6).unwrap().uuid, Uuid::from_u16(0xFFF2));
        assert!(t.characteristic(99).is_none());
        let (c, d) = t.descriptor(4).unwrap();
        assert_eq!(c.handle, 3);
        assert_eq!(d.uuid, Uuid::from_u16(0x2902));
    }

    #[test]
    fn find_is_scoped_by_service() {
        let t = sample();
        assert_eq!(t.find(None, Uuid::from_u16(0xFFF1)).unwrap().handle, 3);
        let scoped = t
            .find(Some(Uuid::from_u16(0x180A)), Uuid::from_u16(0xFFF1))
            .unwrap();
        assert_eq!(scoped.handle, 10);
        assert!(
            t.find(Some(Uuid::from_u16(0x180A)), Uuid::from_u16(0xFFF2))
                .is_none()
        );
    }

    #[test]
    fn duplicate_handles_are_reported() {
        let mut t = sample();
        assert!(t.check_unique_handles().is_ok());
        t.services[1].characteristics[0].handle = 3;
        assert!(t.check_unique_handles().is_err());
    }
}
