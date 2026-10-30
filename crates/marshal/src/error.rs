//! Why a stream was refused.

use std::fmt;

/// A reason the decoder or encoder refused to continue.
///
/// Every variant means "leave this file alone"; none of them is recoverable
/// by guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarshalError {
    /// The stream ended in the middle of a value.
    UnexpectedEnd,
    /// The first byte isn't a marshal signature.
    BadSignature,
    /// A version 0 header declared a negative shared-object count.
    NegativeMapCount,
    /// The shared-object map doesn't fit in the file.
    TruncatedMap,
    /// A shared-object map entry is outside `1..=count`.
    BadMapEntry,
    /// More shared objects appeared than the header declared.
    SharedOverflow,
    /// A reference points outside the shared-object table.
    InvalidReference(i64),
    /// A reference points at an object that is still being read. The client
    /// either rejects this or builds a self-referencing structure, which
    /// can't be represented faithfully.
    IncompleteReference,
    /// The stream carries a checksum that doesn't match its contents.
    ChecksumMismatch,
    /// Values are nested deeper than [`crate::MAX_DEPTH`].
    TooDeep,
    /// A length or count is negative.
    NegativeLength,
    /// A string-table index is outside the table.
    BadStringTableIndex(u8),
    /// Text isn't valid UTF-8 or UTF-16.
    InvalidText,
    /// An instance's class name isn't a string.
    InstanceClassNotString,
    /// A reduce or newobj payload isn't a tuple.
    ReduceNotTuple,
    /// A newobj payload tuple is empty.
    ReduceEmptyNewobj,
    /// A reduce payload tuple has fewer than two items.
    ReduceTooShort,
    /// A reduce or newobj payload tuple has more items than the format
    /// defines. The client ignores the extras, so they would be lost.
    ReduceTooLong,
    /// A type tag this codec doesn't handle (DBRow, streams, pickles, or an
    /// unknown tag).
    UnsupportedTag(u8),
    /// Shared references expand the data beyond
    /// [`crate::MAX_DECODED_WEIGHT`].
    TooLarge,
    /// A value is too long to be written with a 32-bit length.
    LengthOverflow,
}

impl fmt::Display for MarshalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => f.write_str("unexpected end of data"),
            Self::BadSignature => f.write_str("not a marshal stream (bad signature)"),
            Self::NegativeMapCount => f.write_str("negative shared map count"),
            Self::TruncatedMap => f.write_str("truncated shared-object map"),
            Self::BadMapEntry => f.write_str("bogus shared-object map entry"),
            Self::SharedOverflow => f.write_str("shared object table overflow"),
            Self::InvalidReference(id) => write!(f, "invalid shared reference {id}"),
            Self::IncompleteReference => {
                f.write_str("reference to an object that isn't complete yet")
            }
            Self::ChecksumMismatch => f.write_str("checksum mismatch (file damaged?)"),
            Self::TooDeep => f.write_str("data nested too deeply"),
            Self::NegativeLength => f.write_str("negative length in stream"),
            Self::BadStringTableIndex(i) => write!(f, "bad string table index {i}"),
            Self::InvalidText => f.write_str("invalid text in stream"),
            Self::InstanceClassNotString => f.write_str("instance class name isn't a string"),
            Self::ReduceNotTuple => f.write_str("reduce payload isn't a tuple"),
            Self::ReduceEmptyNewobj => f.write_str("empty newobj tuple"),
            Self::ReduceTooShort => f.write_str("reduce tuple too short"),
            Self::ReduceTooLong => f.write_str("reduce tuple has unexpected extra items"),
            Self::UnsupportedTag(t) => write!(f, "unsupported marshal type tag {t}"),
            Self::TooLarge => f.write_str("data is too large once shared references are expanded"),
            Self::LengthOverflow => f.write_str("value too long to encode"),
        }
    }
}

impl std::error::Error for MarshalError {}
