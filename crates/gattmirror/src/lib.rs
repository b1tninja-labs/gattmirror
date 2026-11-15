//! The mirror: a real (or simulated) BLE peripheral on one side, an ATT server for the emulator's
//! virtual controller on the other, and transport-only relay glue between them.
//!
//! - [`Backend`] is the real-adapter side: a profile snapshot, reads, writes and a notification
//!   channel, all addressed by [`CharId`].
//! - [`build_database`] turns a [`gatt_profile::Profile`] into an [`att_server::Database`].
//! - [`Mirror`] owns the [`att_server::Server`] and relays between it and the backend.
//! - [`sim`] is a test dummy adapter; [`cli`] parses the command line.
//!
//! Nothing here parses an application protocol. The Bluetooth Core Specification was not
//! available, so no behaviour is cited to a section; see `ROADMAP.md`.

pub mod backend;
pub mod cli;
pub mod mirror;
pub mod serial;
pub mod sim;
pub mod table;

pub use backend::{Backend, BackendError, CharId, Notification, characteristics};
pub use mirror::{Mirror, NotifyPolicy, Pump, Relay};
pub use table::{MirrorError, Table, apply_name, build_database};
