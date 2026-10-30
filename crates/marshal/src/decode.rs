//! The strict decoder.
//!
//! Shared objects are the subtle part. A flagged object takes a slot in the
//! shared table when it *starts* (before its children are read, matching
//! `Marshal.cpp`) and the slot is filled once the object is complete. A
//! reference to a slot that isn't filled yet is refused.

use crate::adler::adler32;
use crate::string_table::STRING_TABLE;
use crate::tag;
use crate::value::{Long, Reduce, Value};
use crate::{MarshalError, MAX_DEPTH};

/// Upper bound on the size of a decoded tree, in nodes plus payload bytes.
///
/// A stream can repeat a shared object any number of times, so a small file
/// could otherwise expand without limit. Real settings files weigh around
/// 1.5 million.
pub const MAX_DECODED_WEIGHT: u64 = 16_000_000;

/// A decoded stream: the value plus facts about how it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub value: Value,
    /// Format version from the header. The client writes 0.
    pub version: u8,
    pub had_checksum: bool,
    /// Content bytes left unread after the top-level value.
    pub trailing_bytes: usize,
    /// Shared-table slots the header declared but no object claimed.
    pub unused_shared_slots: usize,
    /// How many strings came from the built-in string table.
    pub string_table_uses: usize,
}

/// Decodes a complete marshal stream.
///
/// Trailing bytes and unused shared slots are reported, not refused, because
/// the client itself tolerates them; callers that want a clean file should
/// check [`Decoded::trailing_bytes`] and [`Decoded::unused_shared_slots`].
pub fn decode(buf: &[u8]) -> Result<Decoded, MarshalError> {
    let mut reader = Reader::new(buf);
    reader.header()?;
    let value = reader.obj()?;
    if let Some(checksum) = reader.checksum {
        if reader.version > 0 {
            reader.verify_checksum(checksum, reader.pos)?;
        }
    }
    Ok(Decoded {
        value,
        version: reader.version,
        had_checksum: reader.checksum.is_some(),
        trailing_bytes: reader.end - reader.pos,
        unused_shared_slots: reader.mapping.len() - reader.shared_seen,
        string_table_uses: reader.string_table_uses,
    })
}

struct Slot {
    value: Value,
    weight: u64,
}

