# Research: putting a real BLE adapter inside the Android emulator

Measured 2026-11-14 against Android emulator 36.4.9.0 on Windows 11. Nothing here touched a vehicle or a
physical phone.

## The problem

The emulator has a virtual Bluetooth controller but no path to the host's radio. An app that talks to a BLE
OBD adapter (the Voltage app in `volt` is the motivating case) therefore sees no adapter. `obd-bridge` solves
the narrow case by republishing a BLE ELM327 adapter as ELM327-over-TCP, which works only for an app that has
a Wi-Fi/TCP mode. `gattmirror` solves the general case: **the emulated phone sees a BLE peripheral that is a
live mirror of a real one**, so the app's own BLE code path runs unmodified.

## What the emulator offers (all found on disk or measured)

| Seam | Evidence | What it would take |
|---|---|---|
| **HCI over TCP (H4) to the virtual controller** | `netsimd.exe --help` lists `--hci-port`; on the test machine a listener was open on `127.0.0.1:6402` owned by `netsimd`; a plain TCP client received valid HCI events (below) | A host-side BLE *peripheral* stack: HCI init, advertising, connection, L2CAP LE, ATT/GATT server. No gRPC. |
| `EmulatedBluetoothService.registerGattDevice` (gRPC) | `emulator/lib/emulated_bluetooth.proto` and `emulated_bluetooth_device.proto`: a client registers a `GattDevice` (advertisement, GATT profile, callback endpoint) and serves `GattDeviceService` read/write/observe callbacks | A gRPC client and server (HTTP/2, HPACK, protobuf) and the emulator's gRPC endpoint (`127.0.0.1:8554` was open). The emulator then runs the GATT server for us. **Not tried**: the auth/token requirement of the 8554 endpoint is unknown. |
| USB passthrough of a host Bluetooth dongle | QEMU `usb-host` | Driver ownership problems on Windows; not a seam this project controls. |
| Forking the emulator | AOSP `external/qemu` | Not needed; the first two seams exist. |

**Decision: the H4 seam first.** It needs no gRPC stack (which this portfolio would have to write itself under
its self-sufficiency rule: HTTP/2, HPACK, protobuf, gRPC framing), it exercises the guest's real GATT client
against a real ATT server, and its wire format is simple. The gRPC route stays recorded as the fallback if the
HCI route proves unreliable in the guest.

## Measured behaviour of the H4 socket

Sent as raw bytes to the HCI port; replies decoded by hand, then by `hci-wire`:

- No greeting: the controller says nothing until spoken to.
- Reset (`01 03 0c 00`) -> `04 0e 04 01 03 0c 00` (Command Complete, status 0).
- Read Local Version, Read BD_ADDR and LE Read Buffer Size each answer with a Command Complete, status 0.
  LE Read Buffer Size returned an ACL length of 27 and a count of 20. The version reply reports a Bluetooth 5.x
  controller.
- **A command sent before any Reset is answered with `04 10 01 42`** (event `0x10`, one parameter `0x42`),
  not a Command Complete. A host must Reset first. This is observed, not specified.
- LE Set Advertising Enable (disable) is accepted with status 0, so the LE command group is live.
- The HCI port number is per-`netsimd`-instance; it is not always 6402.
- Each TCP connection is its own virtual controller with its own address, so a mirror is simply another
  simulated device on the emulator's virtual air.

## What the project must build, in dependency order

1. **`hci-wire`** (started): H4 framing and a typed view of commands and events.
2. **`att-server`**: ATT PDU codec and a GATT attribute database: services, characteristics, descriptors,
   the Client Characteristic Configuration descriptor, MTU exchange, discovery requests, read/write/notify.
3. **`le-host`**: the LE peripheral host over HCI: initialise the controller, set advertising data and
   parameters, advertise, accept an LE connection, fragment and reassemble ACL data into L2CAP SDUs on the
   ATT fixed channel, answer or refuse the other fixed channels (signalling, security) politely, and honour
   the controller's buffer credits.
4. **`gatt-profile`**: a plain-text snapshot of a peripheral's GATT table (services, characteristics with
   properties, descriptors) that both sides use, so the virtual device can be built from a file, from a real
   adapter, or from a built-in fixture.
5. **`winrt-gatt`**: the *client* toward the real adapter on Windows: discover the full GATT table, read,
   write with or without response, subscribe to notifications. `obd-bridge/crates/obd-bridge/src/ble.rs` is
   the working precedent (first-party `windows` crate, no async runtime); this crate generalises it from "the
   serial service" to "any table".
6. **`gattmirror`**: the binary. Connect to the real adapter, snapshot its profile, bring up the virtual
   peripheral with the same name, services and characteristics, and relay: guest write -> real write;
   real notification -> guest notification; guest read -> real read (cached where the real one is static).

Also wanted: a **simulated adapter** (the `volt/tools/fake_elm327.py` behaviour behind an FFF0 profile) so the
whole path from guest to mirror can be verified with no hardware.

## What cannot be decided yet

- **No specification text.** `primarysource` holds RFCs, ITU, NIST and USB-IF documents but no Bluetooth
  Core Specification. The HCI opcodes and event layouts in `hci-wire` were verified against a live controller,
  not read, and are `[unread]` in the ROADMAP. ATT, L2CAP and GAP work should not be written from memory:
  Wave 0 is to get the Core Specification (Vol 2 Part E HCI, Vol 3 Parts A, C, F and G) into a
  `primarysource` collection. The Bluetooth SIG publishes it free of charge under a licence the owner must
  accept, so fetching it is a human step.
- Whether the guest's GATT client will complete service discovery and subscribe against a Rootcanal-simulated
  peripheral. Untested; it is the first end-to-end gate (ROADMAP root rows).
- Link-layer parameters (connection interval, supervision timeout) the guest expects from an OBD adapter.
- Whether Windows lets an application act as a GATT *client* to the real adapter while that adapter is also
  what the guest app expects to pair with. The mirror never exposes the real adapter's address, so this
  should be fine; it is untested.

## Safety and scope

The mirror is a transport: it forwards attribute reads, writes and notifications between two BLE endpoints
and implements no OBD or UDS behaviour and no vehicle-write logic. A mirrored adapter is exactly as
unauthenticated as the real one; the virtual peripheral exists only inside the emulator's simulated air and
the H4 socket binds loopback (`netsimd` decides that, not this project). No Bluetooth addresses, VINs or other
identifying data go into the tree.
