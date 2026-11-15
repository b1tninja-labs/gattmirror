use crate::uuid::Uuid;

/// GATT characteristic properties, as the bit set in a characteristic declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Props(pub u8);

impl Props {
    /// Broadcast.
    pub const BROADCAST: Props = Props(0x01);
    /// Read.
    pub const READ: Props = Props(0x02);
    /// Write without response.
    pub const WRITE_NO_RSP: Props = Props(0x04);
    /// Write.
    pub const WRITE: Props = Props(0x08);
    /// Notify.
    pub const NOTIFY: Props = Props(0x10);
    /// Indicate.
    pub const INDICATE: Props = Props(0x20);

    /// True if every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Props) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Props {
    type Output = Props;
    fn bitor(self, rhs: Props) -> Props {
        Props(self.0 | rhs.0)
    }
}

/// What a descriptor allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    /// Readable.
    pub read: bool,
    /// Writable.
    pub write: bool,
}

impl Access {
    /// Read only.
    pub const READ: Access = Access {
        read: true,
        write: false,
    };
    /// Read and write.
    pub const READ_WRITE: Access = Access {
        read: true,
        write: true,
    };
}

/// Where an attribute's value lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// Held in the database; writes replace it (after the [`Backend`](crate::Backend) accepts them).
    Static(Vec<u8>),
    /// Held by the [`Backend`](crate::Backend): every read asks it.
    Dynamic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A primary service declaration; `end` is the last handle of the service.
    Service {
        end: u16,
    },
    CharDeclaration,
    CharValue,
    /// A Client Characteristic Configuration descriptor for the characteristic whose value is at
    /// `value_handle`; `allowed` is the notify/indicate bits its properties permit.
    Cccd {
        value_handle: u16,
        allowed: u16,
    },
    Descriptor,
}

/// One row of the attribute table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub(crate) handle: u16,
    pub(crate) uuid: Uuid,
    pub(crate) value: Value,
    pub(crate) kind: Kind,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) write_cmd: bool,
}

impl Attribute {
    /// The attribute's handle.
    #[must_use]
    pub fn handle(&self) -> u16 {
        self.handle
    }
    /// The attribute's type.
    #[must_use]
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
    /// Where the value lives.
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }
}

/// Why a database could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// A characteristic or descriptor was added before any service, or a descriptor before any
    /// characteristic.
    NoParent,
    /// The handle space (1..=0xFFFF) is exhausted.
    HandleOverflow,
    /// `set_next_handle` asked for a handle below the next free one, or for 0.
    HandleNotAvailable,
}

/// A GATT attribute table, sorted by handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub(crate) attrs: Vec<Attribute>,
}

impl Database {
    /// Start building a table; handles are assigned from 1 upward.
    #[must_use]
    pub fn builder() -> DatabaseBuilder {
        DatabaseBuilder::default()
    }

    /// Every attribute, in handle order.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attrs
    }

    /// Look an attribute up by handle.
    #[must_use]
    pub fn get(&self, handle: u16) -> Option<&Attribute> {
        self.index(handle).map(|i| &self.attrs[i])
    }

    pub(crate) fn index(&self, handle: u16) -> Option<usize> {
        self.attrs.binary_search_by_key(&handle, |a| a.handle).ok()
    }

    /// The indices of attributes with handles in `start..=end`.
    pub(crate) fn range(&self, start: u16, end: u16) -> std::ops::Range<usize> {
        let lo = self.attrs.partition_point(|a| a.handle < start);
        let hi = self.attrs.partition_point(|a| a.handle <= end);
        lo..hi.max(lo)
    }

    /// The handle of the CCCD belonging to the characteristic value at `value_handle`.
    pub(crate) fn cccd_for(&self, value_handle: u16) -> Option<u16> {
        self.attrs.iter().find_map(|a| match a.kind {
            Kind::Cccd {
                value_handle: v, ..
            } if v == value_handle => Some(a.handle),
            _ => None,
        })
    }
}

/// Builds a [`Database`], assigning consecutive handles.
///
/// Each service takes one handle; each characteristic takes two (declaration, value) plus one for
/// the Client Characteristic Configuration descriptor that is added automatically when the
/// properties include notify or indicate; each descriptor takes one.
#[derive(Debug, Default)]
pub struct DatabaseBuilder {
    attrs: Vec<Attribute>,
    next: u16,
    overflowed: bool,
    in_service: Option<usize>,
    /// Index of the last characteristic value, if the current service has one.
    last_char: Option<usize>,
}

