//! A reconnecting adapter over any GATT transport, platform independent.
//!
//! [`Adapter`] is what a mirror's real-adapter backend needs: a [`gatt_profile::Profile`] snapshot, reads,
//! writes and subscriptions addressed by *characteristic index* (position across the whole profile:
//! services in order, characteristics in order within each), and one notification channel that survives
//! reconnects. The platform is reached through the [`Transport`] and [`Connector`] traits, so every
//! behaviour here is tested against a fake transport; `winrt` supplies the real one on Windows.
//!
//! When an operation fails while the transport reports the link down, the adapter walks the
//! [`link::State`](crate::link::State) machine through back-off, reconnects through the
//! [`Connector`], checks the peripheral still offers the same table, re-enables the subscriptions the
//! caller had asked for, and retries the operation once.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use gatt_profile::Profile;

use crate::error::Error;
use crate::link::{self, Backoff, Event, State};
use crate::properties::Properties;
use crate::snapshot::{self, HandleMap, SnapshotOptions};
use crate::table::GattTable;
use crate::{ValueChange, WriteMode};

/// One live connection to a peripheral, addressed by attribute handle.
pub trait Transport {
    /// Read a characteristic value.
    ///
    /// # Errors
    /// A platform or status error.
    fn read(&mut self, handle: u16) -> Result<Vec<u8>, Error>;
    /// Write a value in one operation.
    ///
    /// # Errors
    /// A platform or status error.
    fn write(&mut self, handle: u16, data: &[u8], mode: WriteMode) -> Result<(), Error>;
    /// Enable notifications (or indications) for a characteristic.
    ///
    /// # Errors
    /// A platform or status error.
    fn subscribe(&mut self, handle: u16) -> Result<(), Error>;
    /// Disable notifications for a characteristic.
    ///
    /// # Errors
    /// A platform or status error.
    fn unsubscribe(&mut self, handle: u16) -> Result<(), Error>;
    /// Whether the link is up.
    fn is_connected(&self) -> bool;
}

/// Opens connections. Called again after every drop.
pub trait Connector {
    /// The transport this connector opens.
    type Transport: Transport;

    /// Connect, discover the table, and arrange for notifications to be sent on `notify`. The sender
    /// is the same for every call, so the receiver the adapter hands out survives reconnects.
    ///
    /// # Errors
    /// Any error from finding, connecting to or discovering the peripheral.
    fn connect(
        &mut self,
        notify: Sender<ValueChange>,
    ) -> Result<(Self::Transport, GattTable), Error>;
}

/// The adapter. See the module documentation.
pub struct Adapter<C: Connector> {
    connector: C,
    transport: C::Transport,
    table: GattTable,
    profile: Profile,
    map: HandleMap,
    wanted: BTreeSet<usize>,
    notify_tx: Sender<ValueChange>,
    notify_rx: Option<Receiver<ValueChange>>,
    backoff: Backoff,
    sleep: Box<dyn FnMut(Duration) -> bool>,
    state: Cell<State>,
    reconnects: u32,
}

impl<C: Connector> Adapter<C> {
    /// Connect (retrying per `backoff`) and snapshot the table into a profile.
    ///
    /// `sleep` waits for the given time and returns false to abandon the attempt (for a shutdown flag);
    /// tests pass a closure that does not sleep.
    ///
    /// # Errors
    /// The last connection error once `backoff` gives up.
    pub fn open(
        mut connector: C,
        snapshot: &SnapshotOptions,
        backoff: Backoff,
        mut sleep: Box<dyn FnMut(Duration) -> bool>,
    ) -> Result<Self, Error> {
        let (notify_tx, notify_rx) = mpsc::channel();
        let state = Cell::new(State::Idle);
        let (transport, table) =
            connect_with_retry(&mut connector, &notify_tx, &backoff, &state, &mut *sleep)?;
        let (profile, map) = snapshot::to_profile(&table, snapshot);
        Ok(Self {
            connector,
            transport,
            table,
            profile,
            map,
            wanted: BTreeSet::new(),
            notify_tx,
            notify_rx: Some(notify_rx),
            backoff,
            sleep,
            state,
            reconnects: 0,
        })
    }

    /// The peripheral's profile as of the first connection.
    #[must_use]
    pub const fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The connection state.
    #[must_use]
    pub fn state(&self) -> State {
        self.state.get()
    }

    /// How many times the link has been re-established since [`open`](Self::open).
    #[must_use]
    pub const fn reconnects(&self) -> u32 {
        self.reconnects
    }

