//! The Windows implementation, over `WinRT` `Windows.Devices.Bluetooth`.
//!
//! `WinRT` async operations are driven with `IAsyncOperation::get()` on the calling thread, so no async
//! runtime is needed. Like `obd-bridge`, this relies on the implicit MTA: no explicit COM
//! initialisation (and so no `unsafe`). Nothing here has been run against a real adapter.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
    GattCommunicationStatus, GattDescriptor, GattDeviceService, GattSession,
    GattValueChangedEventArgs, GattWriteOption,
};
use windows::Devices::Bluetooth::{
    BluetoothCacheMode, BluetoothConnectionStatus, BluetoothLEDevice,
};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};
use windows::core::GUID;

use crate::chunk::{self, DEFAULT_MTU};
use crate::error::Error;
use crate::filter::ScanFilter;
use crate::properties::Properties;
use crate::table::{Characteristic, Descriptor, GattTable, Service};
use crate::uuid::Uuid;
use crate::{ValueChange, WriteMode};

fn to_uuid(g: GUID) -> Uuid {
    Uuid(g.to_u128())
}

fn status_ok(status: GattCommunicationStatus, what: &str) -> Result<(), Error> {
    if status == GattCommunicationStatus::Success {
        Ok(())
    } else {
        Err(Error::Status(format!("{what}: {status:?}")))
    }
}

fn buffer_to_vec(buf: &IBuffer) -> Result<Vec<u8>, Error> {
    let len = buf.Length()? as usize;
    let mut bytes = vec![0u8; len];
    if len > 0 {
        DataReader::FromBuffer(buf)?.ReadBytes(&mut bytes)?;
    }
    Ok(bytes)
}

fn vec_to_buffer(data: &[u8]) -> Result<IBuffer, Error> {
    let w = DataWriter::new()?;
    w.WriteBytes(data)?;
    Ok(w.DetachBuffer()?)
}

/// An advertiser a scan matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// 48-bit address as the Windows API reports it.
    pub address: u64,
    /// Advertised local name; empty if the matching packet carried none.
    pub name: String,
}

/// Scan until an advertiser matches `filter`, or `timeout` passes.
///
/// # Errors
/// [`Error::InvalidArgument`] for an empty filter, [`Error::ScanTimeout`] if nothing matched, or a
/// platform error if the watcher cannot start.
pub fn scan(filter: &ScanFilter, timeout: Duration) -> Result<Found, Error> {
    filter.validate()?;
    let watcher = BluetoothLEAdvertisementWatcher::new()?;
    watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;

    let (tx, rx) = mpsc::channel::<Found>();
    let tx = Mutex::new(tx);
    let f = filter.clone();
    let handler = TypedEventHandler::new(
        move |_s, args: &Option<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(args) = args.as_ref() {
                let address = args.BluetoothAddress()?;
                let name = args
                    .Advertisement()
                    .and_then(|a| a.LocalName())
                    .map(|h| h.to_string())
                    .unwrap_or_default();
                if f.matches(&name, address) {
                    if let Ok(tx) = tx.lock() {
                        let _ = tx.send(Found { address, name });
                    }
                }
            }
            Ok(())
        },
    );
    let token = watcher.Received(&handler)?;
    watcher.Start()?;
    let result = rx.recv_timeout(timeout);
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);
    result.map_err(|_| Error::ScanTimeout)
}

/// Options for [`WinrtClient::connect`].
#[derive(Clone, Default)]
pub struct ConnectOptions {
    /// Called on a platform thread when the link drops. Keep it short and do not block: hand off to a
    /// supervisor (see [`crate::link::retry`]) instead of reconnecting inside it.
    pub on_disconnect: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// A connected peripheral with its discovered table.
///
/// Notifications arrive on the [`Receiver`] returned by [`connect`](Self::connect); it is kept out of
/// the struct so that a shared reference can be used from several threads.
pub struct WinrtClient {
    table: GattTable,
    chars: HashMap<u16, GattCharacteristic>,
    descs: HashMap<u16, GattDescriptor>,
    subs: Mutex<HashMap<u16, EventRegistrationToken>>,
    sender: Arc<Mutex<Sender<ValueChange>>>,
    connected: Arc<AtomicBool>,
    status_token: EventRegistrationToken,
    session: Option<GattSession>,
    // Held so the services, and with them the GATT connection, stay open.
    _services: Vec<GattDeviceService>,
    device: BluetoothLEDevice,
}

impl WinrtClient {
    /// Connect to `address`, discover every service, characteristic and descriptor, and return the
    /// client plus the channel notifications are delivered on.
    ///
    /// # Errors
    /// A platform or GATT status error from any step.
    pub fn connect(
        address: u64,
        opts: &ConnectOptions,
    ) -> Result<(Self, Receiver<ValueChange>), Error> {
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;

        let connected = Arc::new(AtomicBool::new(
            device.ConnectionStatus()? == BluetoothConnectionStatus::Connected,
        ));
        let flag = Arc::clone(&connected);
        let hook = opts.on_disconnect.clone();
        let status_handler = TypedEventHandler::new(move |dev: &Option<BluetoothLEDevice>, _| {
            if let Some(dev) = dev.as_ref() {
                let up = dev.ConnectionStatus()? == BluetoothConnectionStatus::Connected;
                flag.store(up, Ordering::SeqCst);
                if !up {
                    if let Some(h) = &hook {
                        h();
                    }
                }
            }
            Ok(())
        });
        let status_token = device.ConnectionStatusChanged(&status_handler)?;

        // Keep the link open between operations; failure to get a session only costs MTU reporting.
        let session = device
            .BluetoothDeviceId()
            .and_then(|id| GattSession::FromDeviceIdAsync(&id))
            .and_then(|op| op.get())
            .ok();
        if let Some(s) = &session {
            let _ = s.SetMaintainConnection(true);
        }

        let services_result = device
            .GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)?
            .get()?;
        status_ok(services_result.Status()?, "service discovery")?;
        let service_list = services_result.Services()?;

