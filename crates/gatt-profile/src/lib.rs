//! A no-I/O model of a BLE peripheral's GATT table, and a plain-text format for saving it.
//!
//! [`Profile`] holds services, characteristics and descriptors plus the advertised identity.
//! [`parse()`] reads the text format with line-numbered errors, [`write()`] emits it, and
//! [`Profile::attributes`] flattens a profile into the handle-ordered attribute list an ATT server
//! would serve. Nothing here touches a socket, an adapter or a file.

mod attribute;
mod model;
mod parse;
mod uuid;
mod write;

pub use attribute::{Attribute, AttributeError, AttributeRole, type_uuids};
pub use model::{Characteristic, Descriptor, Profile, Properties, Service, ServiceKind};
pub use parse::{ParseError, ParseErrorKind, parse};
pub use uuid::{BASE_UUID, ParseUuidError, Uuid};
pub use write::write;
