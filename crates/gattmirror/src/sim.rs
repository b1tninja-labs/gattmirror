//! A simulated adapter: a test dummy behind [`Backend`], not part of the mirror.
//!
//! It serves the FFF0 serial profile and answers `\r`-terminated ASCII `AT` command lines with a
//! canned reply ending in the `>` prompt, so the relay can be exercised without hardware. It is a
//! port of the behaviour of a stdlib Python dummy used the same way. It parses no vehicle protocol:
//! any line that is not an `AT` command (or the `ST` family) gets `NO DATA`.

use std::sync::mpsc::{Receiver, Sender, channel};

use gatt_profile::Profile;

use crate::backend::{Backend, BackendError, CharId, Notification, characteristics};

/// The FFF0 serial profile text: FFF1 notifies (adapter to host), FFF2 is written (host to adapter).
pub const FFF0_SERIAL: &str = include_str!("../../gatt-profile/fixtures/fff0_serial.gatt");

const IDENT: &str = "ELM327 v1.5";
const VOLTAGE: &str = "12.4V";
const NO_DATA: &str = "NO DATA";
const OK: &str = "OK";

/// Per-connection echo and line-feed state plus command handling.
#[derive(Clone, Copy, Debug)]
pub struct Session {
    echo: bool,
    linefeeds: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            echo: true,
            linefeeds: true,
        }
    }
}

impl Session {
    /// The text sent when a client connects: a blank line, the identity string, a blank line, the
    /// prompt.
    #[must_use]
    pub fn banner() -> Vec<u8> {
        format!("\r\n{IDENT}\r\n\r\n>").into_bytes()
    }

    /// The full reply, prompt included, to one command line (without its terminating `\r`).
    pub fn respond(&mut self, line: &[u8]) -> Vec<u8> {
        let text: String = line
            .iter()
            .map(|b| if b.is_ascii() { char::from(*b) } else { '?' })
            .collect();
        let cmd: String = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        if cmd.is_empty() {
            return b">".to_vec();
        }
        // ATE0 takes effect after its own line is echoed.
        let echoed = self.echo;
        let body: Vec<&str> = match cmd.as_str() {
            "ATZ" => {
                *self = Session::default();
                vec!["", IDENT]
            }
            "ATI" | "AT@1" => vec![IDENT],
            "ATRV" => vec![VOLTAGE],
            "ATDPN" => vec!["A6"],
            c if c.starts_with("AT") => {
                if let Some(rest) = c.strip_prefix("ATE") {
                    self.echo = rest != "0";
                } else if let Some(rest) = c.strip_prefix("ATL") {
                    self.linefeeds = rest != "0";
                }
                vec![OK]
            }
            c if c.starts_with("ST") => vec!["?"],
            _ => vec![NO_DATA],
        };
        let eol = if self.linefeeds { "\r\n" } else { "\r" };
        let mut out = String::new();
        if echoed {
            out.push_str(text.trim_end_matches(['\r', '\n']));
            out.push('\r');
        }
        for b in body {
            if !b.is_empty() || cmd == "ATZ" {
                out.push_str(b);
                out.push_str(eol);
            }
        }
        out.push_str(eol);
        out.push('>');
        out.into_bytes()
    }
}

/// The simulated adapter.
#[derive(Debug)]
pub struct SimAdapter {
    profile: Profile,
    tx: Sender<Notification>,
    rx: Option<Receiver<Notification>>,
    notify_char: CharId,
    write_char: CharId,
    session: Session,
    line: Vec<u8>,
    /// Every write the adapter received, oldest first: characteristic, bytes, with-response.
    pub writes: Vec<(CharId, Vec<u8>, bool)>,
    /// How many reads reached the adapter.
    pub reads: usize,
}

