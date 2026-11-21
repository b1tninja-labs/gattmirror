//! [`gattmirror::Backend`] over `winrt-gatt`: the real-adapter side of the mirror.
//!
//! [`WinrtBackend`] wraps a [`winrt_gatt::adapter::Adapter`], so it reconnects, restores subscriptions
//! and keeps one notification channel. It is generic over the adapter's `Connector`, which is what
//! lets the tests run it on a fake connector. `connect` builds the Windows one. Transport only: no
//! application protocol is parsed. Nothing here has been run against a real adapter.

use std::sync::mpsc::{self, Receiver};
use std::thread;

pub mod stream;
pub use stream::{GattStream, ReconnectHook, SerialOptions};

use gatt_profile::Profile;
use gattmirror::{Backend, BackendError, CharId, Notification};
use winrt_gatt::adapter::{Adapter, Connector};
use winrt_gatt::{Error, WriteMode};

/// Map a `winrt-gatt` error for an operation of the given kind onto the mirror's error.
fn map_error(e: &Error, on_read: bool, on_write: bool) -> BackendError {
    match e {
        Error::Unsupported(_) if on_read => BackendError::NotReadable,
        Error::Unsupported(_) if on_write => BackendError::NotWritable,
        _ => BackendError::Failed,
    }
}

/// The backend. Built from an opened [`Adapter`].
pub struct WinrtBackend<C: Connector> {
    adapter: Adapter<C>,
    notifications: Option<Receiver<Notification>>,
}

impl<C: Connector> WinrtBackend<C> {
    /// Wrap an opened adapter. Starts a thread that turns the adapter's handle-addressed values into
    /// [`Notification`]s; it ends when the backend is dropped or the receiver is.
    pub fn new(mut adapter: Adapter<C>) -> Self {
        let handles = adapter.handles();
        let raw = adapter.take_notifications();
        let notifications = raw.map(|raw| {
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                for change in raw {
                    let Some(index) = handles.iter().position(|h| *h == change.handle) else {
                        continue;
                    };
                    let n = Notification {
                        char: CharId(index),
                        value: change.value,
                    };
                    if tx.send(n).is_err() {
                        break;
                    }
                }
            });
            rx
        });
        Self {
            adapter,
            notifications,
        }
    }

    /// The adapter underneath, for state and reconnect counts.
    #[must_use]
    pub const fn adapter(&self) -> &Adapter<C> {
        &self.adapter
    }
}

impl<C: Connector> Backend for WinrtBackend<C> {
    fn profile(&self) -> &Profile {
        self.adapter.profile()
    }

    fn read(&mut self, ch: CharId) -> Result<Vec<u8>, BackendError> {
        self.adapter
            .read(ch.0)
            .map_err(|e| map_error(&e, true, false))
    }

    fn write(&mut self, ch: CharId, value: &[u8], with_response: bool) -> Result<(), BackendError> {
        let mode = if with_response {
            WriteMode::WithResponse
        } else {
            WriteMode::WithoutResponse
        };
        self.adapter
            .write(ch.0, value, mode)
            .map_err(|e| map_error(&e, false, true))
    }

    fn set_notify(&mut self, ch: CharId, enabled: bool) -> Result<(), BackendError> {
        self.adapter
            .set_notify(ch.0, enabled)
            .map_err(|e| map_error(&e, false, false))
    }

    fn notifications(&mut self) -> Option<Receiver<Notification>> {
        self.notifications.take()
    }
}

