# gattmirror plan

Goal: `gattmirror --name <substring>` finds a real BLE peripheral on the host, copies its GATT table, and
appears under the same name and table inside the emulator, relaying reads, writes and notifications.

## Crates

```
hci-wire ──> le-host ──┐
att-server ─┬──────────┼──> gattmirror (binary)
gatt-profile┴──────────┤
winrt-gatt ────────────┘
```

| Crate | Depends on | Owns | Out of scope |
|---|---|---|---|
| `hci-wire` | none | H4 framing, HCI packets, event view | any host behaviour |
| `att-server` | none | ATT PDU codec; GATT attribute database; request handling; MTU; CCCD and notify | transport, HCI |
| `le-host` | `hci-wire`, `att-server` | controller init, advertising, connection, ACL <-> L2CAP, buffer credits, refusing SMP | GATT content |
| `gatt-profile` | none | text format and model for a GATT table snapshot | I/O |
| `winrt-gatt` | `gatt-profile` | Windows GATT client: discover, read, write, subscribe (first-party `windows` crate) | the emulator side |
| `gattmirror` | all of the above | CLI, snapshot-then-serve, the relay loop, a simulated-adapter mode | OBD semantics |

`att-server` and `gatt-profile` have no I/O, so they test anywhere. `winrt-gatt` is Windows-only behind
`cfg(windows)` like `obd-bridge`'s `ble.rs`, and the workspace builds elsewhere without it.

## Waves

0. **Ground truth.** Get the Bluetooth Core Specification into `primarysource` (a human step; the SIG licence).
   Until then every Bluetooth row is `[unread]` and verified against a live controller.
1. **Air presence (E1).** `le-host` resets the controller, sets advertising data and parameters and advertises
   a fixed name; the emulated phone's scan lists it. Needs `hci-wire` plus a few more commands.
2. **GATT (E2, E3).** `att-server` and the connection path in `le-host`: the guest connects, discovers a table
   from a `gatt-profile` fixture, writes and receives notifications from a simulated adapter.
3. **App (E4).** The Voltage app in BLE mode selects the mirrored device and runs its init against the
   simulated adapter.
4. **Real adapter (E5).** `winrt-gatt` and the relay; needs a bench or a vehicle.

## Rules

- First-party dependencies only: `std`, and Microsoft's `windows` crate for the Windows client.
- No OBD/UDS parsing, no vehicle-write logic, no Bluetooth addresses or identifiers in the tree.
- Read the standard before citing it; until one is read, say `[unread]` and verify live.