impl Default for SimAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SimAdapter {
    /// An adapter serving [`FFF0_SERIAL`].
    ///
    /// # Panics
    /// Never in practice: the embedded profile is checked by the tests.
    #[must_use]
    pub fn new() -> Self {
        let profile = gatt_profile::parse(FFF0_SERIAL).expect("the embedded profile parses");
        let find = |short: u16| {
            characteristics(&profile)
                .find(|(_, _, c)| c.uuid.as_u16() == Some(short))
                .map(|(id, _, _)| id)
                .expect("the embedded profile has the serial characteristics")
        };
        let notify_char = find(0xFFF1);
        let write_char = find(0xFFF2);
        let (tx, rx) = channel();
        SimAdapter {
            profile,
            tx,
            rx: Some(rx),
            notify_char,
            write_char,
            session: Session::default(),
            line: Vec::new(),
            writes: Vec::new(),
            reads: 0,
        }
    }

    /// Advertise and serve `name` instead of the profile's.
    pub fn set_name(&mut self, name: &str) {
        crate::table::apply_name(&mut self.profile, name);
    }

    fn send(&self, value: Vec<u8>) {
        // A closed receiver means the mirror is gone; nobody is left to hear the reply.
        let _ = self.tx.send(Notification {
            char: self.notify_char,
            value,
        });
    }
}

impl Backend for SimAdapter {
    fn profile(&self) -> &Profile {
        &self.profile
    }

    fn read(&mut self, ch: CharId) -> Result<Vec<u8>, BackendError> {
        self.reads += 1;
        let _ = ch;
        Err(BackendError::NotReadable)
    }

    fn write(&mut self, ch: CharId, value: &[u8], with_response: bool) -> Result<(), BackendError> {
        if ch != self.write_char {
            return Err(BackendError::NotWritable);
        }
        self.writes.push((ch, value.to_vec(), with_response));
        for &b in value {
            if b == b'\r' {
                let line = std::mem::take(&mut self.line);
                let reply = self.session.respond(&line);
                self.send(reply);
            } else {
                self.line.push(b);
            }
        }
        Ok(())
    }

    fn set_notify(&mut self, ch: CharId, enabled: bool) -> Result<(), BackendError> {
        if ch != self.notify_char {
            return Err(BackendError::Failed);
        }
        if enabled {
            self.session = Session::default();
            self.line.clear();
            self.send(Session::banner());
        }
        Ok(())
    }

    fn notifications(&mut self) -> Option<Receiver<Notification>> {
        self.rx.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(s: &mut Session, line: &str) -> String {
        String::from_utf8(s.respond(line.as_bytes())).unwrap()
    }

    #[test]
    fn identity_and_voltage_are_looked_up() {
        let mut s = Session::default();
        assert_eq!(reply(&mut s, "ATI"), "ATI\rELM327 v1.5\r\n\r\n>");
        assert_eq!(reply(&mut s, "at rv"), "at rv\r12.4V\r\n\r\n>");
        assert_eq!(reply(&mut s, "ATDPN"), "ATDPN\rA6\r\n\r\n>");
    }

    #[test]
    fn reset_prints_a_blank_line_then_the_identity() {
        let mut s = Session::default();
        assert_eq!(reply(&mut s, "ATZ"), "ATZ\r\r\nELM327 v1.5\r\n\r\n>");
    }

    #[test]
    fn echo_off_takes_effect_after_its_own_line() {
        let mut s = Session::default();
        assert_eq!(reply(&mut s, "ATE0"), "ATE0\rOK\r\n\r\n>");
        assert_eq!(reply(&mut s, "ATI"), "ELM327 v1.5\r\n\r\n>");
        reply(&mut s, "ATZ");
        assert_eq!(reply(&mut s, "ATI"), "ATI\rELM327 v1.5\r\n\r\n>");
    }

    #[test]
    fn linefeeds_off_shortens_the_line_ending() {
        let mut s = Session::default();
        assert_eq!(reply(&mut s, "ATL0"), "ATL0\rOK\r\r>");
    }

    #[test]
    fn anything_else_is_no_data_or_not_supported() {
        let mut s = Session::default();
        assert_eq!(reply(&mut s, "0100"), "0100\rNO DATA\r\n\r\n>");
        assert_eq!(reply(&mut s, "STI"), "STI\r?\r\n\r\n>");
        assert_eq!(reply(&mut s, ""), ">");
    }

    #[test]
    fn the_embedded_profile_has_the_serial_pair() {
        let a = SimAdapter::new();
        assert_ne!(a.notify_char, a.write_char);
    }
}
