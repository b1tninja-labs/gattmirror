//! Bluetooth HCI packets and the H4 byte-stream framing.
//!
//! The Android emulator's virtual Bluetooth controller (Rootcanal, inside `netsimd`) accepts a plain
//! TCP connection that carries *H4*: each HCI packet is one indicator byte followed by that packet
//! type's header and payload, with no extra length prefix. This crate encodes and decodes that
//! stream. It knows packet framing and a handful of opcodes; it implements no host stack.
//!
//! Everything here was checked against a live `netsimd` controller (see `tests/live_controller.rs`).
//! The Bluetooth Core Specification text was **not** available when this was written, so none of it
//! is cited to a section; see `ROADMAP.md`.

mod decoder;
mod event;
mod opcode;
mod packet;

pub use decoder::{Decoder, Error};
pub use event::{Event, EventCode};
pub use opcode::{Opcode, opcodes};
pub use packet::{Packet, PacketBoundary, PacketType};
