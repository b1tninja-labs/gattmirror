use std::collections::{BTreeMap, VecDeque};

use crate::DEFAULT_MTU;
use crate::db::{Database, Kind, Value};
use crate::pdu::{DecodeError, ErrorCode, GroupEntry, Pdu, opcode};
use crate::uuid::Uuid;

/// The longest attribute value ATT allows.
const MAX_VALUE_LEN: usize = 512;
/// A CCCD write with bits the characteristic's properties do not allow. The code is from memory
/// of the Core Specification Supplement and is `[unread]`.
const CCCD_IMPROPERLY_CONFIGURED: u8 = 0xFD;

/// A client's Client Characteristic Configuration for one characteristic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Subscription {
    /// Notifications are enabled.
    pub notify: bool,
    /// Indications are enabled.
    pub indicate: bool,
}

impl Subscription {
    fn from_bits(b: u16) -> Self {
        Subscription {
            notify: b & 1 != 0,
            indicate: b & 2 != 0,
        }
    }
}

/// What the server asks of whatever stands behind the table: a real device, a simulator, a test.
///
/// The server calls it for [`Value::Dynamic`] attributes on reads, and for every write to a writable
/// attribute other than a CCCD (the server keeps those itself and reports them through
/// [`Backend::subscription_changed`]).
pub trait Backend {
    /// The current value of a dynamic attribute. The server slices it for the MTU and for Read Blob.
    ///
    /// # Errors
    /// Any ATT error; it is sent to the client as is.
    fn read(&mut self, handle: u16) -> Result<Vec<u8>, ErrorCode>;

    /// A client wrote `value` to `handle`. `with_response` is false for a Write Command, whose
    /// error (if any) is dropped because a command has no response.
    ///
    /// # Errors
    /// Any ATT error; for a Write Request it is sent to the client, and the database is left
    /// unchanged.
    fn write(&mut self, handle: u16, value: &[u8], with_response: bool) -> Result<(), ErrorCode>;

    /// The client changed its subscription to the characteristic whose value is at `value_handle`.
    fn subscription_changed(&mut self, value_handle: u16, subscription: Subscription) {
        let _ = (value_handle, subscription);
    }
}

/// A backend that serves nothing: dynamic reads and all writes are refused.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBackend;

impl Backend for NoBackend {
    fn read(&mut self, _handle: u16) -> Result<Vec<u8>, ErrorCode> {
        Err(ErrorCode::ReadNotPermitted)
    }
    fn write(
        &mut self,
        _handle: u16,
        _value: &[u8],
        _with_response: bool,
    ) -> Result<(), ErrorCode> {
        Err(ErrorCode::WriteNotPermitted)
    }
}

/// Why [`Server::notify`] or [`Server::indicate`] did not queue anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The handle is not a characteristic value with a Client Characteristic Configuration.
    NotSubscribable,
    /// The client has not enabled that kind of update.
    NotSubscribed,
    /// The value is longer than the maximum attribute length.
    TooLong,
}

#[derive(Debug)]
struct Outgoing {
    handle: u16,
    value: Vec<u8>,
    indicate: bool,
}

#[derive(Debug)]
struct Prepared {
    handle: u16,
    offset: u16,
    value: Vec<u8>,
}

/// The server side of one ATT connection.
///
/// It holds its own copy of the [`Database`], the negotiated MTU, the client's subscriptions, the
/// prepare-write queue and the outgoing notification queue. Create one per connection, and call
/// [`Server::reset`] (or drop it) on disconnect.
#[derive(Debug)]
pub struct Server<B> {
    db: Database,
    backend: B,
    max_mtu: u16,
    mtu: u16,
    mtu_exchanged: bool,
    cccd: BTreeMap<u16, u16>,
    prepared: Vec<Prepared>,
    prepare_limit: usize,
    outgoing: VecDeque<Outgoing>,
    indication_pending: bool,
}

fn err(request: u8, handle: u16, code: ErrorCode) -> Pdu {
    Pdu::ErrorRsp {
        request,
        handle,
        code,
    }
}

