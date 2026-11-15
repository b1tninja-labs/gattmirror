# hci-wire roadmap

H4 framing and a typed view of the HCI packets `gattmirror` exchanges with the emulator's virtual controller.
No Bluetooth specification text was available, so every anchor is either a live observation (the controller
accepted these bytes) or `[unread]`. Break notes say what was changed to make the named test fail.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | A command encodes as indicator `01`, little-endian opcode, one length byte, parameters | live: Reset `01 03 0c 00` accepted, 2026-11-14 | `src/packet.rs::command_wire_bytes`, `command_with_parameters`; broken by shifting the opcode field |
| [ok] | An opcode is a 6-bit group over a 10-bit command field, and the five opcodes in `opcodes` have the values a controller accepted | live: Reset, Read Local Version, Read `BD_ADDR`, LE Read Buffer Size, LE Set Advertising Enable | `src/opcode.rs::fields_round_trip`, `opcodes_have_the_values_a_controller_accepted`; broken by changing the group shift from 10 to 9 (three tests failed) |
| [unread] | An ACL header packs a 12-bit handle, a 2-bit boundary flag and a 2-bit broadcast flag into a little-endian word, followed by a 16-bit length | not read; internally consistent only | `src/packet.rs::acl_header_packs_handle_and_flags`, `src/decoder.rs::acl_flags_are_unpacked`; broken by shifting the boundary bits. No live ACL exchange yet |
| [ok] | A stream of bytes split at any point decodes to the same packets as the whole stream | none: a property of the decoder, no standard involved | `src/decoder.rs::one_byte_at_a_time_gives_the_same_packets`, `two_packets_in_one_read`, `a_partial_packet_waits`; broken by draining the wrong byte count and by moving the ACL length offset |
| [ok] | A byte that is not a packet indicator is an error that does not advance the stream | none: H4 has no delimiter, so resynchronisation is not possible | `src/decoder.rs::an_unknown_indicator_is_an_error_that_sticks`; broken by dropping the offending byte |
| [ok] | Command Complete decodes to opcode, credit count and return parameters; a block too short for its layout is returned undecoded, never guessed | live: Command Complete for Reset and LE Read Buffer Size captured | `src/event.rs::command_complete_for_reset`, `command_complete_carries_return_parameters`, `short_blocks_are_not_guessed_at`; broken by reading the opcode big-endian |
| [unread] | Command Status decodes to status, credit count and opcode | not read; layout never seen from a live controller | `src/event.rs::command_status_layout`; not broken |
| [open] | An SCO or ISO packet decodes and re-encodes unchanged | none read | none: carried opaquely, never exercised |
| [open] | The decoder refuses an ACL length larger than the controller's advertised buffer, rather than waiting forever | not read | none yet |
| [ok] | A live controller answers Reset, Read Local Version, Read `BD_ADDR` and LE Read Buffer Size with Command Complete status 0, and answers a pre-Reset command with event `0x10` | live, 2026-11-14 | `tests/live_controller.rs` (skipped unless `GATTMIRROR_HCI` is set; passing against a local emulator, not run in CI; broken by changing the Reset opcode, two live tests failed) |
| [unread] | The opcodes for Set Event Mask, LE Set Event Mask, LE Set Advertising Parameters, Advertising Data and Scan Response Data have the values a controller accepted | live: Command Complete status 0 from `le-host`'s live test; values not read from the specification | `src/opcode.rs::advertising_opcodes_have_the_values_a_controller_accepted`; broken by changing the LE group shift, see `crates/le-host/ROADMAP.md` for the live run; the opcode value is not proven by a live status 0, see that file |