    /// The notification channel. Yields the receiver once; later calls give `None`. Values carry the
    /// peripheral's attribute handle; [`index_of_handle`](Self::index_of_handle) maps it back.
    pub const fn take_notifications(&mut self) -> Option<Receiver<ValueChange>> {
        self.notify_rx.take()
    }

    /// The characteristic index that owns a peripheral handle.
    #[must_use]
    pub fn index_of_handle(&self, handle: u16) -> Option<usize> {
        self.map
            .characteristics
            .iter()
            .position(|(_, _, h)| *h == handle)
    }

    fn handle_of(&self, index: usize) -> Result<u16, Error> {
        self.map
            .characteristics
            .get(index)
            .map(|(_, _, h)| *h)
            .ok_or(Error::InvalidArgument(format!(
                "no characteristic with index {index}"
            )))
    }

    fn properties_of(&self, handle: u16) -> Result<Properties, Error> {
        self.table
            .characteristic(handle)
            .map(|c| c.properties)
            .ok_or(Error::UnknownHandle(handle))
    }

    /// Read a characteristic by index.
    ///
    /// # Errors
    /// An unknown index, a characteristic that cannot be read, or a link failure that reconnecting
    /// did not cure.
    pub fn read(&mut self, index: usize) -> Result<Vec<u8>, Error> {
        let handle = self.handle_of(index)?;
        if !self.properties_of(handle)?.has(Properties::READ) {
            return Err(Error::Unsupported("characteristic is not readable"));
        }
        self.run(|t| t.read(handle))
    }

    /// Write a characteristic by index.
    ///
    /// # Errors
    /// As [`read`](Self::read), for a write mode the characteristic does not declare.
    pub fn write(&mut self, index: usize, data: &[u8], mode: WriteMode) -> Result<(), Error> {
        let handle = self.handle_of(index)?;
        let needed = match mode {
            WriteMode::WithResponse => Properties::WRITE,
            WriteMode::WithoutResponse => Properties::WRITE_WITHOUT_RESPONSE,
        };
        if !self.properties_of(handle)?.has(needed) {
            return Err(Error::Unsupported(
                "characteristic does not declare this write mode",
            ));
        }
        self.run(|t| t.write(handle, data, mode))
    }

    /// Turn notifications for a characteristic on or off, and remember the choice so a reconnect
    /// restores it.
    ///
    /// # Errors
    /// As [`read`](Self::read), for a characteristic with neither notify nor indicate.
    pub fn set_notify(&mut self, index: usize, enabled: bool) -> Result<(), Error> {
        let handle = self.handle_of(index)?;
        if enabled && !self.properties_of(handle)?.can_subscribe() {
            return Err(Error::Unsupported(
                "characteristic has neither notify nor indicate",
            ));
        }
        if enabled {
            self.run(|t| t.subscribe(handle))?;
            self.wanted.insert(index);
        } else {
            self.wanted.remove(&index);
            self.run(|t| t.unsubscribe(handle))?;
        }
        Ok(())
    }

    /// Run one transport operation; on failure with the link down, reconnect and run it once more.
    fn run<T>(
        &mut self,
        mut op: impl FnMut(&mut C::Transport) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.state.get() == State::Closed {
            return Err(Error::State("closed".into()));
        }
        match op(&mut self.transport) {
            Err(e) if is_link_error(&e) && !self.transport.is_connected() => {
                self.reconnect()?;
                op(&mut self.transport)
            }
            other => other,
        }
    }

    fn reconnect(&mut self) -> Result<(), Error> {
        if self.state.get() == State::Ready {
            self.state.set(State::Ready.on(Event::Failed)?);
        }
        let (transport, table) = connect_with_retry(
            &mut self.connector,
            &self.notify_tx,
            &self.backoff,
            &self.state,
            &mut *self.sleep,
        )?;
        if table != self.table {
            self.state.set(State::Closed);
            return Err(Error::State(
                "the peripheral's table changed across the reconnect".into(),
            ));
        }
        self.transport = transport;
        self.reconnects += 1;
        for index in self.wanted.clone() {
            let handle = self.handle_of(index)?;
            self.transport.subscribe(handle)?;
        }
        Ok(())
    }

    /// Close the adapter; every later operation fails. The transport is dropped with it.
    pub fn close(&mut self) {
        self.state.set(State::Closed);
    }
}

/// A failure that might be the link rather than the request.
const fn is_link_error(e: &Error) -> bool {
    matches!(e, Error::Status(_) | Error::Platform(_))
}