impl<B: Backend> Server<B> {
    /// A server over `db`. `max_mtu` is the largest MTU it will accept (at least 23).
    pub fn new(db: Database, backend: B, max_mtu: u16) -> Self {
        Server {
            db,
            backend,
            max_mtu: max_mtu.max(DEFAULT_MTU),
            mtu: DEFAULT_MTU,
            mtu_exchanged: false,
            cccd: BTreeMap::new(),
            prepared: Vec::new(),
            prepare_limit: 64,
            outgoing: VecDeque::new(),
            indication_pending: false,
        }
    }

    /// The attribute table.
    pub fn database(&self) -> &Database {
        &self.db
    }

    /// The backend, to feed it values from the real device.
    pub fn backend(&mut self) -> &mut B {
        &mut self.backend
    }

    /// The MTU in force for this connection.
    pub fn mtu(&self) -> u16 {
        self.mtu
    }

    /// The client's subscription to the characteristic whose value is at `value_handle`.
    pub fn subscription(&self, value_handle: u16) -> Subscription {
        self.db
            .cccd_for(value_handle)
            .and_then(|h| self.cccd.get(&h))
            .map_or_else(Subscription::default, |b| Subscription::from_bits(*b))
    }

    /// Forget everything about the client: MTU, subscriptions, queued writes and updates.
    pub fn reset(&mut self) {
        self.mtu = DEFAULT_MTU;
        self.mtu_exchanged = false;
        self.cccd.clear();
        self.prepared.clear();
        self.outgoing.clear();
        self.indication_pending = false;
    }

    /// True between sending an indication and receiving its confirmation.
    pub fn indication_pending(&self) -> bool {
        self.indication_pending
    }

    /// Queue a notification for the characteristic whose value is at `value_handle`.
    ///
    /// # Errors
    /// [`SendError`] if the handle has no CCCD, the client has not enabled notifications, or the
    /// value is over 512 bytes.
    pub fn notify(&mut self, value_handle: u16, value: &[u8]) -> Result<(), SendError> {
        self.queue(value_handle, value, false)
    }

    /// Queue an indication. Only one is outstanding at a time; the rest wait in the queue until
    /// the confirmation arrives.
    ///
    /// # Errors
    /// As [`Server::notify`], for indications.
    pub fn indicate(&mut self, value_handle: u16, value: &[u8]) -> Result<(), SendError> {
        self.queue(value_handle, value, true)
    }

    fn queue(&mut self, handle: u16, value: &[u8], indicate: bool) -> Result<(), SendError> {
        if value.len() > MAX_VALUE_LEN {
            return Err(SendError::TooLong);
        }
        let Some(cccd) = self.db.cccd_for(handle) else {
            return Err(SendError::NotSubscribable);
        };
        let bits = self.cccd.get(&cccd).copied().unwrap_or(0);
        let wanted = if indicate { 2 } else { 1 };
        if bits & wanted == 0 {
            return Err(SendError::NotSubscribed);
        }
        self.outgoing.push_back(Outgoing {
            handle,
            value: value.to_vec(),
            indicate,
        });
        Ok(())
    }

    /// The next notification or indication to send, if any. A value longer than the MTU allows
    /// (MTU - 3) is cut to fit. An indication is held back while another is unconfirmed.
    pub fn poll_outgoing(&mut self) -> Option<Pdu> {
        let pos = self
            .outgoing
            .iter()
            .position(|o| !o.indicate || !self.indication_pending)?;
        let o = self.outgoing.remove(pos)?;
        let mut value = o.value;
        value.truncate(usize::from(self.mtu) - 3);
        if o.indicate {
            self.indication_pending = true;
            Some(Pdu::Indication {
                handle: o.handle,
                value,
            })
        } else {
            Some(Pdu::Notification {
                handle: o.handle,
                value,
            })
        }
    }

    /// Answer wire bytes with wire bytes. Bytes that do not decode are answered like any other bad
    /// request: Invalid PDU for a known request opcode, Request Not Supported for an unknown
    /// opcode that is not a command, nothing otherwise.
    pub fn handle_bytes(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        match Pdu::decode(bytes) {
            Ok(pdu) => self.handle(&pdu).map(|p| p.encode()),
            Err(DecodeError::Empty) => None,
            Err(DecodeError::Malformed(o)) => {
                is_request_opcode(o).then(|| err(o, 0, ErrorCode::InvalidPdu).encode())
            }
            Err(DecodeError::UnknownOpcode(o)) => {
                // Bit 6 marks a command; bit 7 marks an authentication signature. A command never
                // gets a response. Odd opcodes and the confirmation are responses or events.
                let command = o & 0x40 != 0;
                (!command && o & 1 == 0).then(|| err(o, 0, ErrorCode::RequestNotSupported).encode())
            }
        }
    }

