//! L2CAP framing over ACL: reassembly, fragmentation, and the fixed-channel replies.
//!
//! Channel ids, command codes and the reply layouts are from memory, not the specification, and
//! are `[unread]` in `ROADMAP.md`.

use hci_wire::{Packet, PacketBoundary};

/// The ATT fixed channel.
pub const CID_ATT: u16 = 4;
/// The LE signalling fixed channel.
pub const CID_SIGNALLING: u16 = 5;
/// The Security Manager fixed channel.
pub const CID_SMP: u16 = 6;

/// Rebuilds L2CAP frames from ACL fragments of one connection.
#[derive(Debug, Default)]
pub struct Reassembler {
    buf: Vec<u8>,
    total: usize,
    active: bool,
    discarded: usize,
}

impl Reassembler {
    /// How many fragments or partial frames were dropped as malformed.
    #[must_use]
    pub fn discarded(&self) -> usize {
        self.discarded
    }

    fn drop_partial(&mut self) {
        if self.active {
            self.discarded += 1;
        }
        self.active = false;
        self.buf.clear();
    }

    /// Feed one ACL fragment. Returns a complete frame as `(cid, payload)`.
    ///
    /// A start fragment abandons any frame in progress. A continuation with no frame in progress,
    /// or one that overruns the length in the header, is dropped.
    pub fn push(&mut self, boundary: PacketBoundary, data: &[u8]) -> Option<(u16, Vec<u8>)> {
        match boundary {
            PacketBoundary::FirstNonFlushable | PacketBoundary::FirstFlushable => {
                self.drop_partial();
                self.buf.extend_from_slice(data);
                self.active = true;
            }
            PacketBoundary::Continuing => {
                if !self.active {
                    self.discarded += 1;
                    return None;
                }
                self.buf.extend_from_slice(data);
            }
            PacketBoundary::Reserved => {
                self.discarded += 1;
                return None;
            }
        }
        if self.buf.len() < 4 {
            // Not even the header yet: wait for more.
            return None;
        }
        self.total = 4 + usize::from(u16::from_le_bytes([self.buf[0], self.buf[1]]));
        if self.buf.len() > self.total {
            self.active = false;
            self.buf.clear();
            self.discarded += 1;
            return None;
        }
        if self.buf.len() < self.total {
            return None;
        }
        self.active = false;
        let cid = u16::from_le_bytes([self.buf[2], self.buf[3]]);
        let payload = self.buf[4..].to_vec();
        self.buf.clear();
        Some((cid, payload))
    }
}

/// Wrap `payload` in an L2CAP header for `cid` and cut it into ACL packets of at most `max` data
/// bytes each. The first is a start fragment, the rest continuations.
///
/// # Panics
///
/// If `max` is zero or the payload is longer than a 16-bit length field.
#[must_use]
pub fn fragment(handle: u16, cid: u16, payload: &[u8], max: usize) -> Vec<Packet> {
    assert!(max > 0, "zero ACL size");
    let len = u16::try_from(payload.len()).expect("L2CAP payload over 65535 bytes");
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&cid.to_le_bytes());
    frame.extend_from_slice(payload);
    frame
        .chunks(max)
        .enumerate()
        .map(|(i, c)| Packet::Acl {
            handle,
            boundary: if i == 0 {
                PacketBoundary::FirstNonFlushable
            } else {
                PacketBoundary::Continuing
            },
            broadcast: 0,
            data: c.to_vec(),
        })
        .collect()
}

/// The reply to a signalling-channel frame: a Command Reject (code `01`, reason 0, "command not
/// understood") for any request, and nothing for a response. Requests are the even codes. `[unread]`
pub fn signalling_reply(payload: &[u8]) -> Option<Vec<u8>> {
    let (&code, &id) = (payload.first()?, payload.get(1)?);
    (code != 0 && code % 2 == 0).then(|| vec![0x01, id, 0x02, 0x00, 0x00, 0x00])
}

/// The reply to a Security Manager frame: Pairing Failed (code `05`) with reason `05` (pairing not
/// supported) to a Pairing Request (code `01`), and nothing otherwise. `[unread]`
pub fn smp_reply(payload: &[u8]) -> Option<Vec<u8>> {
    (payload.first() == Some(&0x01)).then(|| vec![0x05, 0x05])
}