#[derive(Clone, Copy)]
struct Checksum {
    /// Offset of the first byte the checksum covers.
    start: usize,
    expected: u32,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Where content ends. In version 0 the shared map follows it.
    end: usize,
    version: u8,
    shared: Vec<Option<Slot>>,
    /// Version 0 only: the slot index for the n-th shared object.
    mapping: Vec<usize>,
    shared_seen: usize,
    depth: usize,
    checksum: Option<Checksum>,
    string_table_uses: usize,
    /// Running size of everything decoded so far.
    weight: u64,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            end: buf.len(),
            version: 0,
            shared: Vec::new(),
            mapping: Vec::new(),
            shared_seen: 0,
            depth: 0,
            checksum: None,
            string_table_uses: 0,
            weight: 0,
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], MarshalError> {
        let stop = self
            .pos
            .checked_add(n)
            .filter(|&stop| stop <= self.end)
            .ok_or(MarshalError::UnexpectedEnd)?;
        let bytes = &self.buf[self.pos..stop];
        self.pos = stop;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], MarshalError> {
        let mut out = [0_u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, MarshalError> {
        Ok(self.array::<1>()?[0])
    }

    fn i32(&mut self) -> Result<i32, MarshalError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn peek(&self) -> Result<u8, MarshalError> {
        if self.pos >= self.end {
            return Err(MarshalError::UnexpectedEnd);
        }
        Ok(self.buf[self.pos])
    }

    /// The format's variable-size integer: one byte, or `0xFF` + int32.
    fn integer(&mut self) -> Result<i64, MarshalError> {
        match self.u8()? {
            0xFF => Ok(i64::from(self.i32()?)),
            byte => Ok(i64::from(byte)),
        }
    }

    fn count(&mut self) -> Result<usize, MarshalError> {
        usize::try_from(self.integer()?).map_err(|_| MarshalError::NegativeLength)
    }

    /// A length-prefixed run of bytes.
    fn buffer(&mut self) -> Result<&'a [u8], MarshalError> {
        let n = self.count()?;
        self.take(n)
    }

    fn header(&mut self) -> Result<(), MarshalError> {
        self.version = match self.u8()? {
            tag::SIGNATURE_V0 => 0,
            tag::SIGNATURE_VERSIONED => self.u8()?,
            _ => return Err(MarshalError::BadSignature),
        };
        if self.version != 0 {
            return Ok(());
        }
        let count = usize::try_from(self.i32()?).map_err(|_| MarshalError::NegativeMapCount)?;
        if count == 0 {
            return Ok(());
        }
        let start = count
            .checked_mul(4)
            .and_then(|bytes| self.buf.len().checked_sub(bytes))
            .filter(|&start| start >= self.pos)
            .ok_or(MarshalError::TruncatedMap)?;
        let mut mapping = Vec::with_capacity(count);
        for entry in self.buf[start..].as_chunks::<4>().0 {
            let raw = i32::from_le_bytes(*entry);
            let slot = usize::try_from(raw)
                .ok()
                .filter(|slot| (1..=count).contains(slot))
                .ok_or(MarshalError::BadMapEntry)?;
            mapping.push(slot - 1);
        }
        self.mapping = mapping;
        self.end = start;
        self.shared.resize_with(count, || None);
        Ok(())
    }

    fn bump(&mut self, amount: u64) -> Result<(), MarshalError> {
        self.weight = self.weight.saturating_add(amount);
        if self.weight > MAX_DECODED_WEIGHT {
            return Err(MarshalError::TooLarge);
        }
        Ok(())
    }

    /// Claims the next shared slot.
    fn alloc(&mut self) -> Result<usize, MarshalError> {
        if self.version == 0 {
            let slot = *self
                .mapping
                .get(self.shared_seen)
                .ok_or(MarshalError::SharedOverflow)?;
            self.shared_seen += 1;
            Ok(slot)
        } else {
            self.shared.push(None);
            Ok(self.shared.len() - 1)
        }
    }

    fn fill(&mut self, slot: usize, value: &Value, started_at: u64) {
        self.shared[slot] = Some(Slot {
            value: value.clone(),
            weight: self.weight - started_at,
        });
    }

    /// Finishes a value that has no children, sharing it if flagged.
    fn leaf(&mut self, shared: bool, started_at: u64, value: Value) -> Result<Value, MarshalError> {
        if shared {
            let slot = self.alloc()?;
            self.fill(slot, &value, started_at);
        }
        Ok(value)
    }

    /// Reads a value that has children. Its slot is claimed before the
    /// children are read and filled afterwards.
    fn container(
        &mut self,
        shared: bool,
        started_at: u64,
        build: impl FnOnce(&mut Self) -> Result<Value, MarshalError>,
    ) -> Result<Value, MarshalError> {
        let slot = if shared { Some(self.alloc()?) } else { None };
        let value = build(self)?;
        if let Some(slot) = slot {
            self.fill(slot, &value, started_at);
        }
        Ok(value)
    }

    fn reference(&mut self) -> Result<Value, MarshalError> {
        let id = self.integer()?;
        let index = if self.version == 0 { id - 1 } else { id };
        let index = usize::try_from(index)
            .ok()
            .filter(|&index| index < self.shared.len())
            .ok_or(MarshalError::InvalidReference(id))?;
        let weight = match &self.shared[index] {
            Some(slot) => slot.weight,
            None => return Err(MarshalError::IncompleteReference),
        };
        // Check the size before copying, so a hostile file can't make us
        // allocate first and refuse afterwards.
        self.bump(weight)?;
        match &self.shared[index] {
            Some(slot) => Ok(slot.value.clone()),
            None => Err(MarshalError::IncompleteReference),
        }
    }

    fn verify_checksum(&self, checksum: Checksum, end: usize) -> Result<(), MarshalError> {
        let end = end.min(self.buf.len());
        if adler32(&self.buf[checksum.start..end]) != checksum.expected {
            return Err(MarshalError::ChecksumMismatch);
        }
        Ok(())
    }

    fn items(&mut self, n: usize) -> Result<Vec<Value>, MarshalError> {
        // Every item needs at least one byte, so a huge count can't be honest.
        let mut items = Vec::with_capacity(n.min(self.end - self.pos));
        for _ in 0..n {
            items.push(self.obj()?);
        }
        Ok(items)
    }

    fn utf16(&mut self, units: usize) -> Result<Value, MarshalError> {
        let bytes = self.take(units.checked_mul(2).ok_or(MarshalError::UnexpectedEnd)?)?;
        self.bump(bytes.len() as u64)?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16(&units)
            .map(Value::Str)
            .map_err(|_| MarshalError::InvalidText)
    }

    fn utf8(&mut self) -> Result<String, MarshalError> {
        let bytes = self.buffer()?;
        self.bump(bytes.len() as u64)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| MarshalError::InvalidText)
    }

    fn bytes(&mut self, n: usize) -> Result<Value, MarshalError> {
        let bytes = self.take(n)?;
        self.bump(bytes.len() as u64)?;
        Ok(Value::Bytes(bytes.to_vec()))
    }

    fn obj(&mut self) -> Result<Value, MarshalError> {
        if self.depth >= MAX_DEPTH {
            return Err(MarshalError::TooDeep);
        }
        self.depth += 1;
        let result = self.obj_inner();
        self.depth -= 1;
        result
    }

    fn obj_inner(&mut self) -> Result<Value, MarshalError> {
        let started_at = self.weight;
        self.bump(1)?;
        let raw = self.u8()?;
        let shared = raw & tag::SHARED_FLAG != 0;
        match raw & tag::TYPE_MASK {
            tag::NONE => Ok(Value::None),
            tag::TRUE => Ok(Value::Bool(true)),
            tag::FALSE => Ok(Value::Bool(false)),
            tag::INT_MINUS_ONE => Ok(Value::Int(-1)),
            tag::INT_ZERO => Ok(Value::Int(0)),
            tag::INT_ONE => Ok(Value::Int(1)),
            tag::INT8 => Ok(Value::Int(i64::from(i8::from_le_bytes(self.array()?)))),
            tag::INT16 => Ok(Value::Int(i64::from(i16::from_le_bytes(self.array()?)))),
            tag::INT32 => Ok(Value::Int(i64::from(self.i32()?))),
            tag::INT64 => Ok(Value::Int(i64::from_le_bytes(self.array()?))),
            tag::FLOAT_ZERO => Ok(Value::Float(0.0)),
            tag::FLOAT => Ok(Value::Float(f64::from_le_bytes(self.array()?))),
            tag::LONG => {
                let bytes = self.buffer()?;
                self.bump(bytes.len() as u64)?;
                self.leaf(shared, started_at, Value::Long(Long::from_le_bytes(bytes)))
            }
            tag::STR_EMPTY => Ok(Value::Bytes(Vec::new())),
            tag::STR_CHAR => self.bytes(1),
            tag::STR_SHORT => {
                let n = usize::from(self.u8()?);
                self.bytes(n)
            }
            tag::STR_TABLE => {
                let index = self.u8()?;
                let entry = usize::from(index)
                    .checked_sub(1)
                    .and_then(|i| STRING_TABLE.get(i))
                    .ok_or(MarshalError::BadStringTableIndex(index))?;
                self.string_table_uses += 1;
                self.bump(entry.len() as u64)?;
                Ok(Value::Bytes(entry.as_bytes().to_vec()))
            }
            tag::STR | tag::BUFFER => {
                let n = self.count()?;
                let value = self.bytes(n)?;
                self.leaf(shared, started_at, value)
            }
            tag::UNICODE_EMPTY => Ok(Value::Str(String::new())),
            tag::UNICODE_CHAR => self.utf16(1),
            tag::UNICODE_UTF16 => {
                let units = self.count()?;
                self.utf16(units)
            }
            tag::UNICODE_UTF8 => Ok(Value::Str(self.utf8()?)),
            tag::GLOBAL => {
                let name = self.utf8()?;
                self.leaf(shared, started_at, Value::Global(name))
            }
            tag::TUPLE_EMPTY => Ok(Value::Tuple(Vec::new())),
            tag::TUPLE_ONE => self.container(shared, started_at, |r| Ok(Value::Tuple(r.items(1)?))),
            tag::TUPLE_TWO => self.container(shared, started_at, |r| Ok(Value::Tuple(r.items(2)?))),
            tag::TUPLE => {
                let n = self.count()?;
                self.container(shared, started_at, |r| Ok(Value::Tuple(r.items(n)?)))
            }
            tag::LIST_EMPTY => Ok(Value::List(Vec::new())),
            tag::LIST_ONE => self.container(shared, started_at, |r| Ok(Value::List(r.items(1)?))),
            tag::LIST => {
                let n = self.count()?;
                self.container(shared, started_at, |r| Ok(Value::List(r.items(n)?)))
            }
            tag::DICT => {
                let n = self.count()?;
                self.container(shared, started_at, |r| r.dict(n))
            }
            tag::INSTANCE => self.container(shared, started_at, Self::instance),
            tag::REDUCE => self.container(shared, started_at, |r| r.reduce(false)),
            tag::NEWOBJ => self.container(shared, started_at, |r| r.reduce(true)),
            tag::CALLBACK => Ok(Value::Callback(Box::new(self.obj()?))),
            tag::REFERENCE => self.reference(),
            tag::CHECKSUM => {
                let expected = u32::from_le_bytes(self.array()?);
                let checksum = Checksum {
                    start: self.pos,
                    expected,
                };
                self.checksum = Some(checksum);
                if self.version == 0 {
                    // Version 0 checksums cover the rest of the file,
                    // shared map included.
                    self.verify_checksum(checksum, self.buf.len())?;
                }
                self.obj()
            }
            other => Err(MarshalError::UnsupportedTag(other)),
        }
    }

    fn dict(&mut self, n: usize) -> Result<Value, MarshalError> {
        let mut pairs = Vec::with_capacity(n.min((self.end - self.pos) / 2));
        for _ in 0..n {
            // The value comes before the key on the wire.
            let value = self.obj()?;
            let key = self.obj()?;
            pairs.push((key, value));
        }
        Ok(Value::Dict(pairs))
    }

    fn instance(&mut self) -> Result<Value, MarshalError> {
        let class = match self.obj()? {
            // Strict: a class name is never altered to make it decode.
            Value::Bytes(bytes) => {
                String::from_utf8(bytes).map_err(|_| MarshalError::InvalidText)?
            }
            Value::Str(name) => name,
            // A global is refused even though it is spelled like a name:
            // writing it back as a string would change its type.
            _ => return Err(MarshalError::InstanceClassNotString),
        };
        let state = Box::new(self.obj()?);
        Ok(Value::Instance { class, state })
    }

    fn reduce(&mut self, newobj: bool) -> Result<Value, MarshalError> {
        let Value::Tuple(head) = self.obj()? else {
            return Err(MarshalError::ReduceNotTuple);
        };
        let mut head = head.into_iter();
        let callable = if newobj {
            None
        } else {
            Some(head.next().ok_or(MarshalError::ReduceTooShort)?)
        };
        let args = head.next().ok_or(if newobj {
            MarshalError::ReduceEmptyNewobj
        } else {
            MarshalError::ReduceTooShort
        })?;
        let state = head.next();
        if head.next().is_some() {
            // The client ignores extra items; re-encoding would drop them.
            return Err(MarshalError::ReduceTooLong);
        }

        let mut list_items = Vec::new();
        // An exact byte comparison, as Marshal.cpp does it: a MARK with the
        // shared flag set is not a terminator.
        while self.peek()? != tag::MARK {
            list_items.push(self.obj()?);
        }
        self.pos += 1;
        let mut dict_items = Vec::new();
        while self.peek()? != tag::MARK {
            let key = self.obj()?;
            let value = self.obj()?;
            dict_items.push((key, value));
        }
        self.pos += 1;

        Ok(Value::Reduce(Box::new(Reduce {
            callable,
            args,
            state,
            list_items,
            dict_items,
        })))
    }
}