    /// Answer one PDU. Requests get `Some(response)`; commands, confirmations and any PDU a server
    /// does not expect get `None`.
    pub fn handle(&mut self, pdu: &Pdu) -> Option<Pdu> {
        Some(match pdu {
            Pdu::MtuReq { mtu } => self.exchange_mtu(*mtu),
            Pdu::FindInfoReq { start, end } => self.find_info(*start, *end),
            Pdu::FindByTypeValueReq {
                start,
                end,
                attr_type,
                value,
            } => self.find_by_type_value(*start, *end, *attr_type, value),
            Pdu::ReadByTypeReq {
                start,
                end,
                attr_type,
            } => self.read_by_type(*start, *end, *attr_type),
            Pdu::ReadReq { handle } => self.read(*handle, 0, opcode::READ_REQ),
            Pdu::ReadBlobReq { handle, offset } => {
                self.read(*handle, usize::from(*offset), opcode::READ_BLOB_REQ)
            }
            Pdu::ReadByGroupTypeReq {
                start,
                end,
                group_type,
            } => self.read_by_group_type(*start, *end, *group_type),
            Pdu::WriteReq { handle, value } => match self.write(*handle, value, true) {
                Ok(()) => Pdu::WriteRsp,
                Err(e) => err(opcode::WRITE_REQ, *handle, e),
            },
            Pdu::WriteCmd { handle, value } => {
                let _ = self.write(*handle, value, false);
                return None;
            }
            Pdu::PrepareWriteReq {
                handle,
                offset,
                value,
            } => self.prepare_write(*handle, *offset, value),
            Pdu::ExecuteWriteReq { flags } => self.execute_write(*flags),
            Pdu::Confirmation => {
                self.indication_pending = false;
                return None;
            }
            _ => return None,
        })
    }

    fn max_payload(&self) -> usize {
        usize::from(self.mtu)
    }

    fn exchange_mtu(&mut self, client: u16) -> Pdu {
        if self.mtu_exchanged {
            return err(opcode::MTU_REQ, 0, ErrorCode::RequestNotSupported);
        }
        self.mtu_exchanged = true;
        self.mtu = client.max(DEFAULT_MTU).min(self.max_mtu);
        Pdu::MtuRsp { mtu: self.max_mtu }
    }

    /// The value of the attribute at `idx`, for a read.
    fn read_attr(&mut self, idx: usize) -> Result<Vec<u8>, ErrorCode> {
        let a = &self.db.attrs[idx];
        if !a.read {
            return Err(ErrorCode::ReadNotPermitted);
        }
        let handle = a.handle;
        match (&a.kind, &a.value) {
            (Kind::Cccd { .. }, _) => Ok(self
                .cccd
                .get(&handle)
                .copied()
                .unwrap_or(0)
                .to_le_bytes()
                .to_vec()),
            (_, Value::Static(v)) => Ok(v.clone()),
            (_, Value::Dynamic) => self.backend.read(handle),
        }
    }

    fn read(&mut self, handle: u16, offset: usize, request: u8) -> Pdu {
        let Some(idx) = self.db.index(handle) else {
            return err(request, handle, ErrorCode::InvalidHandle);
        };
        let value = match self.read_attr(idx) {
            Ok(v) => v,
            Err(e) => return err(request, handle, e),
        };
        if offset > value.len() {
            return err(request, handle, ErrorCode::InvalidOffset);
        }
        let end = value.len().min(offset + self.max_payload() - 1);
        let part = value[offset..end].to_vec();
        if request == opcode::READ_REQ {
            Pdu::ReadRsp { value: part }
        } else {
            Pdu::ReadBlobRsp { value: part }
        }
    }

