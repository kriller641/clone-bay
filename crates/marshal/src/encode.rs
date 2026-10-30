//! The encoder.
//!
//! Writes format version 1 with no shared references: every value is written
//! out in full. This is the same output EVE Wrench produces, and a form the
//! client reads natively.

use crate::adler::adler32;
use crate::tag;
use crate::value::{Reduce, Value};
use crate::{MarshalError, MAX_DEPTH};

/// Encodes `value` as a complete marshal stream.
///
/// With `checksum` set, the stream carries an Adler-32 of everything after
/// the checksum field.
pub fn encode(value: &Value, checksum: bool) -> Result<Vec<u8>, MarshalError> {
    let mut writer = Writer {
        out: vec![tag::SIGNATURE_VERSIONED, 1],
        depth: 0,
    };
    let checksum_at = checksum.then(|| {
        writer.out.push(tag::CHECKSUM);
        let at = writer.out.len();
        writer.out.extend_from_slice(&[0; 4]);
        at
    });
    writer.value(value)?;
    if let Some(at) = checksum_at {
        let sum = adler32(&writer.out[at + 4..]);
        writer.out[at..at + 4].copy_from_slice(&sum.to_le_bytes());
    }
    Ok(writer.out)
}

struct Writer {
    out: Vec<u8>,
    depth: usize,
}

impl Writer {
    /// The format's variable-size integer: one byte, or `0xFF` + int32.
    fn integer(&mut self, n: usize) -> Result<(), MarshalError> {
        if let Some(byte) = u8::try_from(n).ok().filter(|&byte| byte != 0xFF) {
            self.out.push(byte);
        } else {
            let wide = i32::try_from(n).map_err(|_| MarshalError::LengthOverflow)?;
            self.out.push(0xFF);
            self.out.extend_from_slice(&wide.to_le_bytes());
        }
        Ok(())
    }

    /// A length-prefixed run of bytes.
    fn buffer(&mut self, bytes: &[u8]) -> Result<(), MarshalError> {
        self.integer(bytes.len())?;
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    fn byte_string(&mut self, bytes: &[u8]) -> Result<(), MarshalError> {
        match bytes {
            [] => self.out.push(tag::STR_EMPTY),
            [only] => self.out.extend_from_slice(&[tag::STR_CHAR, *only]),
            _ => {
                // The client's own writer uses the buffer tag for strings.
                self.out.push(tag::BUFFER);
                self.buffer(bytes)?;
            }
        }
        Ok(())
    }

    fn int(&mut self, n: i64) {
        match n {
            -1 => self.out.push(tag::INT_MINUS_ONE),
            0 => self.out.push(tag::INT_ZERO),
            1 => self.out.push(tag::INT_ONE),
            _ => {
                if let Ok(small) = i8::try_from(n) {
                    self.out.push(tag::INT8);
                    self.out.extend_from_slice(&small.to_le_bytes());
                } else if let Ok(short) = i16::try_from(n) {
                    self.out.push(tag::INT16);
                    self.out.extend_from_slice(&short.to_le_bytes());
                } else if let Ok(medium) = i32::try_from(n) {
                    self.out.push(tag::INT32);
                    self.out.extend_from_slice(&medium.to_le_bytes());
                } else {
                    self.out.push(tag::INT64);
                    self.out.extend_from_slice(&n.to_le_bytes());
                }
            }
        }
    }

    /// A tuple or list. `pair` is the dedicated two-item tag, which only
    /// tuples have.
    fn sequence(
        &mut self,
        items: &[Value],
        empty: u8,
        single: u8,
        pair: Option<u8>,
        many: u8,
    ) -> Result<(), MarshalError> {
        match (items.len(), pair) {
            (0, _) => {
                self.out.push(empty);
                return Ok(());
            }
            (1, _) => self.out.push(single),
            (2, Some(pair)) => self.out.push(pair),
            (n, _) => {
                self.out.push(many);
                self.integer(n)?;
            }
        }
        items.iter().try_for_each(|item| self.value(item))
    }

    fn reduce(&mut self, reduce: &Reduce) -> Result<(), MarshalError> {
        let mut head = Vec::with_capacity(3);
        match &reduce.callable {
            Some(callable) => {
                self.out.push(tag::REDUCE);
                head.push(callable.clone());
            }
            None => self.out.push(tag::NEWOBJ),
        }
        head.push(reduce.args.clone());
        head.extend(reduce.state.clone());
        self.value(&Value::Tuple(head))?;
        for item in &reduce.list_items {
            self.value(item)?;
        }
        self.out.push(tag::MARK);
        for (key, value) in &reduce.dict_items {
            self.value(key)?;
            self.value(value)?;
        }
        self.out.push(tag::MARK);
        Ok(())
    }

    fn value(&mut self, value: &Value) -> Result<(), MarshalError> {
        if self.depth >= MAX_DEPTH {
            return Err(MarshalError::TooDeep);
        }
        self.depth += 1;
        let result = self.value_inner(value);
        self.depth -= 1;
        result
    }

    fn value_inner(&mut self, value: &Value) -> Result<(), MarshalError> {
        match value {
            Value::None => self.out.push(tag::NONE),
            Value::Bool(true) => self.out.push(tag::TRUE),
            Value::Bool(false) => self.out.push(tag::FALSE),
            Value::Int(n) => self.int(*n),
            Value::Long(long) => {
                self.out.push(tag::LONG);
                self.buffer(long.as_le_bytes())?;
            }
            Value::Float(x) => {
                // Only positive zero gets the short form, so -0.0 keeps its sign.
                if x.to_bits() == 0 {
                    self.out.push(tag::FLOAT_ZERO);
                } else {
                    self.out.push(tag::FLOAT);
                    self.out.extend_from_slice(&x.to_le_bytes());
                }
            }
            Value::Bytes(bytes) => self.byte_string(bytes)?,
            Value::Str(text) => {
                if text.is_empty() {
                    self.out.push(tag::UNICODE_EMPTY);
                } else {
                    self.out.push(tag::UNICODE_UTF8);
                    self.buffer(text.as_bytes())?;
                }
            }
            Value::Global(name) => {
                self.out.push(tag::GLOBAL);
                self.buffer(name.as_bytes())?;
            }
            Value::Tuple(items) => self.sequence(
                items,
                tag::TUPLE_EMPTY,
                tag::TUPLE_ONE,
                Some(tag::TUPLE_TWO),
                tag::TUPLE,
            )?,
            Value::List(items) => {
                self.sequence(items, tag::LIST_EMPTY, tag::LIST_ONE, None, tag::LIST)?;
            }
            Value::Dict(pairs) => {
                self.out.push(tag::DICT);
                self.integer(pairs.len())?;
                for (key, value) in pairs {
                    // The value comes before the key on the wire.
                    self.value(value)?;
                    self.value(key)?;
                }
            }
            Value::Instance { class, state } => {
                self.out.push(tag::INSTANCE);
                self.byte_string(class.as_bytes())?;
                self.value(state)?;
            }
            Value::Reduce(reduce) => self.reduce(reduce)?,
            Value::Callback(inner) => {
                self.out.push(tag::CALLBACK);
                self.value(inner)?;
            }
        }
        Ok(())
    }
}
