//! 48-bit Bluetooth addresses as the `u64` the Windows API uses, and their text form.
//!
//! Addresses are always parameters; none appears in this crate other than synthetic test values.

use crate::error::Error;

/// Format as `AA:BB:CC:DD:EE:FF`.
#[must_use]
pub fn format_address(addr: u64) -> String {
    let b = addr.to_be_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

/// Parse `AA:BB:CC:DD:EE:FF`, `AA-BB-CC-DD-EE-FF` or twelve bare hex digits.
///
/// # Errors
/// Returns [`Error::InvalidArgument`] for anything else.
pub fn parse_address(s: &str) -> Result<u64, Error> {
    let bad = || Error::InvalidArgument(format!("not a Bluetooth address: {s:?}"));
    let t = s.trim();
    let hex: String = if t.contains([':', '-']) {
        let parts: Vec<&str> = t.split([':', '-']).collect();
        if parts.len() != 6 || parts.iter().any(|p| p.len() != 2) {
            return Err(bad());
        }
        parts.concat()
    } else {
        t.to_owned()
    };
    if hex.len() != 12 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    u64::from_str_radix(&hex, 16).map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let a = 0x0011_2233_4455;
        assert_eq!(format_address(a), "00:11:22:33:44:55");
        assert_eq!(parse_address("00:11:22:33:44:55").unwrap(), a);
        assert_eq!(parse_address("00-11-22-33-44-55").unwrap(), a);
        assert_eq!(parse_address("001122334455").unwrap(), a);
        assert_eq!(
            parse_address("aa:bb:cc:dd:ee:ff").unwrap(),
            0xAABB_CCDD_EEFF
        );
    }

    #[test]
    fn rejects_malformed() {
        for s in [
            "",
            "00:11:22:33:44",
            "00:11:22:33:44:5",
            "00:11:22:33:44:55:66",
            "00112233445g",
        ] {
            assert!(parse_address(s).is_err(), "{s}");
        }
    }
}