    fn find_info(&self, start: u16, end: u16) -> Pdu {
        const REQ: u8 = opcode::FIND_INFO_REQ;
        if start == 0 || start > end {
            return err(REQ, start, ErrorCode::InvalidHandle);
        }
        let range = self.db.range(start, end);
        let Some(first) = self.db.attrs[range.clone()].first() else {
            return err(REQ, start, ErrorCode::AttributeNotFound);
        };
        let wide = first.uuid.wire_len() == 16;
        let per = if wide { 18 } else { 4 };
        let room = (self.max_payload() - 2) / per;
        let entries = self.db.attrs[range]
            .iter()
            .take_while(|a| (a.uuid.wire_len() == 16) == wide)
            .take(room)
            .map(|a| (a.handle, a.uuid))
            .collect();
        Pdu::FindInfoRsp { entries }
    }

    fn find_by_type_value(&self, start: u16, end: u16, attr_type: u16, value: &[u8]) -> Pdu {
        const REQ: u8 = opcode::FIND_BY_TYPE_VALUE_REQ;
        if start == 0 || start > end {
            return err(REQ, start, ErrorCode::InvalidHandle);
        }
        let want = Uuid::from_u16(attr_type);
        let ranges: Vec<(u16, u16)> = self.db.attrs[self.db.range(start, end)]
            .iter()
            .filter(|a| a.uuid == want && matches!(&a.value, Value::Static(v) if v == value))
            .map(|a| {
                let last = match a.kind {
                    Kind::Service { end } => end,
                    _ => a.handle,
                };
                (a.handle, last)
            })
            .take((self.max_payload() - 1) / 4)
            .collect();
        if ranges.is_empty() {
            err(REQ, start, ErrorCode::AttributeNotFound)
        } else {
            Pdu::FindByTypeValueRsp { ranges }
        }
    }

    fn read_by_type(&mut self, start: u16, end: u16, attr_type: Uuid) -> Pdu {
        const REQ: u8 = opcode::READ_BY_TYPE_REQ;
        if start == 0 || start > end {
            return err(REQ, start, ErrorCode::InvalidHandle);
        }
        let matches: Vec<usize> = self
            .db
            .range(start, end)
            .filter(|&i| self.db.attrs[i].uuid == attr_type)
            .collect();
        if matches.is_empty() {
            return err(REQ, start, ErrorCode::AttributeNotFound);
        }
        // Each entry is a handle and a value; the first value is cut to fit, and later entries
        // must be the same length.
        let cap = (self.max_payload() - 4).min(253);
        let budget = self.max_payload() - 2;
        let mut entries: Vec<(u16, Vec<u8>)> = Vec::new();
        let mut used = 0;
        for i in matches {
            let handle = self.db.attrs[i].handle;
            let mut v = match self.read_attr(i) {
                Ok(v) => v,
                Err(e) if entries.is_empty() => return err(REQ, handle, e),
                Err(_) => break,
            };
            v.truncate(cap);
            if entries.first().is_some_and(|(_, f)| f.len() != v.len())
                || used + 2 + v.len() > budget
            {
                break;
            }
            used += 2 + v.len();
            entries.push((handle, v));
        }
        Pdu::ReadByTypeRsp { entries }
    }

    fn read_by_group_type(&self, start: u16, end: u16, group_type: Uuid) -> Pdu {
        const REQ: u8 = opcode::READ_BY_GROUP_TYPE_REQ;
        if start == 0 || start > end {
            return err(REQ, start, ErrorCode::InvalidHandle);
        }
        if group_type != Uuid::from_u16(0x2800) {
            return err(REQ, start, ErrorCode::UnsupportedGroupType);
        }
        let budget = self.max_payload() - 2;
        let mut entries: Vec<GroupEntry> = Vec::new();
        let mut used = 0;
        for a in &self.db.attrs[self.db.range(start, end)] {
            let (Kind::Service { end }, Value::Static(v)) = (&a.kind, &a.value) else {
                continue;
            };
            if entries.first().is_some_and(|f| f.value.len() != v.len())
                || used + 4 + v.len() > budget
            {
                break;
            }
            used += 4 + v.len();
            entries.push(GroupEntry {
                start: a.handle,
                end: *end,
                value: v.clone(),
            });
        }
        if entries.is_empty() {
            err(REQ, start, ErrorCode::AttributeNotFound)
        } else {
            Pdu::ReadByGroupTypeRsp { entries }
        }
    }

    /// Check that a write is allowed, without performing it.
    fn check_writable(&self, handle: u16, with_response: bool) -> Result<usize, ErrorCode> {
        let idx = self.db.index(handle).ok_or(ErrorCode::InvalidHandle)?;
        let a = &self.db.attrs[idx];
        if (with_response && a.write) || (!with_response && a.write_cmd) {
            Ok(idx)
        } else {
            Err(ErrorCode::WriteNotPermitted)
        }
    }

