use std::collections::HashSet;
use std::fmt;

use crate::model::{Characteristic, Profile, Properties, Service};
use crate::uuid::Uuid;

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Probably a mistake, but a peripheral could legitimately look like this.
    Warning,
    /// A table that a client could not use as described.
    Error,
}

/// What a lint found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// The same service UUID appears twice (legal for separate instances, usually a mistake).
    DuplicateService(Uuid),
    /// The same characteristic UUID appears twice in one service.
    DuplicateCharacteristic(Uuid),
    /// The same descriptor UUID appears twice on one characteristic.
    DuplicateDescriptor(Uuid),
    /// The same UUID is advertised twice.
    DuplicateAdvertisedService(Uuid),
    /// An advertised service UUID that the table does not contain.
    AdvertisedServiceNotInTable(Uuid),
    /// A service with no characteristics.
    EmptyService,
    /// A characteristic with no property bits.
    NoProperties,
    /// Notify or indicate is set but there is no `2902` descriptor to enable it.
    NotifyWithoutCccd,
    /// A `2902` descriptor on a characteristic that cannot notify or indicate.
    CccdWithoutNotify,
    /// A `2902` static value that is not two bytes.
    CccdBadValue(usize),
    /// A static value on a characteristic without the read property.
    ValueWithoutRead,
    /// The extended-properties bit is set but there is no `2900` descriptor.
    ExtendedWithoutDescriptor,
    /// A `2900` descriptor without the extended-properties bit.
    ExtendedDescriptorWithoutBit,
}

impl fmt::Display for IssueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateService(u) => write!(f, "service {u} appears more than once"),
            Self::DuplicateCharacteristic(u) => {
                write!(
                    f,
                    "characteristic {u} appears more than once in the service"
                )
            }
            Self::DuplicateDescriptor(u) => {
                write!(
                    f,
                    "descriptor {u} appears more than once on the characteristic"
                )
            }
            Self::DuplicateAdvertisedService(u) => {
                write!(f, "advertised service {u} is listed twice")
            }
            Self::AdvertisedServiceNotInTable(u) => {
                write!(f, "advertised service {u} is not in the table")
            }
            Self::EmptyService => f.write_str("service has no characteristics"),
            Self::NoProperties => f.write_str("characteristic has no properties"),
            Self::NotifyWithoutCccd => {
                f.write_str("notify or indicate is set but there is no 2902 descriptor")
            }
            Self::CccdWithoutNotify => {
                f.write_str("2902 descriptor on a characteristic without notify or indicate")
            }
            Self::CccdBadValue(n) => write!(f, "2902 value is {n} bytes, expected 2"),
            Self::ValueWithoutRead => {
                f.write_str("static value on a characteristic that cannot be read")
            }
            Self::ExtendedWithoutDescriptor => {
                f.write_str("extended property is set but there is no 2900 descriptor")
            }
            Self::ExtendedDescriptorWithoutBit => {
                f.write_str("2900 descriptor without the extended property")
            }
        }
    }
}

/// One finding, with where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Seriousness.
    pub severity: Severity,
    /// Index into [`Profile::services`], when the finding is about a service or inside one.
    pub service: Option<usize>,
    /// Index into the service's characteristics, when inside one.
    pub characteristic: Option<usize>,
    /// What was found.
    pub kind: IssueKind,
}

impl Issue {
    /// Render with the UUIDs of the place it was found, e.g. `error: service fff0, characteristic fff1: ...`.
    #[must_use]
    pub fn describe(&self, profile: &Profile) -> String {
        let sev = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        let mut place = Vec::new();
        if let Some(s) = self.service.and_then(|i| profile.services.get(i)) {
            place.push(format!("service {}", s.uuid));
            if let Some(c) = self.characteristic.and_then(|i| s.characteristics.get(i)) {
                place.push(format!("characteristic {}", c.uuid));
            }
        }
        if place.is_empty() {
            format!("{sev}: {}", self.kind)
        } else {
            format!("{sev}: {}: {}", place.join(", "), self.kind)
        }
    }
}

