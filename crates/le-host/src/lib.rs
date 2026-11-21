//! LE peripheral host over `hci-wire`.
//!
//! This crate initialises a controller, advertises, and serves one ATT connection at a time over
//! the LE fixed channels (see [`Peripheral`]).
//! Rows in `ROADMAP.md` name the Core Specification 7.0 sections that were read. Advertising-data
//! type identifiers and the Flags bit field are not among them.

mod advertising;
mod controller;
#[cfg(test)]
mod fake;
mod l2cap;
mod peripheral;

pub use advertising::{AdvParams, Advertisement, Error as AdvertisementError, ServiceUuid};
pub use controller::{Controller, ControllerInfo, Error};
pub use l2cap::{CID_ATT, CID_SIGNALLING, CID_SMP, Reassembler, fragment};
pub use peripheral::{
    Direction, Handle, LinkEvent, LinkHook, NotifyPolicy, Peripheral, Source, Trace,
};