/// Find a peripheral by name substring or address text, connect, and return the backend.
///
/// Compile-checked only; never run against a device.
///
/// # Errors
/// An unusable target, or the last connection error once the back-off gives up.
#[cfg(windows)]
pub fn connect(
    target: &str,
    scan_timeout: std::time::Duration,
    backoff: winrt_gatt::link::Backoff,
) -> Result<WinrtBackend<winrt_gatt::WinrtConnector>, Error> {
    use winrt_gatt::snapshot::SnapshotOptions;
    use winrt_gatt::{ConnectOptions, ScanFilter, Target, WinrtConnector, address};

    let target = match address::parse_address(target) {
        Ok(a) => Target::Address(a),
        Err(_) => Target::Filter(ScanFilter::by_name(target)),
    };
    let connector = WinrtConnector::new(target, scan_timeout, ConnectOptions::default());
    let snapshot = SnapshotOptions {
        drop_cccd: true,
        ..SnapshotOptions::default()
    };
    let adapter = Adapter::open(
        connector,
        &snapshot,
        backoff,
        Box::new(|d| {
            thread::sleep(d);
            true
        }),
    )?;
    Ok(WinrtBackend::new(adapter))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    use winrt_gatt::adapter::Transport;
    use winrt_gatt::link::Backoff;
    use winrt_gatt::snapshot::SnapshotOptions;
    use winrt_gatt::table::{Characteristic, GattTable, Service};
    use winrt_gatt::{Properties, Uuid, ValueChange};

    use super::*;

    #[derive(Default)]
    struct World {
        writes: Vec<(u16, Vec<u8>, WriteMode)>,
        notify: Option<Sender<ValueChange>>,
    }

    struct Fake(Rc<RefCell<World>>);

    impl Transport for Fake {
        fn read(&mut self, h: u16) -> Result<Vec<u8>, Error> {
            Ok(h.to_le_bytes().to_vec())
        }
        fn write(&mut self, h: u16, d: &[u8], m: WriteMode) -> Result<(), Error> {
            self.0.borrow_mut().writes.push((h, d.to_vec(), m));
            Ok(())
        }
        fn subscribe(&mut self, _: u16) -> Result<(), Error> {
            Ok(())
        }
        fn unsubscribe(&mut self, _: u16) -> Result<(), Error> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
    }

    struct FakeConnector(Rc<RefCell<World>>);

    impl Connector for FakeConnector {
        type Transport = Fake;
        fn connect(&mut self, tx: Sender<ValueChange>) -> Result<(Fake, GattTable), Error> {
            self.0.borrow_mut().notify = Some(tx);
            let table = GattTable {
                services: vec![
                    Service {
                        handle: 1,
                        uuid: Uuid::from_u16(0x1800),
                        characteristics: vec![Characteristic {
                            handle: 3,
                            uuid: Uuid::from_u16(0x2A00),
                            properties: Properties(Properties::READ),
                            descriptors: vec![],
                        }],
                    },
                    Service {
                        handle: 10,
                        uuid: Uuid::from_u16(0xFFF0),
                        characteristics: vec![
                            Characteristic {
                                handle: 12,
                                uuid: Uuid::from_u16(0xFFF1),
                                properties: Properties(Properties::NOTIFY),
                                descriptors: vec![],
                            },
                            Characteristic {
                                handle: 15,
                                uuid: Uuid::from_u16(0xFFF2),
                                properties: Properties(Properties::WRITE),
                                descriptors: vec![],
                            },
                        ],
                    },
                ],
            };
            Ok((Fake(Rc::clone(&self.0)), table))
        }
    }

    fn backend() -> (WinrtBackend<FakeConnector>, Rc<RefCell<World>>) {
        let w = Rc::new(RefCell::new(World::default()));
        let adapter = Adapter::open(
            FakeConnector(Rc::clone(&w)),
            &SnapshotOptions::default(),
            Backoff {
                initial: Duration::ZERO,
                max: Duration::ZERO,
                max_attempts: Some(1),
            },
            Box::new(|_| true),
        )
        .unwrap();
        (WinrtBackend::new(adapter), w)
    }

    #[test]
    fn profile_and_char_ids_follow_the_table() {
        let (b, _) = backend();
        let ids: Vec<_> = gattmirror::characteristics(b.profile())
            .map(|(id, _, c)| (id.0, c.uuid.as_u16()))
            .collect();
        assert_eq!(
            ids,
            [(0, Some(0x2A00)), (1, Some(0xFFF1)), (2, Some(0xFFF2))]
        );
    }

    #[test]
    fn reads_and_writes_reach_the_right_handle() {
        let (mut b, w) = backend();
        assert_eq!(b.read(CharId(0)).unwrap(), vec![3, 0]);
        b.write(CharId(2), &[5], true).unwrap();
        assert_eq!(w.borrow().writes, [(15, vec![5], WriteMode::WithResponse)]);
    }

    #[test]
    fn refusals_map_to_the_mirrors_errors() {
        let (mut b, _) = backend();
        assert_eq!(b.read(CharId(2)), Err(BackendError::NotReadable));
        assert_eq!(
            b.write(CharId(0), &[1], true),
            Err(BackendError::NotWritable)
        );
        assert_eq!(
            b.write(CharId(2), &[1], false),
            Err(BackendError::NotWritable)
        );
        assert_eq!(b.set_notify(CharId(2), true), Err(BackendError::Failed));
        assert_eq!(b.read(CharId(9)), Err(BackendError::Failed));
        assert!(b.set_notify(CharId(1), true).is_ok());
    }

    #[test]
    fn notifications_arrive_by_char_id_and_unknown_handles_are_dropped() {
        let (mut b, w) = backend();
        let rx = b.notifications().unwrap();
        assert!(b.notifications().is_none());
        let tx = w.borrow().notify.clone().unwrap();
        tx.send(ValueChange {
            handle: 99,
            value: vec![0],
        })
        .unwrap();
        tx.send(ValueChange {
            handle: 12,
            value: vec![1, 2, 3],
        })
        .unwrap();
        let n = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(n.char, CharId(1));
        assert_eq!(n.value, vec![1, 2, 3]);
    }
}
