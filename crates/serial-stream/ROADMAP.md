# serial-stream roadmap

The shared contract between mirror frontends (a BLE peripheral, ELM327-over-TCP) and backends (a real adapter, a
scenario, a simulated vehicle, an upstream TCP connection): a duplex `ByteStream`, an in-memory pair, a TCP client and
a TCP server that serves a factory of streams. Nothing here parses ELM327, OBD or UDS; chunk boundaries carry no
meaning. Standard library only.

`[ok]` rows were each broken once (code changed, named test watched to fail, change reverted). No standard defines
these behaviours, so anchors are "none". The TCP framing is a raw byte pipe (no telnet or other negotiation).

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | Bytes written to one end of `mem::pair` arrive in order on the other end's receiver, one chunk per write, in both directions | none | `src/mem.rs::tests::bytes_cross_in_both_directions_in_order`; broken by sending an empty chunk (3 failed) |
| [ok] | `close` is idempotent, fails later writes with `Closed`, and disconnects the peer's receiver; writing to a dropped peer is `Closed` | none | `src/mem.rs::tests::close_fails_writes_and_disconnects_the_peer`, `writing_to_a_dropped_peer_is_closed`; broken by making `close` a no-op (1 failed) |
| [ok] | The receiver is handed out once | none | `src/mem.rs::tests::the_receiver_is_handed_out_once` |
| [ok] | `idle_timeout` is a hint: `None` unless the stream was built with one | none | `src/mem.rs::tests::idle_timeout_is_a_hint_only_when_set` |
| [ok] | `TcpClient` writes the bytes to the socket, delivers socket reads as chunks, and refuses writes after `close` | none | `src/tcp.rs::tests::a_client_talks_to_a_served_stream`; broken by dropping the closed check (1 failed) |
| [ok] | `Server` gives every accepted connection its own factory-made stream; socket bytes are written to the stream (so a stream sees what the client sent) | none | `src/tcp.rs::tests::each_connection_gets_its_own_stream`; broken by not writing socket bytes to the stream (2 failed) |
| [ok] | A factory that returns `None` refuses the connection (the client sees end of stream) | none | `src/tcp.rs::tests::a_refused_factory_closes_the_connection`; broken by always refusing (3 failed) |
| [ok] | When the served stream ends, its last replies are delivered and then the socket is closed | none | `src/tcp.rs::tests::the_stream_ending_closes_the_socket_after_its_last_reply`; broken by ignoring the stream's end (1 failed) |
| [ok] | `tcp::loopback` is the default listen address and `Server::local_addr` reports the bound port | none | `src/tcp.rs::tests::a_client_talks_to_a_served_stream`; broken by binding the unspecified address (assertion fails, and the run then does not terminate) |
| [ok] | `Server::shutdown` stops accepting | none | `src/tcp.rs::tests::shutdown_stops_accepting` (slow on Windows: a refused connect takes about 4 s) |
| [open] | A served stream whose factory is slow or whose reply channel never disconnects is cut off by the idle-timeout hint | none | none: the hint is exposed but the server does not act on it |
| [open] | Write back-pressure over TCP is bounded | none | none: `write` blocks on the socket only |
