//! A [`Backend`] over a [`ByteStream`]: the GATT serial profile shim.
//!
//! A serial-over-BLE peripheral has one characteristic the host writes (commands) and one that
//! notifies (replies): FFF2 and FFF1 in the common FFF0 layout, or the RX and TX characteristics of
//! the Nordic UART layout. [`SerialBackend`] finds that pair in any profile, sends guest writes to
//! the stream, and turns the stream's reply chunks into notifications. Splitting a reply to the
//! guest's negotiated MTU is the [`Mirror`](crate::Mirror)'s job, so a reply of any length goes out
//! in order. Nothing here parses what the bytes mean.

use std::fmt;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use gatt_profile::{Profile, Properties};
use serial_stream::ByteStream;

use crate::backend::{Backend, BackendError, CharId, Notification, characteristics};

/// The FFF0 serial profile: FFF1 notifies (adapter to host), FFF2 is written (host to adapter).
pub const FFF0: &str = include_str!("../../gatt-profile/fixtures/fff0_serial.gatt");
/// The Nordic-UART-style profile with 128-bit UUIDs.
pub const NORDIC_UART: &str = include_str!("../../gatt-profile/fixtures/nordic_uart.gatt");

/// How guest writes are grouped before they reach the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteBuffering {
    /// Every guest write goes to the stream as it arrives.
    Passthrough,
    /// Writes accumulate; everything up to and including the last occurrence of the terminator is
    /// sent in one stream write, and the rest waits. A command split over several writes reaches
    /// the stream whole.
    UntilByte(u8),
}

/// Why a profile cannot be shimmed over a byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialError {
    /// No characteristic with notify or indicate.
    NoNotifyCharacteristic,
    /// No characteristic with write or write-without-response.
    NoWriteCharacteristic,
}

impl fmt::Display for SerialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoNotifyCharacteristic => "the profile has no notify characteristic",
            Self::NoWriteCharacteristic => "the profile has no writable characteristic",
        })
    }
}

impl std::error::Error for SerialError {}

/// The `(write, notify)` characteristics of a serial profile: the first of each in profile order.
///
/// # Errors
/// [`SerialError`] when either is missing.
pub fn serial_pair(profile: &Profile) -> Result<(CharId, CharId), SerialError> {
    let writable = Properties::WRITE | Properties::WRITE_NO_RESPONSE;
    let notifying = Properties::NOTIFY | Properties::INDICATE;
    let write = characteristics(profile)
        .find(|(_, _, c)| c.properties.0 & writable != 0)
        .map(|(id, _, _)| id)
        .ok_or(SerialError::NoWriteCharacteristic)?;
    let notify = characteristics(profile)
        .find(|(_, _, c)| c.properties.0 & notifying != 0)
        .map(|(id, _, _)| id)
        .ok_or(SerialError::NoNotifyCharacteristic)?;
    Ok((write, notify))
}

/// A [`Backend`] that serves a serial profile from a [`ByteStream`].
#[derive(Debug)]
pub struct SerialBackend<S: ByteStream> {
    profile: Profile,
    stream: S,
    write_char: CharId,
    notify_char: CharId,
    buffering: WriteBuffering,
    pending: Vec<u8>,
    rx: Option<Receiver<Notification>>,
}

impl<S: ByteStream> SerialBackend<S> {
    /// Serve `profile` from `stream`. Starts a thread that forwards the stream's reply chunks as
    /// notifications on the profile's notify characteristic; it ends when the stream does.
    ///
    /// # Errors
    /// [`SerialError`] when the profile has no serial pair, or the stream's receiver was taken.
    pub fn new(
        profile: Profile,
        mut stream: S,
        buffering: WriteBuffering,
    ) -> Result<Self, SerialError> {
        let (write_char, notify_char) = serial_pair(&profile)?;
        let replies = stream
            .receiver()
            .ok_or(SerialError::NoNotifyCharacteristic)?;
        let (tx, rx) = channel();
        thread::spawn(move || {
            while let Ok(value) = replies.recv() {
                let n = Notification {
                    char: notify_char,
                    value,
                };
                if tx.send(n).is_err() {
                    break;
                }
            }
        });
        Ok(SerialBackend {
            profile,
            stream,
            write_char,
            notify_char,
            buffering,
            pending: Vec::new(),
            rx: Some(rx),
        })
    }

