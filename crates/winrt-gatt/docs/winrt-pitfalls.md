# WinRT GATT client: what Microsoft's documentation says, and what it leaves open

Read from Microsoft Learn; "not documented" means the fetched pages are silent, so a live controller must
settle it. The Bluetooth Core Specification is not available, so nothing here cites it. Base for the API pages:
<https://learn.microsoft.com/en-us/uwp/api/> (shortened `API/`).

## Session and MTU
- `GattSession.MaintainConnection` defaults to false; true makes the system wait indefinitely for the device
  and connect when it is available (`API/windows.devices.bluetooth.genericattributeprofile.gattsession.maintainconnection`).
- `MaxPduSize` is documented as the PDU size, "also known as the MTU"; `MaxPduSizeChanged` fires when the
  negotiated ATT MTU changes (`API/...gattsession.maxpdusize`). Not documented: whether the value is the MTU
  or the payload (MTU - 3), and whether it is valid before the link is up. `winrt-gatt` treats it as the MTU
  and clamps to the 23-byte default.
- Disposing every reference to a `BluetoothLEDevice` triggers a disconnect after a small timeout
  (<https://learn.microsoft.com/en-us/windows/uwp/devices-sensors/gatt-client>). Whether open
  `GattDeviceService` objects alone hold the link is not documented; the client holds both and sets
  `MaintainConnection`.

## Writes and properties
- `GattWriteOption`: `WriteWithResponse` = 0, `WriteWithoutResponse` = 1 (`API/...gattwriteoption`). The
  platform fragments data larger than the MTU "wherever supported" (`API/...gattsession.maxpdusize`
  remarks). Not documented: which procedure a long write uses, flow control of write-without-response, and
  writes above MTU - 3 without response. `write_chunked` splits at MTU - 3 itself.
- `GattCharacteristicProperties` values: Broadcast 1, Read 2, WriteWithoutResponse 4, Write 8, Notify 16,
  Indicate 32, AuthenticatedSignedWrites 64, ExtendedProperties 128, ReliableWrites 256, WritableAuxiliaries
  512 (`API/...gattcharacteristicproperties`). The two high flags have no declaration bit; the snapshot maps
  them to the Extended bit.

## Notifications
- `WriteClientCharacteristicConfigurationDescriptorAsync` enables events only if a `ValueChanged` handler
  is registered (`API/...gattcharacteristic.writeclientcharacteristicconfigurationdescriptorasync`). The
  Microsoft sample writes the CCCD first, then registers; `winrt-gatt` registers first so no early value is
  lost. Whether early values are buffered or dropped is not documented.
- Statuses: Success 0, Unreachable 1, ProtocolError 2, AccessDenied 3 (`API/...gattcommunicationstatus`).
- Not documented: the thread `ValueChanged` and `ConnectionStatusChanged` handlers run on, whether the
  handler must be kept alive (the client keeps it registered for the subscription's life), what the OS
  does with the CCCD across reconnects. The adapter re-subscribes after every reconnect.

## Cache, pairing, connection
- `BluetoothCacheMode.Cached` tries the system-wide cache first; `Uncached` reads the device
  (`API/windows.devices.bluetooth.bluetoothcachemode`). The client always asks for `Uncached`.
- Creating a `BluetoothLEDevice` does not necessarily connect; an uncached discovery, a read or write, or
  `MaintainConnection` does. Operations wait "a finite but variable time", an unreachable device with nothing
  queued fails after about 7 s, each queued request can take that long again, and the connection process
  cannot be cancelled (`API/windows.devices.bluetooth.bluetoothledevice.frombluetoothaddressasync`, the GATT
  client guide above). The default bound per operation is therefore 60 s.
- `FromBluetoothAddressAsync` returns null for a device that is neither paired nor in the system cache;
  a scan populates the cache first. `Target::Address` skips the scan, so it can fail where `Target::Filter`
  works.
- Whether GATT needs pairing or bonding is not documented on these pages; `ProtectionLevel` says Windows
  negotiates the maximum security possible.

## Threads
- C++/WinRT `get()` is not appropriate on a UI thread
  (<https://learn.microsoft.com/en-us/windows/uwp/cpp-and-winrt-apis/concurrency>). The client polls
  `IAsyncOperation::Status` and calls `GetResults` only on completion, so it does not use `get()`. It
  still must not run on an STA thread that has to pump messages. The implicit MTA of a plain thread is
  not documented on the fetched pages.
