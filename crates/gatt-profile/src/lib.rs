//! A no-I/O model of a BLE peripheral's GATT table, and a plain-text format for saving it.
//!
//! [`Profile`] holds services, characteristics and descriptors plus the advertised identity.
//! [`parse()`] reads the text format with line-numbered errors, [`write()`] emits it, and
//! [`Profile::attributes`] flattens a profile into the handle-ordered attribute list an ATT server
//! would serve. Nothing here touches a socket, an adapter or a file.

pub mod assigned;
mod attribute;
mod diff;
mod lint;
mod model;
mod parse;
mod serial;
mod uuid;
mod write;

pub use attribute::{Attribute, AttributeError, AttributeRole, type_uuids};
pub use diff::{Difference, Side};
pub use lint::{Issue, IssueKind, Severity};
pub use model::{Characteristic, Descriptor, Profile, Properties, Service, ServiceKind};
pub use parse::{FORMAT_VERSION, ParseError, ParseErrorKind, parse};
pub use serial::SerialRoles;
pub use uuid::{BASE_UUID, ParseUuidError, Uuid};
pub use write::{write, write_annotated};
