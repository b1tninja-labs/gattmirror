//! The relay: an ATT server whose dynamic values and writes are served by a [`Backend`].

use std::sync::mpsc::Receiver;

use att_server::{ErrorCode, Pdu, Server, Subscription};

use crate::backend::{Backend, BackendError, CharId, Notification};
use crate::table::{MirrorError, build_database};

/// Maps the ATT server's handle-addressed backend calls onto a [`Backend`]'s characteristics.
#[derive(Debug)]
pub struct Relay<B> {
    backend: B,
    value_handles: Vec<u16>,
}

impl<B> Relay<B> {
    fn char_at(&self, handle: u16) -> Option<CharId> {
        self.value_handles
            .iter()
            .position(|h| *h == handle)
            .map(CharId)
    }

    /// The adapter behind the relay.
    pub fn backend(&mut self) -> &mut B {
        &mut self.backend
    }
}

fn read_error(e: BackendError) -> ErrorCode {
    match e {
        BackendError::NotReadable => ErrorCode::ReadNotPermitted,
        BackendError::NotWritable | BackendError::Failed => ErrorCode::Unlikely,
    }
}

fn write_error(e: BackendError) -> ErrorCode {
    match e {
        BackendError::NotWritable => ErrorCode::WriteNotPermitted,
        BackendError::NotReadable | BackendError::Failed => ErrorCode::Unlikely,
    }
}

impl<B: Backend> att_server::Backend for Relay<B> {
    fn read(&mut self, handle: u16) -> Result<Vec<u8>, ErrorCode> {
        let ch = self.char_at(handle).ok_or(ErrorCode::InvalidHandle)?;
        self.backend.read(ch).map_err(read_error)
    }

    fn write(&mut self, handle: u16, value: &[u8], with_response: bool) -> Result<(), ErrorCode> {
        let ch = self.char_at(handle).ok_or(ErrorCode::InvalidHandle)?;
        self.backend
            .write(ch, value, with_response)
            .map_err(write_error)
    }

    fn subscription_changed(&mut self, value_handle: u16, subscription: Subscription) {
        if let Some(ch) = self.char_at(value_handle) {
            // A failure leaves the guest believing it is subscribed; the link layer will report
            // the loss, so there is nobody here to tell.
            let _ = self
                .backend
                .set_notify(ch, subscription.notify || subscription.indicate);
        }
    }
}

/// One guest connection's mirror: the ATT server plus the pump that turns the adapter's
/// notifications into queued ATT notifications.
#[derive(Debug)]
pub struct Mirror<B> {
    server: Server<Relay<B>>,
    rx: Receiver<Notification>,
    value_handles: Vec<u16>,
    dropped: usize,
}

impl<B: Backend> Mirror<B> {
    /// Mirror `backend`'s profile. `max_mtu` is the largest ATT MTU the server will accept.
    ///
    /// # Errors
    /// [`MirrorError`] when the table cannot be built or the backend's notification channel was
    /// already taken.
    pub fn new(mut backend: B, max_mtu: u16) -> Result<Self, MirrorError> {
        let table = build_database(backend.profile())?;
        let rx = backend
            .notifications()
            .ok_or(MirrorError::NotificationsTaken)?;
        let relay = Relay {
            backend,
            value_handles: table.value_handles.clone(),
        };
        Ok(Mirror {
            server: Server::new(table.database, relay, max_mtu),
            rx,
            value_handles: table.value_handles,
            dropped: 0,
        })
    }

    /// The ATT server, for inspection.
    pub fn server(&self) -> &Server<Relay<B>> {
        &self.server
    }

    /// The adapter behind the mirror.
    pub fn backend(&mut self) -> &mut B {
        self.server.backend().backend()
    }

    /// The value handle the guest sees for `ch`.
    #[must_use]
    pub fn value_handle(&self, ch: CharId) -> Option<u16> {
        self.value_handles.get(ch.0).copied()
    }

    /// Notifications that arrived while the guest was not subscribed, and so were discarded.
    #[must_use]
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Answer one ATT PDU from the guest.
    pub fn handle_bytes(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        self.server.handle_bytes(bytes)
    }

