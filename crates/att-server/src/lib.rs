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
//! # Service Changed
//!
//! The table is fixed for the life of a [`Server`]; this crate never adds the Service Changed
//! characteristic and never sends an indication for it. A mirror that rebuilds its table (because
//! the real device changed) should do so by building a new [`Database`] and a new [`Server`] for
//! the next connection. A guest that cached the old table under a bond is the risk; the relay
//! should not bond (see the `[open]` row in `ROADMAP.md`).
//!
//! The Bluetooth Core Specification text was **not** available when this was written, so no
//! behaviour is cited to a section; see `ROADMAP.md`.

mod db;
mod pdu;
mod server;
mod uuid;

pub use db::{Access, Attribute, BuildError, Database, DatabaseBuilder, Level, Props, Value};
pub use pdu::{DecodeError, ErrorCode, GroupEntry, Pdu, opcode};
pub use server::{Backend, CONFIRMATION_TIMEOUT, NoBackend, SendError, Server, Subscription};
pub use uuid::Uuid;

/// The ATT MTU every connection starts with.
pub const DEFAULT_MTU: u16 = 23;
