//! The decoded form of a marshal stream.

/// One marshalled value.
///
/// The variants keep every distinction the format makes that matters on
/// re-encoding: byte strings versus unicode, `int` versus Python 2 `long`,
/// tuples versus lists.
#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    /// A Python 2 `long`, kept distinct from `Int` so it is written back as one.
    Long(Long),
    Float(f64),
    /// A byte string (Python 2 `str`). Most settings keys are these.
    Bytes(Vec<u8>),
    /// Unicode text.
    Str(String),
    /// A class or function referenced by dotted name.
    Global(String),
    Tuple(Vec<Value>),
    List(Vec<Value>),
    /// A dict as `(key, value)` pairs in stream order.
    Dict(Vec<(Value, Value)>),
    Instance {
        class: String,
        state: Box<Value>,
    },
    Reduce(Box<Reduce>),
    Callback(Box<Value>),
}

/// Floats compare by bit pattern, so `-0.0` differs from `0.0` and a NaN
/// equals itself. That is what "the file holds the same data" means.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Long(a), Self::Long(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Str(a), Self::Str(b)) | (Self::Global(a), Self::Global(b)) => a == b,
            (Self::Tuple(a), Self::Tuple(b)) | (Self::List(a), Self::List(b)) => a == b,
            (Self::Dict(a), Self::Dict(b)) => a == b,
            (Self::Instance { class: a, state: x }, Self::Instance { class: b, state: y }) => {
                a == b && x == y
            }
            (Self::Reduce(a), Self::Reduce(b)) => a == b,
            (Self::Callback(a), Self::Callback(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

/// A pickled object rebuilt by calling something (`TY_REDUCE` / `TY_NEWOBJ`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reduce {
    /// The callable for a reduce. `None` means this is a newobj, which has
    /// no callable.
    pub callable: Option<Value>,
    pub args: Value,
    /// `None` means the stream carried no state at all, which is different
    /// from a state of [`Value::None`].
    pub state: Option<Value>,
    pub list_items: Vec<Value>,
    /// `(key, value)` pairs.
    pub dict_items: Vec<(Value, Value)>,
}

/// An arbitrary-size signed integer (Python 2 `long`).
///
/// Stored as minimal little-endian two's complement bytes, which is exactly
/// what the encoder writes. Zero is the empty byte string. Two `Long`s are
/// equal when they are the same number, however many padding bytes the
/// stream used.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Long(Vec<u8>);

impl Long {
    /// From little-endian two's complement bytes of any length.
    pub fn from_le_bytes(bytes: &[u8]) -> Self {
        let mut len = bytes.len();
        // Drop sign-extension bytes that add nothing.
        while len > 1 {
            let (top, next) = (bytes[len - 1], bytes[len - 2]);
            let redundant = (top == 0x00 && next < 0x80) || (top == 0xFF && next >= 0x80);
            if !redundant {
                break;
            }
            len -= 1;
        }
        if len == 1 && bytes[0] == 0 {
            len = 0;
        }
        Self(bytes[..len].to_vec())
    }

    pub fn from_i64(n: i64) -> Self {
        Self::from_le_bytes(&n.to_le_bytes())
    }

    /// The minimal little-endian two's complement bytes; empty for zero.
    pub fn as_le_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The number as an `i64`, or `None` if it doesn't fit.
    pub fn to_i64(&self) -> Option<i64> {
        if self.0.len() > 8 {
            return None;
        }
        let negative = self.0.last().is_some_and(|&b| b >= 0x80);
        let mut buf = [if negative { 0xFF } else { 0x00 }; 8];
        buf[..self.0.len()].copy_from_slice(&self.0);
        Some(i64::from_le_bytes(buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_zero_is_empty_whatever_the_padding() {
        assert_eq!(Long::from_le_bytes(&[]).as_le_bytes(), b"");
        assert_eq!(Long::from_le_bytes(&[0]).as_le_bytes(), b"");
        assert_eq!(Long::from_le_bytes(&[0, 0, 0]).as_le_bytes(), b"");
        assert_eq!(Long::from_i64(0).as_le_bytes(), b"");
    }

    #[test]
    fn long_strips_only_redundant_sign_bytes() {
        assert_eq!(Long::from_le_bytes(&[0x7F, 0x00]).as_le_bytes(), &[0x7F]);
        assert_eq!(
            Long::from_le_bytes(&[0x80, 0x00]).as_le_bytes(),
            &[0x80, 0x00]
        );
        assert_eq!(Long::from_le_bytes(&[0x80, 0xFF]).as_le_bytes(), &[0x80]);
        assert_eq!(
            Long::from_le_bytes(&[0x7F, 0xFF]).as_le_bytes(),
            &[0x7F, 0xFF]
        );
        assert_eq!(
            Long::from_le_bytes(&[0xFF, 0xFF, 0xFF]).as_le_bytes(),
            &[0xFF]
        );
    }

    #[test]
    fn long_i64_round_trips_at_the_edges() {
        for n in [0, 1, -1, 127, 128, -128, -129, 255, 256, i64::MAX, i64::MIN] {
            assert_eq!(Long::from_i64(n).to_i64(), Some(n), "{n}");
        }
        assert_eq!(Long::from_i64(128).as_le_bytes(), &[0x80, 0x00]);
        assert_eq!(Long::from_i64(-129).as_le_bytes(), &[0x7F, 0xFF]);
    }

    #[test]
    fn long_wider_than_i64_does_not_convert() {
        let two_pow_63 = Long::from_le_bytes(&[0, 0, 0, 0, 0, 0, 0, 0x80, 0]);
        assert_eq!(two_pow_63.as_le_bytes().len(), 9);
        assert_eq!(two_pow_63.to_i64(), None);
    }

    #[test]
    fn floats_compare_by_bits() {
        assert_ne!(Value::Float(0.0), Value::Float(-0.0));
        assert_eq!(Value::Float(f64::NAN), Value::Float(f64::NAN));
        assert_eq!(Value::Float(1.5), Value::Float(1.5));
    }

    #[test]
    fn instances_are_equal_only_when_class_and_state_both_match() {
        let instance = |class: &str, state: i64| Value::Instance {
            class: class.into(),
            state: Box::new(Value::Int(state)),
        };
        assert_eq!(instance("a", 1), instance("a", 1));
        assert_ne!(instance("a", 1), instance("b", 1));
        assert_ne!(instance("a", 1), instance("a", 2));
        assert_ne!(instance("a", 1), instance("b", 2));
    }

    #[test]
    fn different_kinds_are_never_equal() {
        assert_ne!(Value::Int(1), Value::Long(Long::from_i64(1)));
        assert_ne!(Value::Int(1), Value::Bool(true));
        assert_ne!(Value::Bytes(b"a".to_vec()), Value::Str("a".into()));
        assert_ne!(Value::Str("a".into()), Value::Global("a".into()));
        assert_ne!(Value::Tuple(vec![]), Value::List(vec![]));
    }
}
