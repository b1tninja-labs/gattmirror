# winrt-backend roadmap

`gattmirror::Backend` over `winrt-gatt`'s reconnecting adapter. Transport only. The Windows constructor
(`connect`) is compile-checked and has never been run; the Bluetooth Core Specification is not available, so
no row cites it. Depends on the `Backend` trait from `crates/gattmirror`.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | Profile and `CharId`s follow the discovered table: ids count characteristics across the whole profile in order | `gattmirror::characteristics` | `src/lib.rs::profile_and_char_ids_follow_the_table` (fake connector) |
| [ok] | Reads and writes by `CharId` reach the matching peripheral handle, with response or without | none: project-defined | `reads_and_writes_reach_the_right_handle`; broken by mapping the wrong handle in the adapter (see `winrt-gatt` ROADMAP) |
| [ok] | Refusals map to `NotReadable`, `NotWritable` and `Failed` | `gattmirror::BackendError` | `refusals_map_to_the_mirrors_errors`; broken by swapping the read and write mappings |
| [ok] | Notifications reach the mirror as `Notification` by `CharId`; values for unknown handles are dropped; the receiver is yielded once | none: project-defined | `notifications_arrive_by_char_id_and_unknown_handles_are_dropped`; broken by mapping every handle to index 0 |
| [open] | `connect` finds a real peripheral, snapshots it and relays reads, writes and notifications; CCCDs are dropped from the profile because the mirror's ATT server adds its own | needs hardware | none: needs hardware |
| [open] | A notification forwarding thread per backend ends only when the backend's sender side or the receiver is dropped; a stalled mirror lets values queue unboundedly | none | none yet |
