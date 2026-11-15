//! Which advertiser a scan is looking for. Both criteria are caller-supplied.

use crate::error::Error;

/// A scan target: a case-insensitive name substring, a 48-bit address, or both (both must match).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanFilter {
    /// Substring of the advertised local name, compared case-insensitively.
    pub name_contains: Option<String>,
    /// Exact 48-bit address.
    pub address: Option<u64>,
}

impl ScanFilter {
    /// Match by name substring.
    #[must_use]
    pub fn by_name(name: impl Into<String>) -> Self {
        Self {
            name_contains: Some(name.into()),
            address: None,
        }
    }

    /// Match by address.
    #[must_use]
    pub const fn by_address(address: u64) -> Self {
        Self {
            name_contains: None,
            address: Some(address),
        }
    }

    /// True if the filter has no criteria and would match any advertiser.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name_contains.as_deref().is_none_or(str::is_empty) && self.address.is_none()
    }

    /// Reject a filter that would grab an arbitrary device.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] if [`is_empty`](Self::is_empty).
    pub fn validate(&self) -> Result<(), Error> {
        if self.is_empty() {
            Err(Error::InvalidArgument(
                "scan filter has neither a name nor an address".into(),
            ))
        } else {
            Ok(())
        }
    }

    /// Whether an advertisement matches; `local_name` is empty when the packet carried none.
    #[must_use]
    pub fn matches(&self, local_name: &str, address: u64) -> bool {
        let name_ok = match self.name_contains.as_deref() {
            None | Some("") => true,
            Some(n) => local_name.to_lowercase().contains(&n.to_lowercase()),
        };
        let addr_ok = self.address.is_none_or(|a| a == address);
        name_ok && addr_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u64 = 0x0011_2233_4455;
    const B: u64 = 0x0011_2233_4466;

    #[test]
    fn name_match_is_a_case_insensitive_substring() {
        let f = ScanFilter::by_name("link");
        assert!(f.matches("My LINK Adapter", A));
        assert!(!f.matches("Other", A));
        assert!(!f.matches("", A));
    }

    #[test]
    fn address_match_is_exact() {
        let f = ScanFilter::by_address(A);
        assert!(f.matches("", A));
        assert!(!f.matches("", B));
    }

    #[test]
    fn both_criteria_must_hold() {
        let f = ScanFilter {
            name_contains: Some("x".into()),
            address: Some(A),
        };
        assert!(f.matches("x", A));
        assert!(!f.matches("x", B));
        assert!(!f.matches("y", A));
    }

    #[test]
    fn an_empty_filter_is_rejected() {
        assert!(ScanFilter::default().validate().is_err());
        assert!(ScanFilter::by_name("").validate().is_err());
        assert!(ScanFilter::by_name("a").validate().is_ok());
    }
}
