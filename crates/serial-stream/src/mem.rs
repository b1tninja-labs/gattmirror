//! In-memory streams for tests.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use crate::{ByteStream, StreamError};

/// One end of an in-memory pair.
#[derive(Debug)]
pub struct MemStream {
    tx: Option<Sender<Vec<u8>>>,
    rx: Option<Receiver<Vec<u8>>>,
    idle: Option<Duration>,
}

impl MemStream {
    /// Report `idle` from [`ByteStream::idle_timeout`].
    #[must_use]
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle = Some(idle);
        self
    }
}

/// Two connected ends: bytes written to one arrive, one chunk per write, on the other's receiver.
#[must_use]
pub fn pair() -> (MemStream, MemStream) {
    let (a_tx, b_rx) = channel();
    let (b_tx, a_rx) = channel();
    (
        MemStream {
            tx: Some(a_tx),
            rx: Some(a_rx),
            idle: None,
        },
        MemStream {
            tx: Some(b_tx),
            rx: Some(b_rx),
            idle: None,
        },
    )
}

impl ByteStream for MemStream {
    fn write(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        let tx = self.tx.as_ref().ok_or(StreamError::Closed)?;
        tx.send(bytes.to_vec()).map_err(|_| StreamError::Closed)
    }

    fn receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.rx.take()
    }

    fn close(&mut self) {
        self.tx = None;
    }

    fn idle_timeout(&self) -> Option<Duration> {
        self.idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_cross_in_both_directions_in_order() {
        let (mut a, mut b) = pair();
        let (arx, brx) = (a.receiver().unwrap(), b.receiver().unwrap());
        a.write(b"AT").unwrap();
        a.write(b"I\r").unwrap();
        b.write(b">").unwrap();
        assert_eq!(brx.recv().unwrap(), b"AT");
        assert_eq!(brx.recv().unwrap(), b"I\r");
        assert_eq!(arx.recv().unwrap(), b">");
    }

    #[test]
    fn the_receiver_is_handed_out_once() {
        let (mut a, _b) = pair();
        assert!(a.receiver().is_some());
        assert!(a.receiver().is_none());
    }

    #[test]
    fn close_fails_writes_and_disconnects_the_peer() {
        let (mut a, mut b) = pair();
        let brx = b.receiver().unwrap();
        a.close();
        a.close();
        assert_eq!(a.write(b"x"), Err(StreamError::Closed));
        assert!(brx.recv().is_err());
    }

    #[test]
    fn writing_to_a_dropped_peer_is_closed() {
        let (mut a, b) = pair();
        drop(b);
        assert_eq!(a.write(b"x"), Err(StreamError::Closed));
    }

    #[test]
    fn idle_timeout_is_a_hint_only_when_set() {
        let (a, b) = pair();
        assert_eq!(a.idle_timeout(), None);
        let b = b.with_idle_timeout(Duration::from_secs(5));
        assert_eq!(b.idle_timeout(), Some(Duration::from_secs(5)));
    }
}
