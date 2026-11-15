//! The real-adapter side of the mirror.

use std::fmt;
use std::sync::mpsc::Receiver;

use gatt_profile::{Characteristic, Profile, Service};

/// A characteristic, by its position in the profile: services in order, characteristics in order
/// within each, counted from 0 across the whole profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharId(pub usize);

/// Every characteristic of `profile` with its id, in id order.
pub fn characteristics(
    profile: &Profile,
) -> impl Iterator<Item = (CharId, &Service, &Characteristic)> {
    profile
        .services
        .iter()
        .flat_map(|s| s.characteristics.iter().map(move |c| (s, c)))
        .enumerate()
        .map(|(i, (s, c))| (CharId(i), s, c))
}

/// A value the peripheral sent unprompted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// The characteristic it arrived on.
    pub char: CharId,
    /// The bytes, however long; the mirror splits them to fit the guest's MTU.
    pub value: Vec<u8>,
}

/// Why the peripheral refused or failed an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// The characteristic cannot be read.
    NotReadable,
    /// The characteristic cannot be written.
    NotWritable,
    /// Anything else: link lost, timeout, unknown characteristic.
    Failed,
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotReadable => "the characteristic is not readable",
            Self::NotWritable => "the characteristic is not writable",
            Self::Failed => "the adapter operation failed",
        })
    }
}

impl std::error::Error for BackendError {}

/// What the mirror needs from whatever stands behind it: a real adapter, or [`crate::sim`].
pub trait Backend {
    /// The peripheral's table and advertised identity.
    fn profile(&self) -> &Profile;

    /// Read a characteristic's current value.
    ///
    /// # Errors
    /// [`BackendError`] when the peripheral refuses or the link fails.
    fn read(&mut self, ch: CharId) -> Result<Vec<u8>, BackendError>;

    /// Write a characteristic; `with_response` is false for a Write Command.
    ///
    /// # Errors
    /// [`BackendError`] when the peripheral refuses or the link fails.
    fn write(&mut self, ch: CharId, value: &[u8], with_response: bool) -> Result<(), BackendError>;

    /// Turn the peripheral's notifications for `ch` on or off. Called when the guest changes its
    /// subscription, with true when either notify or indicate is enabled.
    ///
    /// # Errors
    /// [`BackendError`] when the subscription could not be changed.
    fn set_notify(&mut self, ch: CharId, enabled: bool) -> Result<(), BackendError>;

    /// The channel notifications arrive on. Yields the receiver once; later calls give `None`.
    fn notifications(&mut self) -> Option<Receiver<Notification>>;
}
