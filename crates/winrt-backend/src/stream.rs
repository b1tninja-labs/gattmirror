//! A [`serial_stream::ByteStream`] over a real GATT serial pair.
//!
//! The adapter lives on one dedicated worker thread, built there by a factory, so every platform call
//! happens on a single thread of our own (no UI or STA thread is involved) and the stream itself is
//! `Send`. The profile is searched with [`gattmirror::serial::serial_pair`] for the first writable and
//! the first notifying characteristic, which covers the FFF0 and Nordic UART shapes alike. Notifications
//! on the notify characteristic become received chunks; writes are split to the MTU by the adapter.
//! Reconnect and re-subscription are the adapter's; [`ReconnectHook`] tells the owner when one happened.
//! Nothing here has been run against a real adapter.

use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gatt_profile::Properties;
use gattmirror::characteristics;
use gattmirror::serial::serial_pair;
use serial_stream::{ByteStream, StreamError};
use winrt_gatt::adapter::{Adapter, Connector};
use winrt_gatt::{Error, WriteMode};

/// Called on the worker thread with the running count of reconnects after one completes.
pub type ReconnectHook = Box<dyn FnMut(u32) + Send>;

/// Stream options.
#[derive(Clone, Copy, Debug, Default)]
pub struct SerialOptions {
    /// When the write characteristic declares both write kinds, use Write Without Response (faster, no
    /// acknowledgement). Otherwise the one declared kind is used regardless.
    pub prefer_without_response: bool,
}

enum Cmd {
    Write(Vec<u8>, Sender<Result<(), StreamError>>),
    Close,
}

/// A byte stream over a GATT serial pair. See the module documentation.
pub struct GattStream {
    cmd: Option<Sender<Cmd>>,
    rx: Option<Receiver<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
}

fn to_stream_error(e: &Error) -> StreamError {
    match e {
        Error::State(_) => StreamError::Closed,
        _ => StreamError::Io(ErrorKind::Other),
    }
}

impl GattStream {
    /// Start the worker, build the adapter on it with `make`, find the serial pair and enable
    /// notifications.
    ///
    /// # Errors
    /// Whatever `make` fails with, a profile without a write and a notify characteristic, or a failure
    /// enabling notifications.
    pub fn open<C, F>(
        make: F,
        options: SerialOptions,
        hook: Option<ReconnectHook>,
    ) -> Result<Self, Error>
    where
        C: Connector,
        F: FnOnce() -> Result<Adapter<C>, Error> + Send + 'static,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (data_tx, data_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker = thread::spawn(move || run(make, options, hook, &cmd_rx, data_tx, &ready_tx));
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                cmd: Some(cmd_tx),
                rx: Some(data_rx),
                worker: Some(worker),
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err(Error::Platform("the stream worker ended early".into()))
            }
        }
    }
}

