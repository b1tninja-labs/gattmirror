use std::fmt;

use crate::model::{Characteristic, Descriptor, Profile, Service};
use crate::uuid::Uuid;

/// Which profile an item exists in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The profile `diff` was called on (by convention the source).
    Left,
    /// The profile passed in (by convention the mirror).
    Right,
}

/// One way two profiles differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Difference {
    /// Device names differ.
    DeviceName(Option<String>, Option<String>),
    /// Advertised names differ.
    AdvertisedName(Option<String>, Option<String>),
    /// Advertised service lists differ.
    AdvertisedServices(Vec<Uuid>, Vec<Uuid>),
    /// A service exists on one side only.
    Service(Uuid, Side),
    /// A service is primary on one side and secondary on the other.
    ServiceKind(Uuid),
    /// A characteristic exists on one side only.
    Characteristic {
        /// Containing service.
        service: Uuid,
        /// The characteristic.
        uuid: Uuid,
        /// The side that has it.
        only_in: Side,
    },
    /// Property bits differ.
    Properties {
        /// Containing service.
        service: Uuid,
        /// The characteristic.
        uuid: Uuid,
        /// Bits on the left.
        left: u8,
        /// Bits on the right.
        right: u8,
    },
    /// Static values differ (compared only when both sides carry one).
    Value {
        /// Containing service.
        service: Uuid,
        /// The characteristic.
        uuid: Uuid,
    },
    /// A descriptor exists on one side only.
    Descriptor {
        /// Containing service.
        service: Uuid,
        /// Containing characteristic.
        characteristic: Uuid,
        /// The descriptor.
        uuid: Uuid,
        /// The side that has it.
        only_in: Side,
    },
    /// Descriptor values differ (compared only when both are non-empty).
    DescriptorValue {
        /// Containing service.
        service: Uuid,
        /// Containing characteristic.
        characteristic: Uuid,
        /// The descriptor.
        uuid: Uuid,
    },
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
        })
    }
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceName(a, b) => write!(f, "device name: {a:?} vs {b:?}"),
            Self::AdvertisedName(a, b) => write!(f, "advertised name: {a:?} vs {b:?}"),
            Self::AdvertisedServices(a, b) => write!(f, "advertised services: {a:?} vs {b:?}"),
            Self::Service(u, s) => write!(f, "service {u} only in {s}"),
            Self::ServiceKind(u) => write!(f, "service {u} differs in primary/secondary"),
            Self::Characteristic {
                service,
                uuid,
                only_in,
            } => write!(
                f,
                "service {service}: characteristic {uuid} only in {only_in}"
            ),
            Self::Properties {
                service,
                uuid,
                left,
                right,
            } => write!(
                f,
                "service {service}: characteristic {uuid} properties {left:#04x} vs {right:#04x}"
            ),
            Self::Value { service, uuid } => {
                write!(
                    f,
                    "service {service}: characteristic {uuid} static value differs"
                )
            }
            Self::Descriptor {
                service,
                characteristic,
                uuid,
                only_in,
            } => write!(
                f,
                "service {service}, characteristic {characteristic}: descriptor {uuid} only in {only_in}"
            ),
            Self::DescriptorValue {
                service,
                characteristic,
                uuid,
            } => write!(
                f,
                "service {service}, characteristic {characteristic}: descriptor {uuid} value differs"
            ),
        }
    }
}

/// Pair items by UUID and occurrence number (the second `fff1` pairs with the second `fff1`).
fn pair<'a, T>(
    left: &'a [T],
    right: &'a [T],
    key: impl Fn(&T) -> Uuid,
) -> (Vec<(&'a T, &'a T)>, Vec<&'a T>, Vec<&'a T>) {
    let occurrence = |items: &'a [T], i: usize| {
        items[..i]
            .iter()
            .filter(|x| key(x) == key(&items[i]))
            .count()
    };
    let mut both = Vec::new();
    let mut only_left = Vec::new();
    let mut used = vec![false; right.len()];
    for i in 0..left.len() {
        let n = occurrence(left, i);
        let found = (0..right.len())
            .find(|&j| key(&right[j]) == key(&left[i]) && occurrence(right, j) == n);
        match found {
            Some(j) => {
                used[j] = true;
                both.push((&left[i], &right[j]));
            }
            None => only_left.push(&left[i]),
        }
    }
    let only_right = right
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(r, _)| r)
        .collect();
    (both, only_left, only_right)
}