/// Connect, driving the state machine and retrying per `backoff`. `Backoff` -> `Idle` happens when a
/// retry is due.
fn connect_with_retry<C: Connector>(
    connector: &mut C,
    notify: &Sender<ValueChange>,
    backoff: &Backoff,
    state: &Cell<State>,
    sleep: &mut dyn FnMut(Duration) -> bool,
) -> Result<(C::Transport, GattTable), Error> {
    let step = |e: Event| -> Result<(), Error> {
        state.set(state.get().on(e)?);
        Ok(())
    };
    // The `Cell` lets the attempt and the sleeper both advance the machine.
    link::retry(
        backoff,
        |_| {
            if state.get() == State::Backoff {
                step(Event::RetryDue)?;
            }
            step(Event::ScanStarted)?;
            match connector.connect(notify.clone()) {
                Ok(done) => {
                    step(Event::Found)?;
                    step(Event::Connected)?;
                    step(Event::Discovered)?;
                    Ok(done)
                }
                Err(e) => {
                    step(Event::Failed)?;
                    Err(e)
                }
            }
        },
        sleep,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::table::{Characteristic, Descriptor, Service};
    use crate::uuid::Uuid;

    #[derive(Default)]
    struct World {
        up: bool,
        connects_failing: u32,
        connects: u32,
        log: Vec<String>,
        notify: Option<Sender<ValueChange>>,
        table_variant: u8,
    }

    type Shared = Rc<RefCell<World>>;

    struct Fake(Shared);

    impl Transport for Fake {
        fn read(&mut self, h: u16) -> Result<Vec<u8>, Error> {
            let w = self.0.borrow();
            if !w.up {
                return Err(Error::Platform("link down".into()));
            }
            Ok(h.to_le_bytes().to_vec())
        }
        fn write(&mut self, h: u16, d: &[u8], m: WriteMode) -> Result<(), Error> {
            let mut w = self.0.borrow_mut();
            if !w.up {
                return Err(Error::Status("unreachable".into()));
            }
            w.log.push(format!("write {h} {d:?} {m:?}"));
            Ok(())
        }
        fn subscribe(&mut self, h: u16) -> Result<(), Error> {
            self.0.borrow_mut().log.push(format!("sub {h}"));
            Ok(())
        }
        fn unsubscribe(&mut self, h: u16) -> Result<(), Error> {
            self.0.borrow_mut().log.push(format!("unsub {h}"));
            Ok(())
        }
        fn is_connected(&self) -> bool {
            self.0.borrow().up
        }
    }

    struct FakeConnector(Shared);

    fn table(variant: u8) -> GattTable {
        let extra = u16::from(variant != 0);
        GattTable {
            services: vec![Service {
                handle: 1,
                uuid: Uuid::from_u16(0xFFF0),
                characteristics: vec![
                    Characteristic {
                        handle: 3,
                        uuid: Uuid::from_u16(0xFFF1),
                        properties: Properties(Properties::NOTIFY | Properties::READ),
                        descriptors: vec![Descriptor {
                            handle: 4,
                            uuid: snapshot::CCCD,
                        }],
                    },
                    Characteristic {
                        handle: 6 + extra,
                        uuid: Uuid::from_u16(0xFFF2),
                        properties: Properties(Properties::WRITE),
                        descriptors: vec![],
                    },
                ],
            }],
        }
    }

    impl Connector for FakeConnector {
        type Transport = Fake;
        fn connect(&mut self, notify: Sender<ValueChange>) -> Result<(Fake, GattTable), Error> {
            let mut w = self.0.borrow_mut();
            w.connects += 1;
            if w.connects_failing > 0 {
                w.connects_failing -= 1;
                return Err(Error::ScanTimeout);
            }
            w.up = true;
            w.notify = Some(notify);
            Ok((Fake(Rc::clone(&self.0)), table(w.table_variant)))
        }
    }

    fn open(world: &Shared, backoff: Backoff) -> Result<Adapter<FakeConnector>, Error> {
        Adapter::open(
            FakeConnector(Rc::clone(world)),
            &SnapshotOptions::default(),
            backoff,
            Box::new(|_| true),
        )
    }

    fn quick(max: Option<u32>) -> Backoff {
        Backoff {
            initial: Duration::ZERO,
            max: Duration::ZERO,
            max_attempts: max,
        }
    }

    #[test]
    fn open_snapshots_the_table_and_reaches_ready() {
        let w = Shared::default();
        let a = open(&w, quick(None)).unwrap();
        assert_eq!(a.state(), State::Ready);
        assert_eq!(a.profile().services[0].characteristics.len(), 2);
        assert_eq!(a.index_of_handle(6), Some(1));
        assert_eq!(a.index_of_handle(4), None);
    }

    #[test]
    fn open_retries_and_then_gives_up() {
        let w = Shared::default();
        w.borrow_mut().connects_failing = 2;
        let a = open(&w, quick(Some(5))).unwrap();
        assert_eq!(a.state(), State::Ready);
        assert_eq!(w.borrow().connects, 3);

        let w = Shared::default();
        w.borrow_mut().connects_failing = 99;
        assert_eq!(open(&w, quick(Some(3))).err(), Some(Error::ScanTimeout));
        assert_eq!(w.borrow().connects, 3);
    }

    #[test]
    fn operations_map_indices_to_handles_and_check_properties() {
        let w = Shared::default();
        let mut a = open(&w, quick(None)).unwrap();
        assert_eq!(a.read(0).unwrap(), vec![3, 0]);
        a.write(1, &[9], WriteMode::WithResponse).unwrap();
        assert!(
            w.borrow()
                .log
                .contains(&"write 6 [9] WithResponse".to_string())
        );
        assert!(matches!(a.read(1), Err(Error::Unsupported(_))));
        assert!(matches!(
            a.write(1, &[1], WriteMode::WithoutResponse),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(a.set_notify(1, true), Err(Error::Unsupported(_))));
        assert!(matches!(a.read(7), Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn a_dropped_link_reconnects_resubscribes_and_retries_the_operation() {
        let w = Shared::default();
        let mut a = open(&w, quick(None)).unwrap();
        a.set_notify(0, true).unwrap();
        w.borrow_mut().up = false;
        w.borrow_mut().connects_failing = 1;
        a.write(1, &[7], WriteMode::WithResponse).unwrap();
        assert_eq!(a.reconnects(), 1);
        assert_eq!(a.state(), State::Ready);
        let log = w.borrow().log.clone();
        // subscribed once, then again after the reconnect and before the retried write
        assert_eq!(log, ["sub 3", "sub 3", "write 6 [7] WithResponse"]);
    }

    #[test]
    fn a_disabled_subscription_is_not_restored() {
        let w = Shared::default();
        let mut a = open(&w, quick(None)).unwrap();
        a.set_notify(0, true).unwrap();
        a.set_notify(0, false).unwrap();
        w.borrow_mut().up = false;
        a.write(1, &[1], WriteMode::WithResponse).unwrap();
        assert_eq!(w.borrow().log.iter().filter(|l| *l == "sub 3").count(), 1);
    }

    #[test]
    fn notifications_survive_a_reconnect_on_one_channel() {
        let w = Shared::default();
        let mut a = open(&w, quick(None)).unwrap();
        let rx = a.take_notifications().unwrap();
        assert!(a.take_notifications().is_none());
        w.borrow_mut().up = false;
        a.read(0).unwrap();
        let tx = w.borrow().notify.clone().unwrap();
        tx.send(ValueChange {
            handle: 3,
            value: vec![1, 2],
        })
        .unwrap();
        let got = rx.try_recv().unwrap();
        assert_eq!(a.index_of_handle(got.handle), Some(0));
    }

    #[test]
    fn a_changed_table_closes_the_adapter() {
        let w = Shared::default();
        let mut a = open(&w, quick(None)).unwrap();
        w.borrow_mut().up = false;
        w.borrow_mut().table_variant = 1;
        assert!(matches!(a.read(0), Err(Error::State(_))));
        assert_eq!(a.state(), State::Closed);
        assert!(matches!(a.read(0), Err(Error::State(_))));
    }

    #[test]
    fn a_failed_reconnect_reports_the_error_and_a_non_link_error_does_not_reconnect() {
        let w = Shared::default();
        let mut a = open(&w, quick(Some(2))).unwrap();
        w.borrow_mut().up = false;
        w.borrow_mut().connects_failing = 99;
        assert_eq!(a.read(0), Err(Error::ScanTimeout));
        assert_eq!(a.state(), State::Backoff);

        let w = Shared::default();
        let mut a = open(&w, quick(None)).unwrap();
        a.close();
        assert!(a.write(1, &[0], WriteMode::WithResponse).is_err());
        assert_eq!(a.reconnects(), 0);
    }

    #[test]
    fn the_sleeper_can_abandon_the_reconnect() {
        let w = Shared::default();
        let mut a = Adapter::open(
            FakeConnector(Rc::clone(&w)),
            &SnapshotOptions::default(),
            Backoff {
                initial: Duration::from_millis(1),
                max: Duration::from_millis(1),
                max_attempts: None,
            },
            Box::new(|_| false),
        )
        .unwrap();
        w.borrow_mut().up = false;
        w.borrow_mut().connects_failing = 99;
        assert!(a.read(0).is_err());
        assert_eq!(w.borrow().connects, 2);
    }
}
