//! A Bluetooth LE GATT *client* for Windows.
//!
//! The crate is transport only: it discovers a peripheral's complete GATT table into plain data
//! ([`GattTable`]), and reads, writes and subscribes by attribute handle. It knows nothing about what the
//! bytes mean.
//!
//! Everything except the `winrt` module is platform independent and unit tested: UUIDs, the table
//! model, scan filters, address text, write chunking, the connection state machine and the reconnect
//! back-off. The Windows module is compiled only with `cfg(windows)`, so the workspace still builds
//! elsewhere.

pub mod adapter;
pub mod address;
pub mod chunk;
pub mod error;
pub mod filter;
pub mod link;
pub mod properties;
pub mod snapshot;
pub mod table;
pub mod uuid;

#[cfg(windows)]
mod winrt;

#[cfg(windows)]
pub use winrt::{
    ConnectOptions, DEFAULT_TIMEOUT, Found, Target, WinrtClient, WinrtConnector, scan,
};

pub use error::Error;
pub use filter::ScanFilter;
pub use properties::Properties;
pub use table::{Characteristic, Descriptor, GattTable, Service};
pub use uuid::Uuid;

/// How a value is written to a characteristic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    /// ATT Write Request: the peripheral acknowledges.
    WithResponse,
    /// ATT Write Command: no acknowledgement.
    WithoutResponse,
}

/// A value change delivered by a notification or indication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueChange {
    /// Attribute handle of the characteristic that changed.
    pub handle: u16,
    /// The bytes the peripheral sent.
    pub value: Vec<u8>,
}
