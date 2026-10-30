//! Type tags. The low six bits are the type; `SHARED_FLAG` marks an object
//! that takes a slot in the shared-object table.

pub const SHARED_FLAG: u8 = 0x40;
pub const TYPE_MASK: u8 = 0x3F;

/// Stream signature for format version 0 (shared map at the end of the file).
pub const SIGNATURE_V0: u8 = 0x7E;
/// Stream signature followed by an explicit version byte.
pub const SIGNATURE_VERSIONED: u8 = 0x7D;

pub const NONE: u8 = 1;
pub const GLOBAL: u8 = 2;
pub const INT64: u8 = 3;
pub const INT32: u8 = 4;
pub const INT16: u8 = 5;
pub const INT8: u8 = 6;
pub const INT_MINUS_ONE: u8 = 7;
pub const INT_ZERO: u8 = 8;
pub const INT_ONE: u8 = 9;
pub const FLOAT: u8 = 10;
pub const FLOAT_ZERO: u8 = 11;
pub const STR: u8 = 13;
pub const STR_EMPTY: u8 = 14;
pub const STR_CHAR: u8 = 15;
pub const STR_SHORT: u8 = 16;
pub const STR_TABLE: u8 = 17;
pub const UNICODE_UTF16: u8 = 18;
pub const BUFFER: u8 = 19;
pub const TUPLE: u8 = 20;
pub const LIST: u8 = 21;
pub const DICT: u8 = 22;
pub const INSTANCE: u8 = 23;
pub const CALLBACK: u8 = 25;
pub const REFERENCE: u8 = 27;
pub const CHECKSUM: u8 = 28;
pub const TRUE: u8 = 31;
pub const FALSE: u8 = 32;
pub const REDUCE: u8 = 34;
pub const NEWOBJ: u8 = 35;
pub const TUPLE_EMPTY: u8 = 36;
pub const TUPLE_ONE: u8 = 37;
pub const LIST_EMPTY: u8 = 38;
pub const LIST_ONE: u8 = 39;
pub const UNICODE_EMPTY: u8 = 40;
pub const UNICODE_CHAR: u8 = 41;
pub const TUPLE_TWO: u8 = 44;
pub const MARK: u8 = 45;
pub const UNICODE_UTF8: u8 = 46;
pub const LONG: u8 = 47;
