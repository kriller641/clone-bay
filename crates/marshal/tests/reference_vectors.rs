//! Checks this codec against the Python reference codec and the
//! `blue-marshal` crate, on streams the Python fuzz generator produced.
//!
//! The streams use every tag, alternative encodings, shared flags,
//! back-references, version 0 maps in shuffled order, checksums, and
//! corrupted variants. Regenerate the data with
//! `reference/dev_tools/make_vectors.py`.
//!
//! Set `MARSHAL_VECTORS` to a file path to run a different (larger) set.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use clone_bay_marshal::{decode, encode, MarshalError};
use common::{crate_reencode, from_hex, without_negative_zero};

const COMMITTED: &str = include_str!("data/reference_vectors.txt");

/// The ways this codec is deliberately stricter than the Python reference.
fn is_extra_strictness(error: &MarshalError) -> bool {
    matches!(
        error,
        MarshalError::ReduceTooLong
            | MarshalError::TooLarge
            // The reference accepts a global as an instance's class name.
            | MarshalError::InstanceClassNotString
    )
}

#[test]
fn agrees_with_the_python_reference_and_the_crate() {
    let from_env = std::env::var("MARSHAL_VECTORS")
        .ok()
        .map(|path| std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")));
    let vectors = from_env.as_deref().unwrap_or(COMMITTED);

    let (mut valid, mut corrupt, mut both_accept, mut stricter) = (0, 0, 0, 0);
    for (number, line) in vectors.lines().enumerate() {
        let case = number + 1;
        let mut fields = line.splitn(3, ' ');
        let kind = fields.next().unwrap();
        let stream = from_hex(fields.next().unwrap());
        let python = fields.next().unwrap();
        let python_output = (!python.starts_with('!')).then(|| from_hex(python));

        // What we write for the stream, with the one known difference
        // (the sign of zero) taken out.
        let decoded = decode(&stream);
        let ours = decoded.as_ref().map_err(Clone::clone).map(|decoded| {
            encode(&without_negative_zero(&decoded.value), decoded.had_checksum).unwrap()
        });

        match kind {
            "V" => {
                valid += 1;
                let leftover = decoded.as_ref().map(|decoded| decoded.trailing_bytes);
                assert_eq!(leftover, Ok(0), "case {case}: valid stream");
                assert_eq!(
                    ours.as_ref().ok(),
                    python_output.as_ref(),
                    "case {case}: valid stream"
                );
            }
            "C" => {
                corrupt += 1;
                match (&ours, &python_output) {
                    (Ok(ours), Some(python)) => {
                        both_accept += 1;
                        assert_eq!(ours, python, "case {case}: both accept, outputs differ");
                    }
                    (Err(_), None) => {}
                    (Err(error), Some(_)) => {
                        stricter += 1;
                        assert!(
                            is_extra_strictness(error),
                            "case {case}: refused a stream the reference accepts ({error})"
                        );
                    }
                    (Ok(_), None) => {
                        panic!("case {case}: accepted a stream the reference refuses ({python})")
                    }
                }
            }
            other => panic!("case {case}: unknown kind {other:?}"),
        }

        // Whenever we accept a stream, the crate must accept it too and
        // write the same bytes.
        if let Ok(ours) = &ours {
            assert_eq!(
                crate_reencode(&stream).as_ref(),
                Ok(ours),
                "case {case}: crate differs"
            );
        }
    }

    println!(
        "{valid} valid, {corrupt} corrupted ({both_accept} still accepted by both, \
         {stricter} refused only by us)"
    );
    // Guard against an empty or truncated data file passing silently.
    assert!(
        valid >= 100 && corrupt >= 200,
        "too few cases: {valid} valid, {corrupt} corrupted"
    );
    assert!(
        both_accept > 0,
        "no corrupted stream was accepted; the comparison is untested"
    );
}
