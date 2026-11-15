//! MTU arithmetic and write chunking.
//!
//! An ATT value carried in a single PDU is at most `MTU - 3` bytes (one opcode byte plus a two-byte
//! handle), and the default MTU is 23. Both are what the working precedent (`obd-bridge`) assumes and
//! are `[unread]`.

/// The default ATT MTU before any exchange.
pub const DEFAULT_MTU: u16 = 23;

/// Bytes of ATT header (opcode and handle) that precede a value in a write.
pub const ATT_HEADER: usize = 3;

/// Largest value that fits one write PDU at `mtu`. Never zero.
#[must_use]
pub fn max_payload(mtu: u16) -> usize {
    usize::from(mtu).saturating_sub(ATT_HEADER).max(1)
}

/// Split `data` into pieces of at most `max` bytes (a `max` of zero is treated as one).
///
/// Empty input yields no pieces: an empty write is the caller's decision, not a chunking one.
pub fn chunks(data: &[u8], max: usize) -> impl Iterator<Item = &[u8]> {
    data.chunks(max.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_mtu_minus_header() {
        assert_eq!(max_payload(DEFAULT_MTU), 20);
        assert_eq!(max_payload(247), 244);
    }

    #[test]
    fn payload_is_never_zero() {
        assert_eq!(max_payload(0), 1);
        assert_eq!(max_payload(3), 1);
    }

    #[test]
    fn chunks_cover_the_input_in_order() {
        let data: Vec<u8> = (0..45).collect();
        let parts: Vec<&[u8]> = chunks(&data, 20).collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [20, 20, 5]
        );
        assert_eq!(parts.concat(), data);
    }

    #[test]
    fn exact_multiple_has_no_empty_tail() {
        let data = [0u8; 40];
        assert_eq!(chunks(&data, 20).count(), 2);
        assert_eq!(chunks(&[], 20).count(), 0);
    }

    #[test]
    fn zero_max_does_not_panic() {
        assert_eq!(chunks(&[1, 2, 3], 0).count(), 3);
    }
}
