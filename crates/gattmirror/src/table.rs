//! Profile to attribute database.

use std::fmt;

use att_server::{Access, BuildError, Database, Props, Uuid, Value};
use gatt_profile::{Profile, Properties};

use crate::backend::CharId;

/// The Client Characteristic Configuration UUID. The database builder adds one for every
/// characteristic with notify or indicate, so a profile's own copy is skipped.
const CCCD: u16 = 0x2902;
/// The GAP Device Name characteristic.
const DEVICE_NAME: u16 = 0x2A00;

/// The built database and where each characteristic's value landed.
#[derive(Clone, Debug)]
pub struct Table {
    /// The attribute database to serve.
    pub database: Database,
    /// Value handle by [`CharId`] index.
    pub value_handles: Vec<u16>,
}

impl Table {
    /// The value handle of `ch`.
    #[must_use]
    pub fn value_handle(&self, ch: CharId) -> Option<u16> {
        self.value_handles.get(ch.0).copied()
    }

    /// The characteristic whose value is at `handle`.
    #[must_use]
    pub fn char_at(&self, handle: u16) -> Option<CharId> {
        self.value_handles
            .iter()
            .position(|h| *h == handle)
            .map(CharId)
    }
}

/// Why a profile could not be mirrored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorError {
    /// The attribute database could not be built.
    Build(BuildError),
    /// The backend's notification channel was already taken.
    NotificationsTaken,
}

impl From<BuildError> for MirrorError {
    fn from(e: BuildError) -> Self {
        Self::Build(e)
    }
}

impl fmt::Display for MirrorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(e) => write!(f, "cannot build the attribute table: {e:?}"),
            Self::NotificationsTaken => f.write_str("the backend's notification channel is gone"),
        }
    }
}

impl std::error::Error for MirrorError {}

fn uuid(u: gatt_profile::Uuid) -> Uuid {
    Uuid::from_u128(u.as_u128())
}

/// Replace the advertised and GAP device names, including a captured Device Name value.
pub fn apply_name(profile: &mut Profile, name: &str) {
    profile.advertised_name = Some(name.to_owned());
    profile.device_name = Some(name.to_owned());
    for s in &mut profile.services {
        for c in &mut s.characteristics {
            if c.uuid.as_u16() == Some(DEVICE_NAME) && c.value.is_some() {
                c.value = Some(name.as_bytes().to_vec());
            }
        }
    }
}

/// Build the attribute database for `profile`.
///
/// A characteristic whose value the profile captured and that can only be read (no write, notify or
/// indicate) is served from the database, so reads of it never reach the adapter. Every other
/// characteristic value is dynamic: reads and writes go to the backend. Descriptors are served as
/// captured, read-only, and a profile's own CCCD descriptor is dropped in favour of the one the
/// builder adds.
///
/// # Errors
/// [`MirrorError::Build`] when the handle space runs out.
pub fn build_database(profile: &Profile) -> Result<Table, MirrorError> {
    let mut b = Database::builder();
    let mut value_handles = Vec::new();
    for service in &profile.services {
        b.service(uuid(service.uuid))?;
        for ch in &service.characteristics {
            let props = ch.properties.0;
            let read_only = props
                & (Properties::WRITE
                    | Properties::WRITE_NO_RESPONSE
                    | Properties::NOTIFY
                    | Properties::INDICATE)
                == 0;
            let value = match (&ch.value, read_only) {
                (Some(v), true) => Value::Static(v.clone()),
                _ => Value::Dynamic,
            };
            value_handles.push(b.characteristic(uuid(ch.uuid), Props(props), value)?);
            for d in ch
                .descriptors
                .iter()
                .filter(|d| d.uuid.as_u16() != Some(CCCD))
            {
                b.descriptor(uuid(d.uuid), Access::READ, Value::Static(d.value.clone()))?;
            }
        }
    }
    Ok(Table {
        database: b.build(),
        value_handles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::characteristics;

    fn fixture() -> Profile {
        gatt_profile::parse(crate::sim::FFF0_SERIAL).unwrap()
    }

    #[test]
    fn handles_match_the_flattened_profile() {
        let p = fixture();
        let t = build_database(&p).unwrap();
        let flat = p.attributes().unwrap();
        let ours: Vec<(u16, Uuid)> = t
            .database
            .attributes()
            .iter()
            .map(|a| (a.handle(), a.uuid()))
            .collect();
        let theirs: Vec<(u16, Uuid)> = flat.iter().map(|a| (a.handle, uuid(a.attr_type))).collect();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn value_handles_follow_char_ids() {
        let p = fixture();
        let t = build_database(&p).unwrap();
        assert_eq!(characteristics(&p).count(), t.value_handles.len());
        let h = t.value_handle(CharId(1)).unwrap();
        assert_eq!(t.char_at(h), Some(CharId(1)));
        assert_eq!(t.database.get(h).unwrap().uuid(), Uuid::from_u16(0xFFF1));
    }

    #[test]
    fn read_only_captured_values_are_static_and_the_rest_dynamic() {
        let t = build_database(&fixture()).unwrap();
        let name = t.value_handle(CharId(0)).unwrap();
        assert!(matches!(
            t.database.get(name).unwrap().value(),
            Value::Static(_)
        ));
        let tx = t.value_handle(CharId(1)).unwrap();
        assert_eq!(t.database.get(tx).unwrap().value(), &Value::Dynamic);
    }

    #[test]
    fn apply_name_rewrites_the_captured_device_name() {
        let mut p = fixture();
        apply_name(&mut p, "Mirror");
        assert_eq!(p.advertised_name.as_deref(), Some("Mirror"));
        assert_eq!(
            p.services[0].characteristics[0].value.as_deref(),
            Some(&b"Mirror"[..])
        );
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;

    #[test]
    fn a_captured_value_on_a_live_characteristic_stays_dynamic() {
        let p = gatt_profile::parse(
            "service 180f primary\n  characteristic 2a19 read,notify value=64\n  characteristic 2a1a read value=01\n",
        )
        .unwrap();
        let t = build_database(&p).unwrap();
        let live = t.database.get(t.value_handle(CharId(0)).unwrap()).unwrap();
        assert_eq!(live.value(), &Value::Dynamic);
        let fixed = t.database.get(t.value_handle(CharId(1)).unwrap()).unwrap();
        assert_eq!(fixed.value(), &Value::Static(vec![1]));
    }
}
