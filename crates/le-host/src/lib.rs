//! LE peripheral host over `hci-wire`.
//!
//! This crate initialises a controller, advertises, and serves one ATT connection at a time over
//! the LE fixed channels (see [`Peripheral`]).
//! The Bluetooth Core Specification text was not available: every constant here was checked
//! against a live controller only, and the rows in `ROADMAP.md` say `[unread]` accordingly.

mod advertising;
mod controller;
#[cfg(test)]
mod fake;
mod l2cap;
mod peripheral;

pub use advertising::{AdvParams, Advertisement, Error as AdvertisementError, ServiceUuid};
pub use controller::{Controller, ControllerInfo, Error};
pub use l2cap::{CID_ATT, CID_SIGNALLING, CID_SMP, Reassembler, fragment};
pub use peripheral::{Handle, Peripheral};
