//! The common currency of the mirror: a duplex byte stream.
//!
//! A [`ByteStream`] carries command bytes one way (blocking [`ByteStream::write`]) and reply bytes
//! the other (a channel of chunks). The ELM327 dialect is ASCII lines and replies ending in `>`, but
//! nothing here parses it: chunk boundaries are arbitrary and carry no meaning.
//!
//! - [`mem::pair`]: two connected in-memory ends for tests.
//! - [`tcp::TcpClient`]: a stream over a TCP connection.
//! - [`tcp::Server`]: serves a factory of streams over TCP (loopback by default), one stream per
//!   accepted connection.

pub mod mem;
pub mod tcp;

use std::fmt;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Why a stream operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// The stream is closed, locally or by the peer.
    Closed,
    /// An I/O error of this kind.
    Io(std::io::ErrorKind),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("the stream is closed"),
            Self::Io(k) => write!(f, "I/O error: {k}"),
        }
    }
}

impl std::error::Error for StreamError {}

impl From<std::io::Error> for StreamError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.kind())
    }
}

/// A duplex byte stream.
pub trait ByteStream: Send {
    /// Send command bytes. Blocks until they are accepted, so a slow peer pushes back.
    ///
    /// # Errors
    /// [`StreamError::Closed`] once the stream is closed; [`StreamError::Io`] on a transport error.
    fn write(&mut self, bytes: &[u8]) -> Result<(), StreamError>;

    /// The channel reply bytes arrive on, in order, in chunks of any size. Yields the receiver once;
    /// later calls give `None`. The channel disconnects when the stream ends.
    fn receiver(&mut self) -> Option<Receiver<Vec<u8>>>;

    /// Close the stream. Idempotent. The peer's receiver disconnects and later writes fail.
    fn close(&mut self);

    /// How long a silent peer may be waited for before a reply is considered lost, if the stream
    /// knows. A hint for the caller; the stream itself never times out because of it.
    fn idle_timeout(&self) -> Option<Duration> {
        None
    }
}

impl ByteStream for Box<dyn ByteStream> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        (**self).write(bytes)
    }
    fn receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
        (**self).receiver()
    }
    fn close(&mut self) {
        (**self).close();
    }
    fn idle_timeout(&self) -> Option<Duration> {
        (**self).idle_timeout()
    }
}
