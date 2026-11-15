use std::fmt;

use crate::model::{Profile, ServiceKind};
use crate::uuid::Uuid;

/// The attribute type UUIDs [`Profile::attributes`] uses. `[unread]`: taken from common knowledge of
/// GATT, not from the specification.
pub mod type_uuids {
    use crate::uuid::Uuid;

    /// Primary service declaration.
    pub const PRIMARY_SERVICE: Uuid = Uuid::from_u16(0x2800);
    /// Secondary service declaration.
    pub const SECONDARY_SERVICE: Uuid = Uuid::from_u16(0x2801);
    /// Characteristic declaration.
    pub const CHARACTERISTIC: Uuid = Uuid::from_u16(0x2803);
}

/// What an attribute is, beyond its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeRole {
    /// A service declaration; the group runs from this handle to `end_handle`.
    Service {
        /// Last handle of the service.
        end_handle: u16,
    },
    /// A characteristic declaration.
    Declaration,
    /// A characteristic value. `value` is empty when the profile carried no static value.
    Value {
        /// The characteristic's property bits.
        properties: u8,
        /// True when the profile supplied a static value.
        has_static_value: bool,
    },
    /// A descriptor.
    Descriptor,
}

/// One row of the attribute database an ATT server would serve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// Handle, starting at 1 and increasing by 1.
    pub handle: u16,
    /// Attribute type.
    pub attr_type: Uuid,
    /// Value bytes (declarations are encoded; see [`Profile::attributes`]).
    pub value: Vec<u8>,
    /// Role in the table.
    pub role: AttributeRole,
}

/// A profile that cannot be flattened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeError {
    /// More attributes than the 16-bit handle space holds.
    TooManyAttributes,
}

impl fmt::Display for AttributeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the profile needs more than 65535 attribute handles")
    }
}

impl std::error::Error for AttributeError {}

fn next_handle(len: usize) -> Result<u16, AttributeError> {
    u16::try_from(len + 1).map_err(|_| AttributeError::TooManyAttributes)
}

impl Profile {
    /// Flatten into attributes with sequential handles from 1.
    ///
    /// Per service: a declaration (value is the service UUID, little-endian), then per characteristic a
    /// declaration (value is properties, value handle, characteristic UUID), the value attribute, and
    /// its descriptors. The layout is `[unread]` against the specification.
    ///
    /// # Errors
    /// [`AttributeError::TooManyAttributes`] when handles would pass `0xffff`.
    pub fn attributes(&self) -> Result<Vec<Attribute>, AttributeError> {
        let mut out: Vec<Attribute> = Vec::new();
        for service in &self.services {
            let start = out.len();
            let handle = next_handle(out.len())?;
            out.push(Attribute {
                handle,
                attr_type: match service.kind {
                    ServiceKind::Primary => type_uuids::PRIMARY_SERVICE,
                    ServiceKind::Secondary => type_uuids::SECONDARY_SERVICE,
                },
                value: service.uuid.to_le_bytes(),
                role: AttributeRole::Service { end_handle: handle },
            });
            for ch in &service.characteristics {
                let decl = next_handle(out.len())?;
                let value_handle = next_handle(out.len() + 1)?;
                let mut decl_value = vec![ch.properties.0];
                decl_value.extend_from_slice(&value_handle.to_le_bytes());
                decl_value.extend_from_slice(&ch.uuid.to_le_bytes());
                out.push(Attribute {
                    handle: decl,
                    attr_type: type_uuids::CHARACTERISTIC,
                    value: decl_value,
                    role: AttributeRole::Declaration,
                });
                out.push(Attribute {
                    handle: value_handle,
                    attr_type: ch.uuid,
                    value: ch.value.clone().unwrap_or_default(),
                    role: AttributeRole::Value {
                        properties: ch.properties.0,
                        has_static_value: ch.value.is_some(),
                    },
                });
                for d in &ch.descriptors {
                    let handle = next_handle(out.len())?;
                    out.push(Attribute {
                        handle,
                        attr_type: d.uuid,
                        value: d.value.clone(),
                        role: AttributeRole::Descriptor,
                    });
                }
            }
            let end = u16::try_from(out.len()).map_err(|_| AttributeError::TooManyAttributes)?;
            out[start].role = AttributeRole::Service { end_handle: end };
        }
        Ok(out)
    }
}