fn diff_descriptors(
    s: Uuid,
    c: Uuid,
    l: &Characteristic,
    r: &Characteristic,
    out: &mut Vec<Difference>,
) {
    let (both, ol, or) = pair(&l.descriptors, &r.descriptors, |d: &Descriptor| d.uuid);
    for d in ol {
        out.push(Difference::Descriptor {
            service: s,
            characteristic: c,
            uuid: d.uuid,
            only_in: Side::Left,
        });
    }
    for d in or {
        out.push(Difference::Descriptor {
            service: s,
            characteristic: c,
            uuid: d.uuid,
            only_in: Side::Right,
        });
    }
    for (a, b) in both {
        if !a.value.is_empty() && !b.value.is_empty() && a.value != b.value {
            out.push(Difference::DescriptorValue {
                service: s,
                characteristic: c,
                uuid: a.uuid,
            });
        }
    }
}

fn diff_service(l: &Service, r: &Service, out: &mut Vec<Difference>) {
    let s = l.uuid;
    if l.kind != r.kind {
        out.push(Difference::ServiceKind(s));
    }
    let (both, ol, or) = pair(
        &l.characteristics,
        &r.characteristics,
        |c: &Characteristic| c.uuid,
    );
    for c in ol {
        out.push(Difference::Characteristic {
            service: s,
            uuid: c.uuid,
            only_in: Side::Left,
        });
    }
    for c in or {
        out.push(Difference::Characteristic {
            service: s,
            uuid: c.uuid,
            only_in: Side::Right,
        });
    }
    for (a, b) in both {
        if a.properties != b.properties {
            out.push(Difference::Properties {
                service: s,
                uuid: a.uuid,
                left: a.properties.0,
                right: b.properties.0,
            });
        }
        if let (Some(x), Some(y)) = (&a.value, &b.value)
            && x != y
        {
            out.push(Difference::Value {
                service: s,
                uuid: a.uuid,
            });
        }
        diff_descriptors(s, a.uuid, a, b, out);
    }
}

impl Profile {
    /// Compare `self` (conventionally the source) with `other` (the mirror).
    ///
    /// Services, characteristics and descriptors are paired by UUID and occurrence, so handle order does
    /// not matter. A static value is compared only when both sides carry one, and a descriptor value
    /// only when both are non-empty, because a mirror serves live values the snapshot may lack. The
    /// result is empty when the two present the same table.
    #[must_use]
    pub fn diff(&self, other: &Profile) -> Vec<Difference> {
        let mut out = Vec::new();
        if self.device_name != other.device_name {
            out.push(Difference::DeviceName(
                self.device_name.clone(),
                other.device_name.clone(),
            ));
        }
        if self.advertised_name != other.advertised_name {
            out.push(Difference::AdvertisedName(
                self.advertised_name.clone(),
                other.advertised_name.clone(),
            ));
        }
        if self.advertised_services != other.advertised_services {
            out.push(Difference::AdvertisedServices(
                self.advertised_services.clone(),
                other.advertised_services.clone(),
            ));
        }
        let (both, ol, or) = pair(&self.services, &other.services, |s: &Service| s.uuid);
        for s in ol {
            out.push(Difference::Service(s.uuid, Side::Left));
        }
        for s in or {
            out.push(Difference::Service(s.uuid, Side::Right));
        }
        for (a, b) in both {
            diff_service(a, b, &mut out);
        }
        out
    }
}
