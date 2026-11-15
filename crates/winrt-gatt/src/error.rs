//! The crate's error type.

use std::fmt;

/// Everything that can go wrong, with no platform type in the signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A caller-supplied value was unusable (empty filter, malformed UUID or address).
    InvalidArgument(String),
    /// No advertiser matched within the timeout.
    ScanTimeout,
    /// The peripheral reported a GATT status other than success.
    Status(String),
    /// A handle does not name an attribute in the discovered table.
    UnknownHandle(u16),
    /// The attribute does not support the requested operation.
    Unsupported(&'static str),
    /// The platform API failed.
    Platform(String),
    /// A state transition that the connection state machine does not allow.
    State(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(s) => write!(f, "invalid argument: {s}"),
            Self::ScanTimeout => f.write_str("no matching advertiser seen before the timeout"),
            Self::Status(s) => write!(f, "GATT status: {s}"),
            Self::UnknownHandle(h) => write!(f, "no attribute with handle {h:#06x}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Platform(s) => write!(f, "platform error: {s}"),
            Self::State(s) => write!(f, "state: {s}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(windows)]
impl From<::windows::core::Error> for Error {
    fn from(e: ::windows::core::Error) -> Self {
        Self::Platform(e.to_string())
    }
}
