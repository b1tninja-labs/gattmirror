use crate::uuid::Uuid;

/// Characteristic property bits, one byte as carried in a characteristic declaration.
///
/// Bit positions: Core Specification 7.0, Vol 3 Part G, Table 3.5. In 7.0 bit `0x40` is "previously
/// used"; Core 6.0 names it Authenticated Signed Writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Properties(pub u8);

impl Properties {
    /// Broadcast.
    pub const BROADCAST: u8 = 0x01;
    /// Read.
    pub const READ: u8 = 0x02;
    /// Write without response.
    pub const WRITE_NO_RESPONSE: u8 = 0x04;
    /// Write.
    pub const WRITE: u8 = 0x08;
    /// Notify.
    pub const NOTIFY: u8 = 0x10;
    /// Indicate.
    pub const INDICATE: u8 = 0x20;
    /// Authenticated signed writes (`0x40`; marked previously used in Core 7.0).
    pub const SIGNED_WRITE: u8 = 0x40;
    /// Extended properties descriptor present.
    pub const EXTENDED: u8 = 0x80;

    /// The text-format names in bit order, lowest first.
    pub const NAMES: [(&'static str, u8); 8] = [
        ("broadcast", Self::BROADCAST),
        ("read", Self::READ),
        ("write-no-response", Self::WRITE_NO_RESPONSE),
        ("write", Self::WRITE),
        ("notify", Self::NOTIFY),
        ("indicate", Self::INDICATE),
        ("signed-write", Self::SIGNED_WRITE),
        ("extended", Self::EXTENDED),
    ];

    /// True when every bit in `mask` is set.
    #[must_use]
    pub const fn contains(self, mask: u8) -> bool {
        self.0 & mask == mask
    }
}

/// Whether a service is primary or secondary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    /// A primary service.
    Primary,
    /// A secondary service, included by others.
    Secondary,
}

/// A descriptor: a UUID and its static value (empty when the value is live).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// Descriptor type.
    pub uuid: Uuid,
    /// Static value bytes.
    pub value: Vec<u8>,
}

/// A characteristic with its properties, optional static value and descriptors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Characteristic {
    /// Characteristic type.
    pub uuid: Uuid,
    /// Property bits.
    pub properties: Properties,
    /// A static value captured from the peripheral, if one was read.
    pub value: Option<Vec<u8>>,
    /// Descriptors in declaration order.
    pub descriptors: Vec<Descriptor>,
}

/// A service and its characteristics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    /// Service type.
    pub uuid: Uuid,
    /// Primary or secondary.
    pub kind: ServiceKind,
    /// Characteristics in declaration order.
    pub characteristics: Vec<Characteristic>,
}

/// A snapshot of a peripheral: its table and its advertised identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    /// The GAP Device Name as read from the table.
    pub device_name: Option<String>,
    /// The name carried in advertising data.
    pub advertised_name: Option<String>,
    /// Service UUIDs carried in advertising data.
    pub advertised_services: Vec<Uuid>,
    /// Services in declaration order.
    pub services: Vec<Service>,
}
