use crate::model::{Profile, Properties};
use crate::uuid::Uuid;

/// Which characteristics of a service carry a serial stream, chosen by property, not by number.
///
/// The rule follows obd-bridge's `docs/ble-re.md`: the notify side is the characteristic with notify
/// (preferred) or indicate; the write side is the one with write-without-response (preferred) or write.
/// When one characteristic has both, `write` and `notify` are the same UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialRoles {
    /// The service holding both characteristics.
    pub service: Uuid,
    /// The characteristic the host writes to.
    pub write: Uuid,
    /// True when `write` was chosen for write-without-response.
    pub write_without_response: bool,
    /// The characteristic that notifies (or indicates) back.
    pub notify: Uuid,
    /// True when `notify` was chosen for indications only.
    pub indicate_only: bool,
}

impl Profile {
    /// Every service that has a writable and a notifying characteristic, in table order.
    #[must_use]
    pub fn serial_candidates(&self) -> Vec<SerialRoles> {
        let mut out = Vec::new();
        for s in &self.services {
            let ch = &s.characteristics;
            let notify = ch
                .iter()
                .find(|c| c.properties.contains(Properties::NOTIFY))
                .map(|c| (c.uuid, false))
                .or_else(|| {
                    ch.iter()
                        .find(|c| c.properties.contains(Properties::INDICATE))
                        .map(|c| (c.uuid, true))
                });
            let write = ch
                .iter()
                .find(|c| c.properties.contains(Properties::WRITE_NO_RESPONSE))
                .map(|c| (c.uuid, true))
                .or_else(|| {
                    ch.iter()
                        .find(|c| c.properties.contains(Properties::WRITE))
                        .map(|c| (c.uuid, false))
                });
            if let (Some((notify, indicate_only)), Some((write, write_without_response))) =
                (notify, write)
            {
                out.push(SerialRoles {
                    service: s.uuid,
                    write,
                    write_without_response,
                    notify,
                    indicate_only,
                });
            }
        }
        out
    }

    /// The first serial candidate, if any.
    #[must_use]
    pub fn serial(&self) -> Option<SerialRoles> {
        self.serial_candidates().into_iter().next()
    }
}