        let mut table = GattTable::default();
        let mut chars = HashMap::new();
        let mut descs = HashMap::new();
        let mut services = Vec::new();
        for service in &service_list {
            let mut svc = Service {
                handle: service.AttributeHandle()?,
                uuid: to_uuid(service.Uuid()?),
                characteristics: Vec::new(),
            };
            let cres = service
                .GetCharacteristicsWithCacheModeAsync(BluetoothCacheMode::Uncached)?
                .get()?;
            status_ok(cres.Status()?, "characteristic discovery")?;
            for ch in &cres.Characteristics()? {
                let mut c = Characteristic {
                    handle: ch.AttributeHandle()?,
                    uuid: to_uuid(ch.Uuid()?),
                    // Low 16 bits of the flag set; the flag values match `Properties`.
                    properties: Properties(
                        u16::try_from(ch.CharacteristicProperties()?.0 & 0xFFFF).unwrap_or(0),
                    ),
                    descriptors: Vec::new(),
                };
                let dres = ch
                    .GetDescriptorsWithCacheModeAsync(BluetoothCacheMode::Uncached)?
                    .get()?;
                status_ok(dres.Status()?, "descriptor discovery")?;
                for d in &dres.Descriptors()? {
                    let handle = d.AttributeHandle()?;
                    c.descriptors.push(Descriptor {
                        handle,
                        uuid: to_uuid(d.Uuid()?),
                    });
                    descs.insert(handle, d);
                }
                chars.insert(c.handle, ch);
                svc.characteristics.push(c);
            }
            table.services.push(svc);
            services.push(service);
        }
        table.check_unique_handles()?;

        let (tx, rx) = mpsc::channel();
        Ok((
            Self {
                table,
                chars,
                descs,
                subs: Mutex::new(HashMap::new()),
                sender: Arc::new(Mutex::new(tx)),
                connected,
                status_token,
                session,
                _services: services,
                device,
            },
            rx,
        ))
    }

    /// The discovered table.
    #[must_use]
    pub const fn table(&self) -> &GattTable {
        &self.table
    }

    /// Whether the link is currently up, per the last connection-status event.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// The ATT MTU the platform reports, or the default if no session could be opened.
    #[must_use]
    pub fn mtu(&self) -> u16 {
        self.session
            .as_ref()
            .and_then(|s| s.MaxPduSize().ok())
            .filter(|m| *m >= DEFAULT_MTU)
            .unwrap_or(DEFAULT_MTU)
    }

    /// The largest value that fits one write at the current MTU.
    #[must_use]
    pub fn max_payload(&self) -> usize {
        chunk::max_payload(self.mtu())
    }

    fn characteristic(&self, handle: u16) -> Result<&GattCharacteristic, Error> {
        self.chars.get(&handle).ok_or(Error::UnknownHandle(handle))
    }

    /// Read a characteristic from the peripheral (not the platform cache).
    ///
    /// # Errors
    /// Unknown handle, a characteristic that cannot be read, or a platform or status error.
    pub fn read(&self, handle: u16) -> Result<Vec<u8>, Error> {
        let ch = self.characteristic(handle)?;
        let r = ch
            .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
            .get()?;
        status_ok(r.Status()?, "read")?;
        buffer_to_vec(&r.Value()?)
    }

    /// Read a descriptor value.
    ///
    /// # Errors
    /// Unknown handle, or a platform or status error.
    pub fn read_descriptor(&self, handle: u16) -> Result<Vec<u8>, Error> {
        let d = self
            .descs
            .get(&handle)
            .ok_or(Error::UnknownHandle(handle))?;
        let r = d
            .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
            .get()?;
        status_ok(r.Status()?, "descriptor read")?;
        buffer_to_vec(&r.Value()?)
    }