    /// Forget the guest: MTU, subscriptions and queued updates.
    pub fn reset(&mut self) {
        self.server.reset();
    }

    /// Move every notification the adapter has sent onto the server's queue. A value longer than
    /// the MTU allows is split into pieces of `MTU - 3` bytes, sent in order, using the MTU in
    /// force now. A notification for a characteristic the guest has not subscribed to is dropped.
    pub fn pump(&mut self) {
        while let Ok(n) = self.rx.try_recv() {
            let Some(&handle) = self.value_handles.get(n.char.0) else {
                self.dropped += 1;
                continue;
            };
            let sub = self.server.subscription(handle);
            if !sub.notify && !sub.indicate {
                self.dropped += 1;
                continue;
            }
            let payload = usize::from(self.server.mtu()).saturating_sub(3).max(1);
            for piece in n.value.chunks(payload) {
                let queued = if sub.notify {
                    self.server.notify(handle, piece)
                } else {
                    self.server.indicate(handle, piece)
                };
                if queued.is_err() {
                    self.dropped += 1;
                }
            }
        }
    }

    /// The next PDU to send to the guest, after pumping the adapter's notifications.
    pub fn poll_outgoing(&mut self) -> Option<Pdu> {
        self.pump();
        self.server.poll_outgoing()
    }

    /// Every PDU currently ready to send to the guest.
    pub fn drain_outgoing(&mut self) -> Vec<Pdu> {
        self.pump();
        std::iter::from_fn(|| self.server.poll_outgoing()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use att_server::Backend as _;

    use super::*;

    struct Stub {
        profile: gatt_profile::Profile,
        err: BackendError,
        rx: Option<Receiver<Notification>>,
    }

    impl Backend for Stub {
        fn profile(&self) -> &gatt_profile::Profile {
            &self.profile
        }
        fn read(&mut self, _: CharId) -> Result<Vec<u8>, BackendError> {
            Err(self.err)
        }
        fn write(&mut self, _: CharId, _: &[u8], _: bool) -> Result<(), BackendError> {
            Err(self.err)
        }
        fn set_notify(&mut self, _: CharId, _: bool) -> Result<(), BackendError> {
            Ok(())
        }
        fn notifications(&mut self) -> Option<Receiver<Notification>> {
            self.rx.take()
        }
    }

    fn relay(err: BackendError) -> Mirror<Stub> {
        let profile =
            gatt_profile::parse("service fff0 primary\n  characteristic fff1 read,write\n")
                .unwrap();
        let (_tx, rx) = channel();
        Mirror::new(
            Stub {
                profile,
                err,
                rx: Some(rx),
            },
            23,
        )
        .unwrap()
    }

    #[test]
    fn backend_errors_become_the_matching_att_errors() {
        let mut m = relay(BackendError::NotReadable);
        let h = m.value_handle(CharId(0)).unwrap();
        let r = m.server.backend();
        assert_eq!(r.read(h), Err(ErrorCode::ReadNotPermitted));
        assert_eq!(r.write(h, b"x", true), Err(ErrorCode::Unlikely));
        let mut m = relay(BackendError::NotWritable);
        let r = m.server.backend();
        assert_eq!(r.write(h, b"x", true), Err(ErrorCode::WriteNotPermitted));
        assert_eq!(r.read(h), Err(ErrorCode::Unlikely));
        let mut m = relay(BackendError::Failed);
        let r = m.server.backend();
        assert_eq!(r.read(h), Err(ErrorCode::Unlikely));
        assert_eq!(r.write(h, b"x", true), Err(ErrorCode::Unlikely));
    }

    #[test]
    fn a_handle_that_is_no_characteristic_is_invalid() {
        let mut m = relay(BackendError::Failed);
        let r = m.server.backend();
        assert_eq!(r.read(0x7F), Err(ErrorCode::InvalidHandle));
        assert_eq!(r.write(0x7F, b"", true), Err(ErrorCode::InvalidHandle));
    }
}
