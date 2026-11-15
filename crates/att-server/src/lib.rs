//! An ATT/GATT server with no I/O.
//!
//! Three layers, none of which touches a socket or a controller:
//!
//! - [`Pdu`]: encode and decode ATT protocol data units.
//! - [`Database`]: a GATT attribute table (services, characteristics, descriptors) with handle
//!   assignment, built with [`DatabaseBuilder`].
//! - [`Server`]: answers one connection's requests from a [`Database`], consulting a [`Backend`] for
//!   dynamic values and for writes, tracking the negotiated MTU and each Client Characteristic
//!   Configuration, and queueing notifications and indications for the transport to send.
//!
//! A relay plugs a real device in behind [`Backend`]. The crate parses no application protocol.
//!
//! The Bluetooth Core Specification text was **not** available when this was written, so no
//! behaviour is cited to a section; see `ROADMAP.md`.

mod db;
mod pdu;
mod server;
mod uuid;

pub use db::{Access, Attribute, BuildError, Database, DatabaseBuilder, Props, Value};
pub use pdu::{DecodeError, ErrorCode, GroupEntry, Pdu, opcode};
pub use server::{Backend, NoBackend, SendError, Server, Subscription};
pub use uuid::Uuid;

/// The ATT MTU every connection starts with.
pub const DEFAULT_MTU: u16 = 23;