    /// Write one value in a single operation. With [`WriteMode::WithResponse`] the platform performs
    /// a long write itself if the value exceeds the MTU; that behaviour is unverified.
    ///
    /// # Errors
    /// Unknown handle, a mode the characteristic does not declare, or a platform or status error.
    pub fn write(&self, handle: u16, data: &[u8], mode: WriteMode) -> Result<(), Error> {
        let ch = self.characteristic(handle)?;
        let props = self
            .table
            .characteristic(handle)
            .ok_or(Error::UnknownHandle(handle))?
            .properties;
        let (needed, option) = match mode {
            WriteMode::WithResponse => (Properties::WRITE, GattWriteOption::WriteWithResponse),
            WriteMode::WithoutResponse => (
                Properties::WRITE_WITHOUT_RESPONSE,
                GattWriteOption::WriteWithoutResponse,
            ),
        };
        if !props.has(needed) {
            return Err(Error::Unsupported(
                "characteristic does not declare this write mode",
            ));
        }
        let status = ch
            .WriteValueWithOptionAsync(&vec_to_buffer(data)?, option)?
            .get()?;
        status_ok(status, "write")
    }

    /// Write `data` as a sequence of writes of at most [`max_payload`](Self::max_payload) bytes each,
    /// stopping at the first failure.
    ///
    /// # Errors
    /// As [`write`](Self::write).
    pub fn write_chunked(&self, handle: u16, data: &[u8], mode: WriteMode) -> Result<(), Error> {
        for piece in chunk::chunks(data, self.max_payload()) {
            self.write(handle, piece, mode)?;
        }
        Ok(())
    }

    /// Subscribe to a characteristic. Notify is preferred over indicate when both are declared.
    ///
    /// # Errors
    /// Unknown handle, a characteristic that declares neither, or a platform or status error.
    pub fn subscribe(&self, handle: u16) -> Result<(), Error> {
        let ch = self.characteristic(handle)?;
        let props = self
            .table
            .characteristic(handle)
            .ok_or(Error::UnknownHandle(handle))?
            .properties;
        let cccd = if props.has(Properties::NOTIFY) {
            GattClientCharacteristicConfigurationDescriptorValue::Notify
        } else if props.has(Properties::INDICATE) {
            GattClientCharacteristicConfigurationDescriptorValue::Indicate
        } else {
            return Err(Error::Unsupported(
                "characteristic has neither notify nor indicate",
            ));
        };
        let mut subs = self
            .subs
            .lock()
            .map_err(|_| Error::Platform("poisoned lock".into()))?;
        if subs.contains_key(&handle) {
            return Ok(());
        }
        let tx = Arc::clone(&self.sender);
        let handler =
            TypedEventHandler::new(move |_c, args: &Option<GattValueChangedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    let value = buffer_to_vec(&args.CharacteristicValue()?)
                        .map_err(|_| windows::core::Error::from(windows::core::HRESULT(-1)))?;
                    if let Ok(tx) = tx.lock() {
                        let _ = tx.send(ValueChange { handle, value });
                    }
                }
                Ok(())
            });
        let token = ch.ValueChanged(&handler)?;
        let status = ch
            .WriteClientCharacteristicConfigurationDescriptorAsync(cccd)?
            .get()?;
        if let Err(e) = status_ok(status, "enable notifications") {
            let _ = ch.RemoveValueChanged(token);
            return Err(e);
        }
        subs.insert(handle, token);
        Ok(())
    }

    /// Stop notifications for a characteristic. A no-op if not subscribed.
    ///
    /// # Errors
    /// Unknown handle, or a platform or status error.
    pub fn unsubscribe(&self, handle: u16) -> Result<(), Error> {
        let ch = self.characteristic(handle)?;
        let token = self
            .subs
            .lock()
            .map_err(|_| Error::Platform("poisoned lock".into()))?
            .remove(&handle);
        if let Some(token) = token {
            let _ = ch.RemoveValueChanged(token);
            let status = ch
                .WriteClientCharacteristicConfigurationDescriptorAsync(
                    GattClientCharacteristicConfigurationDescriptorValue::None,
                )?
                .get()?;
            status_ok(status, "disable notifications")?;
        }
        Ok(())
    }
}

impl Drop for WinrtClient {
    fn drop(&mut self) {
        if let Ok(subs) = self.subs.lock() {
            for (handle, token) in subs.iter() {
                if let Some(ch) = self.chars.get(handle) {
                    let _ = ch.RemoveValueChanged(*token);
                }
            }
        }
        let _ = self.device.RemoveConnectionStatusChanged(self.status_token);
    }
}
