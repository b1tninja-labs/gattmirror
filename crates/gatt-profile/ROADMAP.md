# gatt-profile roadmap

A no-I/O model of a peripheral's GATT table, a plain-text snapshot format with a parser and writer, and a
flattening into handle-ordered attributes for a future `att-server`. No Bluetooth specification text was
available, so every anchor that would be a standard is `[unread]`. Break notes say what was changed to make the
named test fail (all restored afterwards).

Format, one directive per line, `#` comments, free indentation: `device-name`, `adv-name`, `adv-service UUID`,
`service UUID [primary|secondary]`, `characteristic UUID PROPS [value=HEX]`, `descriptor UUID [value=HEX]`.
Fixtures: `fixtures/fff0_serial.gatt` (FFF1 notify with CCCD, FFF2 write; shape from obd-bridge `docs/ble-re.md`)
and `fixtures/nordic_uart.gatt` (128-bit UUIDs, a secondary service).

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | A 16-bit UUID expands onto the base UUID `0000xxxx-0000-1000-8000-00805f9b34fb`; short and dashed forms parse to the same value, case-insensitively | base UUID as stated in obd-bridge `docs/ble-re.md` (the SIG definition itself is unread) | `src/uuid.rs::short_form_expands_onto_the_base_uuid`, `short_displays_short_and_long_displays_dashed`; broken by shifting the short field by 95 and by changing the base's last byte |
| [ok] | Malformed UUIDs (wrong length, non-hex, misplaced dash) are rejected; a base UUID with extra bits set is not treated as short | none: a property of this parser | `src/uuid.rs::malformed_uuids_are_rejected`, `a_base_uuid_with_other_bits_set_is_not_short`; not broken |
| [ok] | The text format round-trips: `parse(write(p)) == p`, and `write` is idempotent | none: a property of this format | `tests/profile.rs::fixtures_round_trip_and_the_writer_is_idempotent`; broken by appending junk in the writer |
| [ok] | The FFF0 fixture parses to a primary service FFF0 with FFF1 notify plus CCCD and FFF2 write/write-without-response, and property selection works by bit | `D:\code\obd-bridge\docs\ble-re.md` (static recovery from the app, not a live adapter) | `tests/profile.rs::fff0_fixture_has_the_expected_shape`; broken by swapping the notify bit |
| [ok] | Parse errors carry the 1-based line number and a typed reason (unknown directive, missing argument, bad UUID, property, hex or option, orphaned characteristic or descriptor, duplicates, trailing text) | none: a property of this parser | `tests/profile.rs::errors_carry_line_numbers`, `error_text_names_the_line`; broken by reporting line `i` instead of `i + 1` and by accepting odd-length hex |
| [unread] | Property names map to bits `01 02 04 08 10 20 40 80` (broadcast, read, write-no-response, write, notify, indicate, signed-write, extended) | not read; remembered layout | `tests/profile.rs::raw_property_bytes_and_none_parse`, `fff0_fixture_has_the_expected_shape`; broken by changing the notify bit, which only proves the test is wired, not that the value is right |
| [unread] | `Profile::attributes` yields sequential handles from 1; a service declaration is type `2800`/`2801` with the service UUID little-endian; a characteristic declaration is type `2803` with properties, value handle, UUID; the service group's end handle is its last attribute | not read; consistent with itself only | `tests/profile.rs::attributes_have_sequential_handles_and_encoded_declarations`, `a_long_uuid_declaration_carries_sixteen_bytes`; broken by encoding the declaration handle instead of the value handle, by an end handle off by one, and by starting handles at 0 |
| [ok] | A profile needing more than 65535 handles is an error, not a wrap | none: arithmetic on a 16-bit space | `tests/profile.rs::too_many_attributes_is_an_error`; not broken |
| [open] | A profile captured from a live adapter (discovery plus reads) rather than written by hand or recovered statically | needs hardware or the `winrt-gatt` client | none |
| [open] | A notify or indicate characteristic without a `2902` descriptor is flagged, and a characteristic whose descriptors include `2902` is checked for the notify or indicate bit | not read | none yet |
| [open] | Names with line breaks or edge whitespace round-trip (the format cannot carry them; the writer does not escape) | none | none |
| [open] | Included-service and other descriptor-typed declarations (`2802`, extended properties values) | not read | none |