fn run<C, F>(
    make: F,
    options: SerialOptions,
    mut hook: Option<ReconnectHook>,
    cmd_rx: &Receiver<Cmd>,
    data_tx: Sender<Vec<u8>>,
    ready: &Sender<Result<(), Error>>,
) where
    C: Connector,
    F: FnOnce() -> Result<Adapter<C>, Error>,
{
    let mut adapter = match make() {
        Ok(a) => a,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let (write, notify) = match serial_pair(adapter.profile()) {
        Ok(p) => p,
        Err(e) => {
            let _ = ready.send(Err(Error::InvalidArgument(e.to_string())));
            return;
        }
    };
    let write_props = characteristics(adapter.profile())
        .find(|(id, _, _)| *id == write)
        .map_or(0, |(_, _, c)| c.properties.0);
    let mode = if write_props & Properties::WRITE != 0
        && (write_props & Properties::WRITE_NO_RESPONSE == 0 || !options.prefer_without_response)
    {
        WriteMode::WithResponse
    } else {
        WriteMode::WithoutResponse
    };
    if let Err(e) = adapter.set_notify(notify.0, true) {
        let _ = ready.send(Err(e));
        return;
    }
    let notify_handle = adapter.handles()[notify.0];
    let stop = Arc::new(AtomicBool::new(false));
    let forwarder = adapter.take_notifications().map(|raw| {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match raw.recv_timeout(Duration::from_millis(100)) {
                    Ok(c) if c.handle == notify_handle => {
                        if data_tx.send(c.value).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
    });
    let _ = ready.send(Ok(()));

    let mut seen = adapter.reconnects();
    while let Ok(Cmd::Write(bytes, reply)) = cmd_rx.recv() {
        let result = adapter.write(write.0, &bytes, mode);
        if adapter.reconnects() != seen {
            seen = adapter.reconnects();
            if let Some(h) = hook.as_mut() {
                h(seen);
            }
        }
        let _ = reply.send(result.map_err(|e| to_stream_error(&e)));
    }
    stop.store(true, Ordering::SeqCst);
    adapter.close();
    if let Some(f) = forwarder {
        let _ = f.join();
    }
}

impl ByteStream for GattStream {
    fn write(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        let cmd = self.cmd.as_ref().ok_or(StreamError::Closed)?;
        let (tx, rx) = mpsc::channel();
        cmd.send(Cmd::Write(bytes.to_vec(), tx))
            .map_err(|_| StreamError::Closed)?;
        rx.recv().map_err(|_| StreamError::Closed)?
    }

    fn receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.rx.take()
    }

    fn close(&mut self) {
        if let Some(cmd) = self.cmd.take() {
            let _ = cmd.send(Cmd::Close);
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for GattStream {
    fn drop(&mut self) {
        self.close();
    }
}

/// Find a peripheral by name substring or address text and open a stream over its serial pair, with
/// every platform call on the stream's own worker thread.
///
/// Compile-checked only; never run against a device.
///
/// # Errors
/// As [`GattStream::open`].
#[cfg(windows)]
pub fn connect_stream(
    target: &str,
    scan_timeout: Duration,
    backoff: winrt_gatt::link::Backoff,
    options: SerialOptions,
    hook: Option<ReconnectHook>,
) -> Result<GattStream, Error> {
    use winrt_gatt::snapshot::SnapshotOptions;
    use winrt_gatt::{ConnectOptions, ScanFilter, Target, WinrtConnector, address};

    let target = target.to_owned();
    GattStream::open(
        move || {
            let target = match address::parse_address(&target) {
                Ok(a) => Target::Address(a),
                Err(_) => Target::Filter(ScanFilter::by_name(target)),
            };
            let connector = WinrtConnector::new(target, scan_timeout, ConnectOptions::default());
            Adapter::open(
                connector,
                &SnapshotOptions::default(),
                backoff,
                Box::new(|d| {
                    thread::sleep(d);
                    true
                }),
            )
        },
        options,
        hook,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use winrt_gatt::adapter::Transport;
    use winrt_gatt::link::Backoff;
    use winrt_gatt::snapshot::SnapshotOptions;
    use winrt_gatt::table::{Characteristic, GattTable, Service};
    use winrt_gatt::{Properties as P, Uuid, ValueChange};

    use super::*;

    #[derive(Default)]
    struct World {
        up: bool,
        fail_connects: u32,
        connects: u32,
        log: Vec<String>,
        notify: Option<Sender<ValueChange>>,
        write_props: u16,
        no_serial: bool,
    }

    type Shared = Arc<Mutex<World>>;

    struct Fake(Shared);

    impl Transport for Fake {
        fn read(&mut self, _: u16) -> Result<Vec<u8>, Error> {
            Ok(vec![])
        }
        fn write(&mut self, h: u16, d: &[u8], m: WriteMode) -> Result<(), Error> {
            let mut w = self.0.lock().unwrap();
            if !w.up {
                return Err(Error::Platform("down".into()));
            }
            w.log.push(format!("write {h} {} {m:?}", d.len()));
            Ok(())
        }
        fn subscribe(&mut self, h: u16) -> Result<(), Error> {
            self.0.lock().unwrap().log.push(format!("sub {h}"));
            Ok(())
        }
        fn unsubscribe(&mut self, _: u16) -> Result<(), Error> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            self.0.lock().unwrap().up
        }
    }

    struct FakeConnector(Shared);

    impl Connector for FakeConnector {
        type Transport = Fake;
        fn connect(&mut self, tx: Sender<ValueChange>) -> Result<(Fake, GattTable), Error> {
            let mut w = self.0.lock().unwrap();
            w.connects += 1;
            if w.fail_connects > 0 {
                w.fail_connects -= 1;
                return Err(Error::ScanTimeout);
            }
            w.up = true;
            w.notify = Some(tx);
            let mut chars = vec![Characteristic {
                handle: 3,
                uuid: Uuid::from_u16(0x2A00),
                properties: winrt_gatt::Properties(P::READ),
                descriptors: vec![],
            }];
            if !w.no_serial {
                chars.push(Characteristic {
                    handle: 12,
                    uuid: Uuid::from_u16(0xFFF1),
                    properties: winrt_gatt::Properties(P::NOTIFY),
                    descriptors: vec![],
                });
                chars.push(Characteristic {
                    handle: 15,
                    uuid: Uuid::from_u16(0xFFF2),
                    properties: winrt_gatt::Properties(w.write_props),
                    descriptors: vec![],
                });
            }
            let table = GattTable {
                services: vec![Service {
                    handle: 1,
                    uuid: Uuid::from_u16(0xFFF0),
                    characteristics: chars,
                }],
            };
            Ok((Fake(Arc::clone(&self.0)), table))
        }
    }

    fn world(write_props: u16) -> Shared {
        Arc::new(Mutex::new(World {
            write_props,
            ..World::default()
        }))
    }

    fn open(
        w: &Shared,
        opts: SerialOptions,
        hook: Option<ReconnectHook>,
    ) -> Result<GattStream, Error> {
        let w = Arc::clone(w);
        GattStream::open(
            move || {
                Adapter::open(
                    FakeConnector(w),
                    &SnapshotOptions::default(),
                    Backoff {
                        initial: Duration::ZERO,
                        max: Duration::ZERO,
                        max_attempts: Some(3),
                    },
                    Box::new(|_| true),
                )
            },
            opts,
            hook,
        )
    }

    #[test]
    fn open_finds_the_pair_and_subscribes_to_the_notify_characteristic() {
        let w = world(P::WRITE);
        let _s = open(&w, SerialOptions::default(), None).unwrap();
        assert_eq!(w.lock().unwrap().log, ["sub 12"]);
    }

    #[test]
    fn a_profile_without_a_serial_pair_is_refused() {
        let w = world(P::WRITE);
        w.lock().unwrap().no_serial = true;
        assert!(matches!(
            open(&w, SerialOptions::default(), None),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn writes_are_chunked_to_the_mtu_in_order() {
        let w = world(P::WRITE);
        let mut s = open(&w, SerialOptions::default(), None).unwrap();
        s.write(&[7u8; 45]).unwrap();
        let log = w.lock().unwrap().log.clone();
        assert_eq!(
            &log[1..],
            [
                "write 15 20 WithResponse",
                "write 15 20 WithResponse",
                "write 15 5 WithResponse"
            ]
        );
    }

    #[test]
    fn the_write_kind_follows_the_declaration_and_the_preference() {
        let both = P::WRITE | P::WRITE_WITHOUT_RESPONSE;
        let w = world(both);
        let mut s = open(&w, SerialOptions::default(), None).unwrap();
        s.write(&[1]).unwrap();
        assert!(w.lock().unwrap().log[1].ends_with("WithResponse"));
        drop(s);

        let w = world(both);
        let mut s = open(
            &w,
            SerialOptions {
                prefer_without_response: true,
            },
            None,
        )
        .unwrap();
        s.write(&[1]).unwrap();
        assert!(w.lock().unwrap().log[1].ends_with("WithoutResponse"));
        drop(s);

        let w = world(P::WRITE_WITHOUT_RESPONSE);
        let mut s = open(&w, SerialOptions::default(), None).unwrap();
        s.write(&[1]).unwrap();
        assert!(w.lock().unwrap().log[1].ends_with("WithoutResponse"));
    }

    #[test]
    fn notifications_on_the_notify_characteristic_arrive_in_order_and_others_do_not() {
        let w = world(P::WRITE);
        let mut s = open(&w, SerialOptions::default(), None).unwrap();
        let rx = s.receiver().unwrap();
        assert!(s.receiver().is_none());
        let tx = w.lock().unwrap().notify.clone().unwrap();
        for (handle, value) in [(12, vec![1]), (3, vec![9]), (12, vec![2, 3])] {
            tx.send(ValueChange { handle, value }).unwrap();
        }
        let got: Vec<Vec<u8>> = (0..2)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(got, [vec![1], vec![2, 3]]);
        assert!(rx.recv_timeout(Duration::from_millis(250)).is_err());
    }

    #[test]
    fn a_dropped_link_reconnects_resubscribes_and_calls_the_hook() {
        let w = world(P::WRITE);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let hook: ReconnectHook = Box::new(move |n| sink.lock().unwrap().push(n));
        let mut s = open(&w, SerialOptions::default(), Some(hook)).unwrap();
        {
            let mut g = w.lock().unwrap();
            g.up = false;
            g.fail_connects = 1;
        }
        s.write(&[1, 2, 3]).unwrap();
        assert_eq!(*seen.lock().unwrap(), [1]);
        let log = w.lock().unwrap().log.clone();
        assert_eq!(log, ["sub 12", "sub 12", "write 15 3 WithResponse"]);
    }

    #[test]
    fn a_reconnect_that_gives_up_is_an_io_error_and_close_ends_the_stream() {
        let w = world(P::WRITE);
        let mut s = open(&w, SerialOptions::default(), None).unwrap();
        {
            let mut g = w.lock().unwrap();
            g.up = false;
            g.fail_connects = 99;
        }
        assert_eq!(s.write(&[1]), Err(StreamError::Io(ErrorKind::Other)));
        s.close();
        assert_eq!(s.write(&[1]), Err(StreamError::Closed));
    }
}
