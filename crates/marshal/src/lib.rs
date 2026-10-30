//! Strict reader and writer for CCP's `blue.marshal` format, the binary
//! serialisation EVE Online uses for its settings files.
//!
//! This is a port of the codec in `reference/eanm_sync.py`, which is itself a
//! port of `Marshal.cpp` from CCP's carbonengine/blue. The decoder is
//! deliberately stricter than the client's: anything unusual is an error, so
//! a caller can leave the file alone instead of guessing.
//!
//! The encoder writes format version 1 with no shared references, which the
//! client reads natively.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod adler;
mod decode;
mod encode;
mod error;
mod string_table;
mod tag;
mod value;

pub use decode::{decode, Decoded, MAX_DECODED_WEIGHT};
pub use encode::encode;
pub use error::MarshalError;
pub use string_table::STRING_TABLE;
pub use value::{Long, Reduce, Value};

/// Deepest nesting the decoder and encoder accept.
///
/// The client allows 1000 levels; settings files use about a dozen. Kept low
/// so the native stack is never at risk.
pub const MAX_DEPTH: usize = 100;
