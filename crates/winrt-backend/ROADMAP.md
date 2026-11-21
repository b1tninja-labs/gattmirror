# winrt-backend roadmap

`gattmirror::Backend` over `winrt-gatt`'s reconnecting adapter. Transport only. The Windows constructor
(`connect`) is compile-checked and has never been run. The only Bluetooth fact it relies on, a write or notification
carrying at most ATT_MTU-3 value octets, is Core Specification 7.0 Vol 3 Part F §3.2.9 and §3.4.7.1 (also cited in
the `winrt-gatt` roadmap). Depends on the `Backend` trait from `crates/gattmirror`.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | Profile and `CharId`s follow the discovered table: ids count characteristics across the whole profile in order | `gattmirror::characteristics` | `src/lib.rs::profile_and_char_ids_follow_the_table` (fake connector) |
| [ok] | Reads and writes by `CharId` reach the matching peripheral handle, with response or without | none: project-defined | `reads_and_writes_reach_the_right_handle`; broken by mapping the wrong handle in the adapter (see `winrt-gatt` ROADMAP) |
| [ok] | Refusals map to `NotReadable`, `NotWritable` and `Failed` | `gattmirror::BackendError` | `refusals_map_to_the_mirrors_errors`; broken by swapping the read and write mappings |
| [ok] | Notifications reach the mirror as `Notification` by `CharId`; values for unknown handles are dropped; the receiver is yielded once | none: project-defined | `notifications_arrive_by_char_id_and_unknown_handles_are_dropped`; broken by mapping every handle to index 0 |
| [open] | `connect` finds a real peripheral, snapshots it and relays reads, writes and notifications; CCCDs are dropped from the profile because the mirror's ATT server adds its own | needs hardware | none: needs hardware |
| [open] | A notification forwarding thread per backend ends only when the backend's sender side or the receiver is dropped; a stalled mirror lets values queue unboundedly | none | none yet |
| [ok] | `GattStream` is a `ByteStream` over a GATT serial pair: it finds the first writable and first notifying characteristic with `gattmirror::serial::serial_pair`, subscribes to the notify one, and refuses a profile without both | `serial_stream::ByteStream`; none for the pair rule beyond gattmirror's | `src/stream.rs::open_finds_the_pair_and_subscribes_to_the_notify_characteristic`, `a_profile_without_a_serial_pair_is_refused`; broken by swapping write and notify (6 failed) |
| [ok] | Notifications on the notify characteristic reach the stream's receiver in order; other characteristics' values do not; the receiver is yielded once | none: project-defined | `notifications_on_the_notify_characteristic_arrive_in_order_and_others_do_not`; broken by dropping the handle filter |
| [ok] | Writes are split to the transport's payload in order; the write kind follows the declaration, and `prefer_without_response` only chooses when both are declared | none: project-defined | `writes_are_chunked_to_the_mtu_in_order`, `the_write_kind_follows_the_declaration_and_the_preference`; broken by ignoring the preference and (in `winrt-gatt`) by not chunking |
| [ok] | A reconnect re-subscribes, calls the `ReconnectHook` with the running count, and the write then succeeds; a reconnect that gives up is an I/O error; after `close` writes fail with `Closed` | none: project-defined | `a_dropped_link_reconnects_resubscribes_and_calls_the_hook`, `a_reconnect_that_gives_up_is_an_io_error_and_close_ends_the_stream`; broken by not calling the hook |
| [ok] | `connect_stream` (Windows) builds the adapter on the stream's worker thread, so all platform calls share one thread; compile-checked only | none | `cargo clippy --all-targets -- -D warnings` |
| [open] | Serial throughput and ordering over a real link: whether Write Without Response needs flow control above the platform's queue, and whether notifications arrive in order across a reconnect | `winrt-gatt/docs/winrt-pitfalls.md` | none: needs hardware |
| [open] | A stream write blocks until the adapter finishes, including reconnect back-off, so a caller on a latency-sensitive thread can stall for the backoff total | none | none yet |
