use std::collections::{BTreeMap, VecDeque};

use crate::DEFAULT_MTU;
use crate::db::{Database, Kind, Level, Value};
use crate::pdu::{DecodeError, ErrorCode, GroupEntry, Pdu, opcode};
use crate::uuid::Uuid;

/// The longest attribute value ATT allows.
const MAX_VALUE_LEN: usize = 512;
/// Prepare Write parts a client may queue before Prepare Queue Full.
const MAX_PREPARED_PARTS: usize = 64;
/// Bytes of prepare-write payload held per connection (four maximum-length values).
const MAX_PREPARED_BYTES: usize = 4 * MAX_VALUE_LEN;
/// Notifications and indications waiting for the transport; beyond it `notify` fails.
const MAX_OUTGOING: usize = 256;
/// How long a transport should wait for an indication confirmation before treating the
/// connection as lost (Bluetooth Core Specification 6.3 Vol 3 Part F §3.3.3: a transaction not
/// completed within 30 seconds times out, and the bearer carries no further PDUs).
pub const CONFIRMATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// A CCCD write with bits the characteristic's properties do not allow. Core 6.3 Vol 3 Part G
/// §3.3.3.3 forbids the bits but names no error; 0xFD is a Common Profile and Service Error Code
/// (the range Part F Table 3.43 allows for writes) that profiles define as "Client Characteristic
/// Configuration Descriptor Improperly Configured", so using it here is a choice, not a rule.
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

    /// The client confirmed the indication that was outstanding for `handle`.
    fn indication_confirmed(&mut self, handle: u16) {
        let _ = handle;
    }

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
    /// Too many updates are already waiting for the transport.
    QueueFull,
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
    cccd: BTreeMap<u16, u16>,
    prepared: Vec<Prepared>,
    prepared_bytes: usize,
    link_level: Level,
    outgoing: VecDeque<Outgoing>,
    pending_indication: Option<u16>,
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
            cccd: BTreeMap::new(),
            prepared: Vec::new(),
            prepared_bytes: 0,
            link_level: Level::Open,
            outgoing: VecDeque::new(),
            pending_indication: None,
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
        self.cccd.clear();
        self.prepared.clear();
        self.outgoing.clear();
        self.pending_indication = None;
        self.prepared_bytes = 0;
        self.link_level = Level::Open;
    }

    /// True between sending an indication and receiving its confirmation.
    pub fn indication_pending(&self) -> bool {
        self.pending_indication.is_some()
    }

    /// The value handle of the indication awaiting its confirmation, if any. A transport that
    /// sees it unchanged for [`CONFIRMATION_TIMEOUT`] should drop the connection.
    pub fn pending_indication(&self) -> Option<u16> {
        self.pending_indication
    }

    /// Tell the server how well the link is protected; attributes with a higher
    /// [`require`](crate::DatabaseBuilder::require)ment are refused until it is raised.
    pub fn set_link_level(&mut self, level: Level) {
        self.link_level = level;
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
        if self.outgoing.len() >= MAX_OUTGOING {
            return Err(SendError::QueueFull);
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
            .position(|o| !o.indicate || self.pending_indication.is_none())?;
        let o = self.outgoing.remove(pos)?;
        let mut value = o.value;
        value.truncate(usize::from(self.mtu) - 3);
        if o.indicate {
            self.pending_indication = Some(o.handle);
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
            Pdu::ReadMultipleReq { handles } => self.read_multiple(handles),
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
                if let Some(h) = self.pending_indication.take() {
                    self.backend.indication_confirmed(h);
                }
                return None;
            }
            _ => return None,
        })
    }

    fn max_payload(&self) -> usize {
        usize::from(self.mtu)
    }

    fn exchange_mtu(&mut self, client: u16) -> Pdu {
        self.mtu = client.max(DEFAULT_MTU).min(self.max_mtu);
        Pdu::MtuRsp { mtu: self.max_mtu }
    }

    /// The value of the attribute at `idx`, for a read.
    fn read_attr(&mut self, idx: usize) -> Result<Vec<u8>, ErrorCode> {
        let a = &self.db.attrs[idx];
        // Authentication and authorization come before any other check (Vol 3 Part F §4).
        self.check_level(a.read_level)?;
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

    fn read_multiple(&mut self, handles: &[u16]) -> Pdu {
        const REQ: u8 = opcode::READ_MULTIPLE_REQ;
        let mut values = Vec::new();
        for &h in handles {
            let Some(idx) = self.db.index(h) else {
                return err(REQ, h, ErrorCode::InvalidHandle);
            };
            match self.read_attr(idx) {
                Ok(v) => values.extend_from_slice(&v),
                Err(e) => return err(REQ, h, e),
            }
        }
        // Every handle is checked before the concatenation is cut.
        values.truncate(self.max_payload() - 1);
        Pdu::ReadMultipleRsp { values }
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

    fn find_by_type_value(&mut self, start: u16, end: u16, attr_type: u16, value: &[u8]) -> Pdu {
        const REQ: u8 = opcode::FIND_BY_TYPE_VALUE_REQ;
        if start == 0 || start > end {
            return err(REQ, start, ErrorCode::InvalidHandle);
        }
        let want = Uuid::from_u16(attr_type);
        let candidates: Vec<usize> = self
            .db
            .range(start, end)
            .filter(|&i| self.db.attrs[i].uuid == want)
            .collect();
        let room = (self.max_payload() - 1) / 4;
        let mut ranges: Vec<(u16, u16)> = Vec::new();
        for i in candidates {
            if ranges.len() >= room {
                break;
            }
            // Only attributes the client may read are considered, and a refusal is skipped, never
            // reported: this request must not answer with a security error.
            if self.read_attr(i).is_ok_and(|v| v == value) {
                let a = &self.db.attrs[i];
                let last = match a.kind {
                    Kind::Service { end } => end,
                    _ => a.handle,
                };
                ranges.push((a.handle, last));
            }
        }
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
        if group_type == Uuid::from_u16(0x2801) {
            // A grouping type the protocol allows; this server has no secondary services.
            return err(REQ, start, ErrorCode::AttributeNotFound);
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
        // Security first, as for reads.
        self.check_level(a.write_level)?;
        if (with_response && a.write) || (!with_response && a.write_cmd) {
            Ok(idx)
        } else {
            Err(ErrorCode::WriteNotPermitted)
        }
    }

    /// Insufficient Authentication if the attribute needs an authenticated link, Insufficient
    /// Encryption if only encryption is missing. When nothing is encrypted and authentication is
    /// required both apply, and Part F §3.4.1.1 leaves the choice to the server.
    fn check_level(&self, needed: Level) -> Result<(), ErrorCode> {
        if self.link_level >= needed {
            Ok(())
        } else if needed == Level::Authenticated {
            Err(ErrorCode::InsufficientAuthentication)
        } else {
            Err(ErrorCode::InsufficientEncryption)
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
        // The offset and the total length are validated when the queue is executed (Vol 3 Part F
        // §3.4.6.1); only the echo, which must fit one PDU, is refused here.
        if value.len() + 5 > self.max_payload() {
            return err(REQ, handle, ErrorCode::InvalidAttributeValueLength);
        }
        if self.prepared.len() >= MAX_PREPARED_PARTS
            || self.prepared_bytes + value.len() > MAX_PREPARED_BYTES
        {
            return err(REQ, handle, ErrorCode::PrepareQueueFull);
        }
        self.prepared_bytes += value.len();
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
        self.prepared_bytes = 0;
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
            | opcode::READ_MULTIPLE_REQ
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