    /// The characteristic the guest writes commands to.
    #[must_use]
    pub fn write_char(&self) -> CharId {
        self.write_char
    }

    /// The characteristic replies arrive on.
    #[must_use]
    pub fn notify_char(&self) -> CharId {
        self.notify_char
    }

    /// Bytes accepted from the guest but not yet sent (waiting for a terminator).
    #[must_use]
    pub fn pending(&self) -> &[u8] {
        &self.pending
    }

    /// The stream, for inspection.
    pub fn stream(&mut self) -> &mut S {
        &mut self.stream
    }
}

impl<S: ByteStream> Drop for SerialBackend<S> {
    fn drop(&mut self) {
        self.stream.close();
    }
}

impl<S: ByteStream> Backend for SerialBackend<S> {
    fn profile(&self) -> &Profile {
        &self.profile
    }

    fn read(&mut self, _ch: CharId) -> Result<Vec<u8>, BackendError> {
        Err(BackendError::NotReadable)
    }

    fn write(
        &mut self,
        ch: CharId,
        value: &[u8],
        _with_response: bool,
    ) -> Result<(), BackendError> {
        if ch != self.write_char {
            return Err(BackendError::NotWritable);
        }
        let send = match self.buffering {
            WriteBuffering::Passthrough => value.to_vec(),
            WriteBuffering::UntilByte(t) => {
                self.pending.extend_from_slice(value);
                match self.pending.iter().rposition(|b| *b == t) {
                    Some(end) => self.pending.drain(..=end).collect(),
                    None => return Ok(()),
                }
            }
        };
        if send.is_empty() {
            return Ok(());
        }
        self.stream.write(&send).map_err(|_| BackendError::Failed)
    }

    fn set_notify(&mut self, ch: CharId, _enabled: bool) -> Result<(), BackendError> {
        if ch == self.notify_char {
            Ok(())
        } else {
            Err(BackendError::Failed)
        }
    }

    fn notifications(&mut self) -> Option<Receiver<Notification>> {
        self.rx.take()
    }
}

#[cfg(test)]
mod tests {
    use serial_stream::mem::{self, MemStream};

    use super::*;

    fn backend(text: &str, b: WriteBuffering) -> (SerialBackend<MemStream>, MemStream) {
        let (ours, theirs) = mem::pair();
        let p = gatt_profile::parse(text).unwrap();
        (SerialBackend::new(p, ours, b).unwrap(), theirs)
    }

    #[test]
    fn both_fixtures_have_a_serial_pair() {
        for (text, w, n) in [
            (FFF0, 0xFFF2_u16, 0xFFF1_u16),
            (NORDIC_UART, 0x0002, 0x0003),
        ] {
            let p = gatt_profile::parse(text).unwrap();
            let (wc, nc) = serial_pair(&p).unwrap();
            let uuid = |id: CharId| {
                characteristics(&p)
                    .find(|(i, _, _)| *i == id)
                    .map(|(_, _, c)| c.uuid)
                    .unwrap()
            };
            assert_eq!(uuid(wc).as_u16().unwrap_or_else(|| short(uuid(wc))), w);
            assert_eq!(uuid(nc).as_u16().unwrap_or_else(|| short(uuid(nc))), n);
        }
    }

    /// The 16-bit field of a 128-bit UUID of the form `6e40xxxx-...`.
    fn short(u: gatt_profile::Uuid) -> u16 {
        u16::try_from((u.as_u128() >> 96) & 0xFFFF).unwrap()
    }

