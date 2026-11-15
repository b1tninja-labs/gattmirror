# gattmirror

Republish a real Bluetooth LE peripheral inside the Android emulator's virtual Bluetooth controller, so an
app in the emulator runs its own BLE code against a live adapter. A Cargo workspace; first-party code only.

Status: **early**. `hci-wire` (H4 framing, HCI packets, events) exists and is checked against a live
emulator. Everything else is planned: read [PLAN.md](PLAN.md), [ROADMAP.md](ROADMAP.md) and
[docs/research.md](docs/research.md). It does not yet advertise, connect or relay anything.

It is a transport. It will not parse OBD, ELM327 or UDS and issues no vehicle writes. It sits beside
`obd-bridge`, which solves the narrower case of an app with a Wi-Fi/TCP mode; see `volt/docs/AVD-OBD-BRIDGE.md`.

## Develop

    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo doc --no-deps

Live check against a running emulator (its `netsimd` serves H4 on a TCP port; find it with
`Get-NetTCPConnection -OwningProcess (Get-Process netsimd).Id`):

    GATTMIRROR_HCI=127.0.0.1:<port> cargo test -- --test-threads=1

Minimum supported Rust version: 1.85 (`docs/msrv.md`). Contributing: `CONTRIBUTING.md`.
