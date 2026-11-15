# winrt-gatt roadmap

A Windows GATT client: scan, connect, discover a full table into plain data, read, write, subscribe. The
Windows code (`src/winrt.rs`, `cfg(windows)`) compiles and passes clippy but has **never been run against a
real adapter**; every row that needs one is `[open]`. The Bluetooth Core Specification is not available, so
anchors are the Windows API's documented behaviour as used by `obd-bridge/crates/obd-bridge/src/ble.rs`
(a working precedent), or none. Break notes say what was changed to make the named test fail.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | A 16- or 32-bit UUID expands onto the Bluetooth base and prints as canonical `8-4-4-4-12`; parsing accepts short, `0x`, braced, dashed and bare-hex forms and rejects malformed text | base value as reported by Windows stacks; unread | `src/uuid.rs::short_form_expands_onto_the_base`, `parse_accepts_every_documented_form`, `parse_rejects_malformed_text`, `short_forms_round_trip_only_on_the_base`; broken by shifting the base by 95 (3 failed) |
| [ok] | Property bits match the declaration bits; subscribe and write predicates are correct | Windows `GattCharacteristicProperties` flag values; unread | `src/properties.rs::subscribe_and_write_predicates`, `names_follow_bit_order`; broken by dropping Indicate from `can_subscribe` |
| [ok] | The table model finds characteristics and descriptors by handle and by type, scoped by service, and reports duplicate handles | none: project-defined | `src/table.rs::lookup_by_handle_and_type`, `find_is_scoped_by_service`, `duplicate_handles_are_reported`; broken by ignoring the service scope |
| [ok] | Addresses format as `AA:BB:CC:DD:EE:FF` and parse from colon, dash and bare forms | none: text convention | `src/address.rs::round_trip`, `rejects_malformed`; broken by swapping two octets |
| [ok] | A scan filter matches by case-insensitive name substring, exact address, or both; an empty filter is rejected so a scan never grabs an arbitrary device | none: project-defined | `src/filter.rs` four tests; broken by making the name match case-sensitive |
| [ok] | Max write payload is `MTU - 3`, never zero; chunking covers the input in order with no empty tail | ATT header size as `obd-bridge` assumes; unread | `src/chunk.rs` five tests; broken by changing the header size to 2 |
| [ok] | The connection state machine reaches Ready only through scan, found, connected, discovered; a drop goes to back-off; Close works anywhere; Closed is terminal; skipped steps are refused | none: project-defined | `src/link.rs::happy_path_reaches_ready`, `a_drop_goes_to_backoff_then_rescans`, `nothing_leaves_closed_and_close_works_anywhere`, `skipping_a_step_is_refused`; broken by a wrong Discovered target and by limiting Close to Idle |
| [ok] | Reconnect back-off doubles to a ceiling, gives up at the attempt limit, and `retry` stops on success, reports the last error, and stops when the sleeper declines | none: project-defined | `src/link.rs::backoff_doubles_and_caps`, `backoff_gives_up_at_the_limit`, `retry_*`; broken by removing the ceiling and by ignoring the sleeper's answer |
| [ok] | The Windows module builds under `cfg(windows)` and is excluded elsewhere, with no `unsafe` | `Cargo.toml` lint table | `cargo clippy --all-targets -- -D warnings` on Windows (the non-Windows build was not run) |
| [open] | `scan` finds an advertiser by name substring or address on a live radio | Windows `BluetoothLEAdvertisementWatcher`; needs an adapter | none: needs hardware |
| [open] | `connect` discovers every service, characteristic and descriptor of a real peripheral, with unique handles matching what the peripheral assigned | Windows `GetGattServicesWithCacheModeAsync`; needs a peripheral | none: needs hardware |
| [open] | `read`, `read_descriptor`, `write` (both modes) and `write_chunked` succeed against a real peripheral; a long write with response is handled by the platform | needs a peripheral | none: needs hardware |
| [open] | `subscribe` enables notify (or indicate) and delivers values on the channel in arrival order; `unsubscribe` stops them | needs a peripheral | none: needs hardware |
| [open] | `mtu` reports the negotiated ATT MTU via `GattSession.MaxPduSize` | needs a peripheral | none: needs hardware |
| [open] | `on_disconnect` fires when the link drops and `is_connected` follows it | needs a peripheral | none: needs hardware |
| [open] | The state machine is driven by the client (today `link` is standalone logic and `retry` a helper; the client does not own a `State`) | none | none yet |
| [open] | Included-service declarations are captured in the table | not read | none yet: not implemented |
| [open] | Attribute handles reported by WinRT equal the peripheral's real handles (a mirror may depend on this) | unread | none: needs hardware |