    #[test]
    fn a_profile_without_the_pair_is_refused() {
        let p = gatt_profile::parse("service 180a primary\n  characteristic 2a29 read\n").unwrap();
        assert_eq!(serial_pair(&p), Err(SerialError::NoWriteCharacteristic));
        let p = gatt_profile::parse("service fff0 primary\n  characteristic fff2 write\n").unwrap();
        assert_eq!(serial_pair(&p), Err(SerialError::NoNotifyCharacteristic));
    }

    #[test]
    fn passthrough_sends_each_write_as_it_arrives() {
        let (mut b, mut peer) = backend(FFF0, WriteBuffering::Passthrough);
        let rx = peer.receiver().unwrap();
        let w = b.write_char();
        b.write(w, b"AT", true).unwrap();
        b.write(w, b"I\r", false).unwrap();
        assert_eq!(rx.recv().unwrap(), b"AT");
        assert_eq!(rx.recv().unwrap(), b"I\r");
    }

    #[test]
    fn until_byte_holds_a_partial_command_and_sends_whole_lines() {
        let (mut b, mut peer) = backend(FFF0, WriteBuffering::UntilByte(b'\r'));
        let rx = peer.receiver().unwrap();
        let w = b.write_char();
        b.write(w, b"AT", true).unwrap();
        assert_eq!(b.pending(), b"AT");
        assert!(rx.try_recv().is_err());
        b.write(w, b"I\rATR", true).unwrap();
        assert_eq!(rx.recv().unwrap(), b"ATI\r");
        assert_eq!(b.pending(), b"ATR");
        b.write(w, b"V\r", true).unwrap();
        assert_eq!(rx.recv().unwrap(), b"ATRV\r");
        assert_eq!(b.pending(), b"");
        b.write(w, b"ATI\rATRV\r", true).unwrap();
        assert_eq!(rx.recv().unwrap(), b"ATI\rATRV\r");
    }

    #[test]
    fn replies_become_notifications_on_the_notify_characteristic() {
        let (mut b, mut peer) = backend(FFF0, WriteBuffering::Passthrough);
        let rx = b.notifications().unwrap();
        assert!(b.notifications().is_none());
        peer.write(b"OK\r>").unwrap();
        let n = rx.recv().unwrap();
        assert_eq!(n.char, b.notify_char());
        assert_eq!(n.value, b"OK\r>");
    }

    #[test]
    fn the_wrong_characteristic_and_reads_are_refused() {
        let (mut b, _peer) = backend(FFF0, WriteBuffering::Passthrough);
        let n = b.notify_char();
        assert_eq!(b.write(n, b"x", true), Err(BackendError::NotWritable));
        assert_eq!(b.read(n), Err(BackendError::NotReadable));
        assert_eq!(
            b.set_notify(b.write_char(), true),
            Err(BackendError::Failed)
        );
    }

    #[test]
    fn a_closed_stream_fails_the_write() {
        let (mut b, peer) = backend(FFF0, WriteBuffering::Passthrough);
        drop(peer);
        let w = b.write_char();
        assert_eq!(b.write(w, b"x", true), Err(BackendError::Failed));
    }

    struct Spy(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        Option<Receiver<Vec<u8>>>,
    );

    impl ByteStream for Spy {
        fn write(&mut self, _: &[u8]) -> Result<(), serial_stream::StreamError> {
            Ok(())
        }
        fn receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
            self.1.take()
        }
        fn close(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_the_backend_closes_the_stream() {
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_tx, rx) = channel();
        let spy = Spy(closed.clone(), Some(rx));
        let p = gatt_profile::parse(FFF0).unwrap();
        let b = SerialBackend::new(p, spy, WriteBuffering::Passthrough).unwrap();
        assert!(!closed.load(std::sync::atomic::Ordering::SeqCst));
        drop(b);
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    }
}