const CCCD: Uuid = Uuid::from_u16(0x2902);
const CEPD: Uuid = Uuid::from_u16(0x2900);

fn first_duplicate(uuids: impl Iterator<Item = Uuid>) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    let mut dup = Vec::new();
    for u in uuids {
        if !seen.insert(u) && !dup.contains(&u) {
            dup.push(u);
        }
    }
    dup
}

fn lint_characteristic(si: usize, ci: usize, c: &Characteristic, out: &mut Vec<Issue>) {
    let mut push = |severity, kind| {
        out.push(Issue {
            severity,
            service: Some(si),
            characteristic: Some(ci),
            kind,
        });
    };
    let p = c.properties;
    let can_notify = p.0 & (Properties::NOTIFY | Properties::INDICATE) != 0;
    let cccd = c.descriptors.iter().find(|d| d.uuid == CCCD);
    if p.0 == 0 {
        push(Severity::Warning, IssueKind::NoProperties);
    }
    if can_notify && cccd.is_none() {
        push(Severity::Error, IssueKind::NotifyWithoutCccd);
    }
    if let Some(d) = cccd {
        if !can_notify {
            push(Severity::Error, IssueKind::CccdWithoutNotify);
        }
        if !d.value.is_empty() && d.value.len() != 2 {
            push(Severity::Error, IssueKind::CccdBadValue(d.value.len()));
        }
    }
    if c.value.is_some() && !p.contains(Properties::READ) {
        push(Severity::Warning, IssueKind::ValueWithoutRead);
    }
    let cepd = c.descriptors.iter().any(|d| d.uuid == CEPD);
    if p.contains(Properties::EXTENDED) && !cepd {
        push(Severity::Warning, IssueKind::ExtendedWithoutDescriptor);
    }
    if cepd && !p.contains(Properties::EXTENDED) {
        push(Severity::Warning, IssueKind::ExtendedDescriptorWithoutBit);
    }
    for u in first_duplicate(c.descriptors.iter().map(|d| d.uuid)) {
        push(Severity::Error, IssueKind::DuplicateDescriptor(u));
    }
}

fn lint_service(si: usize, s: &Service, out: &mut Vec<Issue>) {
    if s.characteristics.is_empty() {
        out.push(Issue {
            severity: Severity::Warning,
            service: Some(si),
            characteristic: None,
            kind: IssueKind::EmptyService,
        });
    }
    for u in first_duplicate(s.characteristics.iter().map(|c| c.uuid)) {
        out.push(Issue {
            severity: Severity::Warning,
            service: Some(si),
            characteristic: None,
            kind: IssueKind::DuplicateCharacteristic(u),
        });
    }
    for (ci, c) in s.characteristics.iter().enumerate() {
        lint_characteristic(si, ci, c, out);
    }
}

impl Profile {
    /// Check the table for internal consistency, returning every finding in table order.
    ///
    /// The rules are the crate's own reading of what a usable table looks like; none is verified
    /// against the Core Specification (`[unread]` in the roadmap).
    #[must_use]
    pub fn lint(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        for u in first_duplicate(self.advertised_services.iter().copied()) {
            out.push(Issue {
                severity: Severity::Warning,
                service: None,
                characteristic: None,
                kind: IssueKind::DuplicateAdvertisedService(u),
            });
        }
        for u in &self.advertised_services {
            if !self.services.iter().any(|s| s.uuid == *u) {
                out.push(Issue {
                    severity: Severity::Warning,
                    service: None,
                    characteristic: None,
                    kind: IssueKind::AdvertisedServiceNotInTable(*u),
                });
            }
        }
        for u in first_duplicate(self.services.iter().map(|s| s.uuid)) {
            out.push(Issue {
                severity: Severity::Warning,
                service: None,
                characteristic: None,
                kind: IssueKind::DuplicateService(u),
            });
        }
        for (si, s) in self.services.iter().enumerate() {
            lint_service(si, s, &mut out);
        }
        out
    }
}
