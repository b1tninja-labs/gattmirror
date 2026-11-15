//! LE peripheral host over `hci-wire`.
//!
//! This slice initialises a controller and advertises. Connections, ACL and L2CAP come later.
//! The Bluetooth Core Specification text was not available: every constant here was checked
//! against a live controller only, and the rows in `ROADMAP.md` say `[unread]` accordingly.

mod advertising;
mod controller;
#[cfg(test)]
mod fake;

pub use advertising::{AdvParams, Advertisement, Error as AdvertisementError, ServiceUuid};
pub use controller::{Controller, ControllerInfo, Error};
