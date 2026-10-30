//! Randomised tests: whatever is written can be read back unchanged, and
//! the `blue-marshal` crate agrees about the bytes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use clone_bay_marshal::{decode, encode};
use common::{any_value, crate_reencode, without_negative_zero};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    #[test]
    fn decode_returns_exactly_what_was_encoded(value in any_value(), checksum in any::<bool>()) {
        let stream = encode(&value, checksum).unwrap();
        let decoded = decode(&stream).unwrap();
        prop_assert_eq!(&decoded.value, &value);
        prop_assert_eq!(decoded.version, 1);
        prop_assert_eq!(decoded.had_checksum, checksum);
        prop_assert_eq!(decoded.trailing_bytes, 0);
        prop_assert_eq!(decoded.unused_shared_slots, 0);
        // Encoding is a pure function of the value.
        prop_assert_eq!(encode(&decoded.value, checksum).unwrap(), stream);
    }

    #[test]
    fn output_is_byte_identical_to_the_crate(value in any_value(), checksum in any::<bool>()) {
        let value = without_negative_zero(&value);
        let stream = encode(&value, checksum).unwrap();
        prop_assert_eq!(crate_reencode(&stream), Ok(stream));
    }

    #[test]
    fn damaged_streams_never_panic_and_never_decode_to_something_the_crate_disagrees_with(
        value in any_value(),
        checksum in any::<bool>(),
        position in any::<prop::sample::Index>(),
        replacement in any::<u8>(),
        cut in any::<bool>(),
    ) {
        let mut stream = encode(&value, checksum).unwrap();
        let at = position.index(stream.len());
        if cut {
            stream.truncate(at);
        } else {
            stream[at] = replacement;
        }
        if let Ok(decoded) = decode(&stream) {
            // This decoder is the stricter one: if it accepts, so must the crate.
            let ours = encode(&without_negative_zero(&decoded.value), decoded.had_checksum).unwrap();
            prop_assert_eq!(crate_reencode(&stream), Ok(ours));
        }
    }
}