#[cfg(test)]
mod tests {
    use hci_wire::{Packet, PacketBoundary as B};

    use super::{Reassembler, fragment, signalling_reply, smp_reply};

    #[test]
    fn a_frame_in_one_fragment() {
        let mut r = Reassembler::default();
        assert_eq!(
            r.push(B::FirstFlushable, &[2, 0, 4, 0, 0xA, 0xB]),
            Some((4, vec![0xA, 0xB]))
        );
    }

    #[test]
    fn fragments_are_joined() {
        let mut r = Reassembler::default();
        assert_eq!(r.push(B::FirstNonFlushable, &[3, 0, 4, 0, 1]), None);
        assert_eq!(r.push(B::Continuing, &[2]), None);
        assert_eq!(r.push(B::Continuing, &[3]), Some((4, vec![1, 2, 3])));
    }

    #[test]
    fn a_continuation_with_nothing_started_is_dropped() {
        let mut r = Reassembler::default();
        assert_eq!(r.push(B::Continuing, &[1, 2]), None);
        assert_eq!(r.discarded(), 1);
    }

    #[test]
    fn a_new_start_abandons_the_frame_in_progress() {
        let mut r = Reassembler::default();
        r.push(B::FirstNonFlushable, &[5, 0, 4, 0, 1]);
        assert_eq!(
            r.push(B::FirstNonFlushable, &[1, 0, 4, 0, 9]),
            Some((4, vec![9]))
        );
        assert_eq!(r.discarded(), 1);
    }

    #[test]
    fn an_overrun_is_dropped() {
        let mut r = Reassembler::default();
        r.push(B::FirstNonFlushable, &[2, 0, 4, 0, 1]);
        assert_eq!(r.push(B::Continuing, &[2, 3]), None);
        // The frame is gone: a further continuation has nothing to attach to.
        assert_eq!(r.push(B::Continuing, &[4]), None);
        assert_eq!(r.discarded(), 2);
    }

    #[test]
    fn a_header_split_across_fragments_is_joined() {
        let mut r = Reassembler::default();
        assert_eq!(r.push(B::FirstNonFlushable, &[1, 0]), None);
        assert_eq!(r.push(B::Continuing, &[4, 0]), None);
        assert_eq!(r.push(B::Continuing, &[7]), Some((4, vec![7])));
    }

    #[test]
    fn a_zero_length_frame_completes() {
        let mut r = Reassembler::default();
        assert_eq!(r.push(B::FirstFlushable, &[0, 0, 6, 0]), Some((6, vec![])));
    }

    #[test]
    fn fragmenting_respects_the_buffer_size_and_round_trips() {
        let payload: Vec<u8> = (0..50).collect();
        let packets = fragment(0x40, 4, &payload, 27);
        let sizes: Vec<_> = packets
            .iter()
            .map(|p| match p {
                Packet::Acl { data, .. } => data.len(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(sizes, [27, 27]);
        let mut r = Reassembler::default();
        let mut got = None;
        for p in &packets {
            let Packet::Acl { boundary, data, .. } = p else {
                unreachable!()
            };
            got = r.push(*boundary, data);
        }
        assert_eq!(got, Some((4, payload)));
        assert!(matches!(
            &packets[1],
            Packet::Acl {
                boundary: B::Continuing,
                ..
            }
        ));
    }

    #[test]
    fn signalling_requests_are_rejected_and_responses_ignored() {
        assert_eq!(
            signalling_reply(&[0x12, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Some(vec![1, 7, 2, 0, 0, 0])
        );
        assert_eq!(signalling_reply(&[0x13, 7, 2, 0, 0, 0]), None);
        assert_eq!(signalling_reply(&[0x01, 7, 2, 0, 0, 0]), None);
    }

    #[test]
    fn pairing_is_refused() {
        assert_eq!(smp_reply(&[1, 3, 0, 1, 16, 0, 0]), Some(vec![5, 5]));
        assert_eq!(smp_reply(&[0x0B, 0]), None);
    }
}
