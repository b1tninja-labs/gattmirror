# le-host roadmap

The LE peripheral host over `hci-wire`. This slice: controller init and advertising (gate E1). Connections,
ACL and L2CAP are not started. No Bluetooth specification text was available, so every anchor is a live
observation or `[unread]`. "Broken by" says what was changed to make the named test fail, then reverted.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | Init sends Reset first, then event masks, `BD_ADDR`, LE buffer size, and returns the address, ACL length and count | live: a pre-Reset command gets event `0x10`, see `crates/hci-wire/ROADMAP.md` | `src/controller.rs::init_sends_reset_first_and_reads_the_buffer_size`; broken by swapping Reset and Set Event Mask (three tests failed) |
| [ok] | A command answered with event `0x10` is reported as `NotReset`, not a hang or a misparse | live: `04 10 01 42` observed | `src/controller.rs::a_command_before_reset_is_reported_not_reset`; broken by skipping the `0x10` check. Live: `tests/live_advertise.rs::a_command_before_reset_is_not_reset` |
| [ok] | No command is written while the controller's credit count is zero; a no-op Command Complete grants a credit | none read: behaviour of the host as designed; `Num_HCI_Command_Packets` semantics `[unread]` | `src/controller.rs::no_command_is_sent_while_the_credit_count_is_zero`; broken by removing the wait loop |
| [ok] | A nonzero status in a Command Complete or Status is an error carrying opcode and status | none: project behaviour | `src/controller.rs::a_refused_command_carries_its_status`; broken by treating any status as success |
| [ok] | Packets received while waiting for an answer (ACL, other events) are kept in order, not dropped | none: project behaviour | `src/controller.rs::unrelated_packets_are_kept_not_dropped`; broken by dropping them |
| [ok] | A closed transport is an error, not a hang | none: project behaviour | `src/controller.rs::a_closed_transport_is_an_error_not_a_hang` (fake returns 0 bytes) |
| [unread] | Advertising runs disable, parameters, advertising data, scan response, enable, in that order, enable last with `1` | not read; live controller returned status 0 for each | `src/controller.rs::advertising_runs_the_five_commands_in_order`; broken by sending `0` as the last enable. Live: `tests/live_advertise.rs::init_and_advertise_a_fixed_name` |
| [unread] | Advertising data is flags `06`, a service UUID (16-bit little-endian, or 128-bit reversed from text order) and the name, shortened (`08`) when it does not fit in 31 bytes; scan response carries the name | not read; the AD type numbers and the flags value are from memory. Live: the emulator's scan listed the name | `src/advertising.rs::data_has_flags_uuid_and_name`, `a_128_bit_uuid_is_sent_least_significant_byte_first`, `a_long_name_is_shortened_to_fit`; broken by changing the flags value and by removing the UUID reversal |
| [unread] | The advertising parameter block is 15 bytes: interval min and max, type `0` (connectable undirected), public own address, channel map `7`, no filter | not read; field order from memory, accepted live | `src/advertising.rs::parameter_block_is_fifteen_bytes`; broken by changing the channel map |
| [unread] | Data commands carry a length byte and 31 zero-padded bytes | not read | `src/advertising.rs::data_commands_carry_a_length_and_31_padded_bytes`; broken by removing the padding |
| [unread] | **E1.** The emulated phone's scan lists the advertised name | none read | live, manual: with `examples/advertise.rs` running against the emulator, the Voltage app (BLE mode, adapter dropdown) on `emulator-5554` listed the test name; screenshot kept outside the tree. Not automated |
| [open] | A live status 0 proves the right command was sent | the live test cannot tell Set Advertising Parameters from a different command that also returns status 0 | demonstrated: changing the opcode to `0x2007` still passed the live test, so the opcode value is held only by the hci-wire unit test, whose value is itself from memory and `[unread]` |
| [open] | Connection, ACL fragmentation, L2CAP, refusing SMP | | not started |
