# gattmirror roadmap

`gattmirror` republishes a real Bluetooth LE peripheral inside the Android emulator's virtual Bluetooth
controller, so an app running in the emulator uses its own BLE code against a live adapter. Why this approach
and what was measured: [docs/research.md](docs/research.md). How the crates fit together: [PLAN.md](PLAN.md).

Rows are `Status | Obligation | Anchor | Test`, the grammar `projecthub sync` reads for Rust projects. Each crate
keeps its own rows in `crates/<name>/ROADMAP.md`; this file holds the rows that span them.

Status:

- `[ok]` the named test exists, passes, and failed when the code it covers was deliberately broken.
- `[unread]` implemented and a test passes, but the anchor is not a standard that has been read (here: no
  Bluetooth Core Specification text is available, so behaviour was verified against a live controller only).
- `[open]` not proven, not done, or needs hardware or a human decision.

## Wave 0: ground truth

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [open] | The Bluetooth Core Specification (Vol 2 Part E, Vol 3 Parts A, C, F, G) is available as a `primarysource` collection so every ATT, L2CAP and HCI row can cite a section. Needs the owner to accept the SIG licence and fetch it | Bluetooth Core Specification | none: a document gate, not code |
| [ok] | The emulator's H4 socket answers a host that sends Reset first, and answers a command sent before Reset with event `0x10` | observed, `docs/research.md` | `crates/hci-wire/tests/live_controller.rs`; see `crates/hci-wire/ROADMAP.md` (needs `GATTMIRROR_HCI`, not run in CI) |

## Spanning gates (need more than one crate)

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [open] | **E1.** The emulated phone's BLE scan lists a mirrored device by the real adapter's name | Bluetooth Core Specification (advertising); unread | none yet |
| [open] | **E2.** The guest completes GATT service discovery against the virtual peripheral and sees the mirrored table | Bluetooth Core Specification Vol 3 Parts F, G; unread | none yet |
| [open] | **E3.** A guest write to the mirrored write characteristic reaches a simulated adapter and its notification returns to the guest (no hardware) | none: end-to-end, project-defined | none yet |
| [open] | **E4.** The Voltage app in BLE mode selects the mirrored adapter and runs its ELM327 init against the simulated adapter | none: end-to-end | none yet; needs the emulator and the app |
| [open] | **E5.** The same against a real BLE adapter on a bench or in a vehicle | none | none: needs hardware |
| [open] | The mirror relays attribute traffic only: no OBD, ELM327 or UDS parsing anywhere in the workspace | scope rule, `AGENTS.md` | none yet: a dependency/grep check to add |

## Crates

| Crate | Role | Rows |
|---|---|---|
| `hci-wire` | H4 framing, HCI packets, typed events | [crates/hci-wire/ROADMAP.md](crates/hci-wire/ROADMAP.md) |
| `att-server` | ATT PDUs and the GATT attribute database | not started |
| `le-host` | LE peripheral host over HCI: advertise, connect, L2CAP | not started |
| `gatt-profile` | text snapshot of a GATT table | not started |
| `winrt-gatt` | Windows GATT client toward the real adapter | not started |
| `gattmirror` | the relay binary | not started |

A crate gets a ROADMAP file when work on it starts, not before: an empty table is a promise nobody keeps.

## Not built, and why

- No OBD, ELM327 or UDS logic, and nothing that writes to a vehicle, other than forwarding an app's own
  bytes: this is transport. Scope as in `volt/docs/OBD-CONFIG-SCOPE.md`.
- No gRPC route (`registerGattDevice`) until the HCI route has been shown to fail: it would make this
  portfolio write HTTP/2, HPACK, protobuf and gRPC first. Recorded in `docs/research.md`.
- No Bluetooth classic (SPP). The Voltage app's *Bluetooth* mode is classic; only BLE is mirrored.