    fn write(&mut self, handle: u16, value: &[u8], with_response: bool) -> Result<(), ErrorCode> {
        let idx = self.check_writable(handle, with_response)?;
        if value.len() > MAX_VALUE_LEN {
            return Err(ErrorCode::InvalidAttributeValueLength);
        }
        if let Kind::Cccd {
            value_handle,
            allowed,
        } = self.db.attrs[idx].kind
        {
            let [lo, hi] =
                <[u8; 2]>::try_from(value).map_err(|_| ErrorCode::InvalidAttributeValueLength)?;
            let bits = u16::from_le_bytes([lo, hi]);
            if bits & !allowed != 0 {
                return Err(ErrorCode::Other(CCCD_IMPROPERLY_CONFIGURED));
            }
            self.cccd.insert(handle, bits);
            if bits & 1 == 0 {
                self.outgoing
                    .retain(|o| o.handle != value_handle || o.indicate);
            }
            if bits & 2 == 0 {
                self.outgoing
                    .retain(|o| o.handle != value_handle || !o.indicate);
            }
            self.backend
                .subscription_changed(value_handle, Subscription::from_bits(bits));
            return Ok(());
        }
        self.backend.write(handle, value, with_response)?;
        if let Value::Static(v) = &mut self.db.attrs[idx].value {
            *v = value.to_vec();
        }
        Ok(())
    }

    fn prepare_write(&mut self, handle: u16, offset: u16, value: &[u8]) -> Pdu {
        const REQ: u8 = opcode::PREPARE_WRITE_REQ;
        if let Err(e) = self.check_writable(handle, true) {
            return err(REQ, handle, e);
        }
        if self.prepared.len() >= self.prepare_limit {
            return err(REQ, handle, ErrorCode::PrepareQueueFull);
        }
        self.prepared.push(Prepared {
            handle,
            offset,
            value: value.to_vec(),
        });
        Pdu::PrepareWriteRsp {
            handle,
            offset,
            value: value.to_vec(),
        }
    }

    /// Flag 1 applies the queue, flag 0 discards it. Queued parts are grouped by handle in order of
    /// first appearance and laid down by offset into an empty buffer; a part that leaves a gap
    /// fails with Invalid Offset. A write over only part of an existing value is not supported.
    fn execute_write(&mut self, flags: u8) -> Pdu {
        const REQ: u8 = opcode::EXECUTE_WRITE_REQ;
        let queue = std::mem::take(&mut self.prepared);
        match flags {
            0 => return Pdu::ExecuteWriteRsp,
            1 => {}
            _ => return err(REQ, 0, ErrorCode::InvalidPdu),
        }
        let mut order: Vec<u16> = Vec::new();
        for p in &queue {
            if !order.contains(&p.handle) {
                order.push(p.handle);
            }
        }
        let mut assembled: Vec<(u16, Vec<u8>)> = Vec::new();
        for handle in order {
            let mut buf: Vec<u8> = Vec::new();
            for p in queue.iter().filter(|p| p.handle == handle) {
                let off = usize::from(p.offset);
                if off > buf.len() {
                    return err(REQ, handle, ErrorCode::InvalidOffset);
                }
                buf.truncate(off);
                buf.extend_from_slice(&p.value);
            }
            assembled.push((handle, buf));
        }
        for (handle, buf) in assembled {
            if let Err(e) = self.write(handle, &buf, true) {
                return err(REQ, handle, e);
            }
        }
        Pdu::ExecuteWriteRsp
    }
}

/// Opcodes of the requests a server answers (not commands).
fn is_request_opcode(o: u8) -> bool {
    matches!(
        o,
        opcode::MTU_REQ
            | opcode::FIND_INFO_REQ
            | opcode::FIND_BY_TYPE_VALUE_REQ
            | opcode::READ_BY_TYPE_REQ
            | opcode::READ_REQ
            | opcode::READ_BLOB_REQ
            | opcode::READ_BY_GROUP_TYPE_REQ
            | opcode::WRITE_REQ
            | opcode::PREPARE_WRITE_REQ
            | opcode::EXECUTE_WRITE_REQ
    )
}