impl DatabaseBuilder {
    fn take_handle(&mut self) -> Result<u16, BuildError> {
        if self.overflowed {
            return Err(BuildError::HandleOverflow);
        }
        let h = self.next.max(1);
        match h.checked_add(1) {
            Some(n) => self.next = n,
            None => {
                // 0xFFFF is the last handle; mark the space used up after handing it out.
                self.overflowed = true;
            }
        }
        Ok(h)
    }

    /// Make the next attribute get `handle` instead of the next consecutive one, so a mirror can
    /// keep a real device's handles. The handle must not be below the next free handle.
    ///
    /// # Errors
    /// [`BuildError::HandleNotAvailable`] for 0 or a handle already passed.
    pub fn set_next_handle(&mut self, handle: u16) -> Result<&mut Self, BuildError> {
        if handle == 0 || handle < self.next.max(1) || self.overflowed {
            return Err(BuildError::HandleNotAvailable);
        }
        self.next = handle;
        Ok(self)
    }

    /// Open a primary service. Returns its declaration handle.
    ///
    /// # Errors
    /// [`BuildError::HandleOverflow`].
    pub fn service(&mut self, uuid: Uuid) -> Result<u16, BuildError> {
        let handle = self.take_handle()?;
        self.in_service = Some(self.attrs.len());
        self.last_char = None;
        self.attrs.push(Attribute {
            handle,
            uuid: Uuid::from_u16(0x2800),
            value: Value::Static(uuid.to_le_bytes()),
            kind: Kind::Service { end: handle },
            read: true,
            write: false,
            write_cmd: false,
        });
        Ok(handle)
    }

    /// Add a characteristic to the open service. Returns the handle of its *value*.
    ///
    /// # Errors
    /// [`BuildError::NoParent`] before any service; [`BuildError::HandleOverflow`].
    pub fn characteristic(
        &mut self,
        uuid: Uuid,
        props: Props,
        value: Value,
    ) -> Result<u16, BuildError> {
        if self.in_service.is_none() {
            return Err(BuildError::NoParent);
        }
        let decl = self.take_handle()?;
        let vh = self.take_handle()?;
        let mut decl_value = vec![props.0];
        decl_value.extend_from_slice(&vh.to_le_bytes());
        decl_value.extend_from_slice(&uuid.to_le_bytes());
        self.attrs.push(Attribute {
            handle: decl,
            uuid: Uuid::from_u16(0x2803),
            value: Value::Static(decl_value),
            kind: Kind::CharDeclaration,
            read: true,
            write: false,
            write_cmd: false,
        });
        self.last_char = Some(self.attrs.len());
        self.attrs.push(Attribute {
            handle: vh,
            uuid,
            value,
            kind: Kind::CharValue,
            read: props.contains(Props::READ),
            write: props.contains(Props::WRITE),
            write_cmd: props.contains(Props::WRITE_NO_RSP),
        });
        let mut allowed = 0;
        if props.contains(Props::NOTIFY) {
            allowed |= 1;
        }
        if props.contains(Props::INDICATE) {
            allowed |= 2;
        }
        if allowed != 0 {
            let h = self.take_handle()?;
            self.attrs.push(Attribute {
                handle: h,
                uuid: Uuid::from_u16(0x2902),
                value: Value::Static(vec![0, 0]),
                kind: Kind::Cccd {
                    value_handle: vh,
                    allowed,
                },
                read: true,
                write: true,
                write_cmd: false,
            });
        }
        self.close_group();
        Ok(vh)
    }

    /// Add a descriptor to the last characteristic. Returns its handle. Do not add a Client
    /// Characteristic Configuration (0x2902) by hand: one is added for notify and indicate.
    ///
    /// # Errors
    /// [`BuildError::NoParent`] before any characteristic; [`BuildError::HandleOverflow`].
    pub fn descriptor(
        &mut self,
        uuid: Uuid,
        access: Access,
        value: Value,
    ) -> Result<u16, BuildError> {
        if self.last_char.is_none() {
            return Err(BuildError::NoParent);
        }
        let h = self.take_handle()?;
        self.attrs.push(Attribute {
            handle: h,
            uuid,
            value,
            kind: Kind::Descriptor,
            read: access.read,
            write: access.write,
            write_cmd: false,
        });
        self.close_group();
        Ok(h)
    }

    /// Record the open service's last handle.
    fn close_group(&mut self) {
        if let Some(i) = self.in_service {
            let last = self.attrs.last().map_or(0, |a| a.handle);
            self.attrs[i].kind = Kind::Service { end: last };
        }
    }

    /// Finish.
    #[must_use]
    pub fn build(self) -> Database {
        Database { attrs: self.attrs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Database {
        let mut b = Database::builder();
        b.service(Uuid::from_u16(0x1800)).unwrap();
        b.characteristic(
            Uuid::from_u16(0x2A00),
            Props::READ,
            Value::Static(b"dev".to_vec()),
        )
        .unwrap();
        b.service(Uuid::from_u16(0xFFF0)).unwrap();
        b.characteristic(
            Uuid::from_u16(0xFFF1),
            Props::NOTIFY | Props::WRITE,
            Value::Dynamic,
        )
        .unwrap();
        b.descriptor(
            Uuid::from_u16(0x2901),
            Access::READ,
            Value::Static(b"x".to_vec()),
        )
        .unwrap();
        b.build()
    }

    #[test]
    fn handles_are_consecutive_and_cccd_is_added() {
        let db = sample();
        let hs: Vec<u16> = db.attributes().iter().map(Attribute::handle).collect();
        // svc, decl, value | svc, decl, value, cccd, user-descriptor
        assert_eq!(hs, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(db.get(7).unwrap().uuid(), Uuid::from_u16(0x2902));
    }

    #[test]
    fn service_group_ends_at_its_last_attribute() {
        let db = sample();
        assert_eq!(db.get(1).unwrap().kind, Kind::Service { end: 3 });
        assert_eq!(db.get(4).unwrap().kind, Kind::Service { end: 8 });
    }

    #[test]
    fn declaration_value_names_properties_value_handle_and_uuid() {
        let db = sample();
        assert_eq!(
            db.get(2).unwrap().value,
            Value::Static(vec![0x02, 3, 0, 0x00, 0x2A])
        );
    }

    #[test]
    fn orphans_are_refused() {
        let mut b = Database::builder();
        assert_eq!(
            b.characteristic(Uuid::from_u16(1), Props::READ, Value::Dynamic),
            Err(BuildError::NoParent)
        );
        b.service(Uuid::from_u16(1)).unwrap();
        assert_eq!(
            b.descriptor(Uuid::from_u16(2), Access::READ, Value::Dynamic),
            Err(BuildError::NoParent)
        );
    }

    #[test]
    fn explicit_handles_keep_a_real_devices_numbering() {
        let mut b = Database::builder();
        b.set_next_handle(0x10).unwrap();
        assert_eq!(b.service(Uuid::from_u16(1)).unwrap(), 0x10);
        assert_eq!(
            b.set_next_handle(0x10).err(),
            Some(BuildError::HandleNotAvailable)
        );
        assert_eq!(
            b.set_next_handle(0).err(),
            Some(BuildError::HandleNotAvailable)
        );
        b.set_next_handle(0x20).unwrap();
        assert_eq!(
            b.characteristic(Uuid::from_u16(2), Props::READ, Value::Dynamic)
                .unwrap(),
            0x21
        );
        assert_eq!(
            b.build().get(0x10).unwrap().kind,
            Kind::Service { end: 0x21 }
        );
    }

    #[test]
    fn the_last_handle_is_usable_and_then_the_space_is_full() {
        let mut b = Database::builder();
        b.set_next_handle(0xFFFE).unwrap();
        assert_eq!(b.service(Uuid::from_u16(1)).unwrap(), 0xFFFE);
        assert_eq!(
            b.characteristic(Uuid::from_u16(2), Props::READ, Value::Dynamic),
            Err(BuildError::HandleOverflow)
        );
    }

    #[test]
    fn range_is_inclusive_and_tolerates_gaps() {
        let db = sample();
        assert_eq!(db.range(2, 4), 1..4);
        assert_eq!(db.range(9, 20), 8..8);
        assert_eq!(db.range(5, 2), 4..4);
    }
}
