//! Reading a settings file and summarising what is in it, without changing
//! anything.

use std::fs;
use std::io::Read;
use std::path::Path;

use clone_bay_marshal::{decode, Decoded, MarshalError, Value};
use thiserror::Error;

/// Largest file that will be read. Real settings files are around 1 MB.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Why a file is being left alone.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InspectError {
    #[error("can't read it ({0})")]
    Read(String),
    #[error("it is too large to be a settings file")]
    TooLarge,
    #[error("can't decode it ({0})")]
    Decode(#[from] MarshalError),
    #[error("unexpected structure (top level isn't a dict)")]
    NotADict,
    #[error("unexpected extra data in the file")]
    ExtraData,
}

/// What a settings file contains, at a glance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSummary {
    /// Marshal format version. The client writes 0; this app writes 1.
    pub format_version: u8,
    pub had_checksum: bool,
    /// Top-level sections in file order.
    pub sections: Vec<SectionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSummary {
    pub name: String,
    /// How many entries the section holds, when it is a collection.
    pub entries: Option<usize>,
}

/// Reads a file, refusing anything larger than [`MAX_FILE_BYTES`].
pub fn read_settings_bytes(path: &Path) -> Result<Vec<u8>, InspectError> {
    read_at_most(path, MAX_FILE_BYTES)
}

fn read_at_most(path: &Path, limit: u64) -> Result<Vec<u8>, InspectError> {
    let read_error = |error: std::io::Error| InspectError::Read(error.to_string());
    let file = fs::File::open(path).map_err(read_error)?;
    let mut bytes = Vec::new();
    // Read one byte past the limit so a file that is too large is detected
    // even if it grew after its size was first seen.
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() as u64 > limit {
        return Err(InspectError::TooLarge);
    }
    Ok(bytes)
}

/// Reads and summarises a settings file.
pub fn inspect_file(path: &Path) -> Result<FileSummary, InspectError> {
    summarize(&read_settings_bytes(path)?)
}

/// Decodes the bytes of a settings file.
///
/// Refuses anything that isn't a clean settings file: the top level must be
/// a dict, with no bytes left over and no unused shared slots.
pub fn decode_settings(bytes: &[u8]) -> Result<Decoded, InspectError> {
    let decoded = decode(bytes)?;
    if !matches!(decoded.value, Value::Dict(_)) {
        return Err(InspectError::NotADict);
    }
    if decoded.trailing_bytes != 0 || decoded.unused_shared_slots != 0 {
        return Err(InspectError::ExtraData);
    }
    Ok(decoded)
}

/// Summarises the bytes of a settings file. Refuses whatever
/// [`decode_settings`] refuses.
pub fn summarize(bytes: &[u8]) -> Result<FileSummary, InspectError> {
    let decoded = decode_settings(bytes)?;
    let Value::Dict(pairs) = &decoded.value else {
        return Err(InspectError::NotADict);
    };
    let sections = pairs
        .iter()
        .map(|(key, value)| SectionSummary {
            name: key_label(key),
            entries: entry_count(value),
        })
        .collect();
    Ok(FileSummary {
        format_version: decoded.version,
        had_checksum: decoded.had_checksum,
        sections,
    })
}

/// A readable name for a dict key. Most keys are byte strings; a few are
/// numbers, tuples or `None`.
pub fn key_label(key: &Value) -> String {
    match key {
        Value::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Value::Str(text) => text.clone(),
        Value::Int(n) => n.to_string(),
        Value::Long(long) => match long.to_i64() {
            Some(n) => n.to_string(),
            None => "(large number)".to_owned(),
        },
        Value::None => "(none)".to_owned(),
        _ => "(non-text key)".to_owned(),
    }
}

/// The number of entries in a section, when it is a collection.
///
/// A section is either a collection itself, or a single setting stored as
/// `(timestamp, value)` whose value may be a collection.
fn entry_count(value: &Value) -> Option<usize> {
    match value {
        Value::Dict(pairs) => Some(pairs.len()),
        Value::List(items) => Some(items.len()),
        Value::Tuple(items) => match items.as_slice() {
            [Value::Long(_), Value::Dict(pairs)] => Some(pairs.len()),
            [Value::Long(_), Value::List(items)] => Some(items.len()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use clone_bay_marshal::{encode, Long};

    use super::*;

    fn bytes(text: &str) -> Value {
        Value::Bytes(text.as_bytes().to_vec())
    }

    fn timestamped(value: Value) -> Value {
        Value::Tuple(vec![
            Value::Long(Long::from_i64(134_360_688_266_310_454)),
            value,
        ])
    }

    fn sample_root() -> Value {
        Value::Dict(vec![
            (
                bytes("ui"),
                Value::Dict(vec![
                    (bytes("a"), timestamped(Value::Int(1))),
                    (bytes("b"), timestamped(Value::Int(2))),
                ]),
            ),
            (
                bytes("openWindows"),
                timestamped(Value::List(vec![
                    Value::Int(1),
                    Value::Int(2),
                    Value::Int(3),
                ])),
            ),
            (
                bytes("byId"),
                timestamped(Value::Dict(vec![(Value::Int(5), Value::None)])),
            ),
            (bytes("enableWindowBlur"), timestamped(Value::Bool(true))),
            (bytes("plain"), Value::List(vec![Value::None])),
        ])
    }

    #[test]
    fn summarises_sections_in_file_order_with_entry_counts() {
        let data = encode(&sample_root(), false).unwrap();
        let summary = summarize(&data).unwrap();
        assert_eq!(summary.format_version, 1);
        assert!(!summary.had_checksum);
        let got: Vec<(&str, Option<usize>)> = summary
            .sections
            .iter()
            .map(|s| (s.name.as_str(), s.entries))
            .collect();
        assert_eq!(
            got,
            vec![
                ("ui", Some(2)),
                ("openWindows", Some(3)),
                ("byId", Some(1)),
                ("enableWindowBlur", None),
                ("plain", Some(1)),
            ]
        );
    }

    #[test]
    fn reports_a_checksum() {
        let data = encode(&sample_root(), true).unwrap();
        assert!(summarize(&data).unwrap().had_checksum);
    }

    #[test]
    fn a_pair_without_a_timestamp_is_not_counted_as_a_collection() {
        let root = Value::Dict(vec![(
            bytes("pair"),
            Value::Tuple(vec![Value::Int(1), Value::List(vec![Value::None])]),
        )]);
        let summary = summarize(&encode(&root, false).unwrap()).unwrap();
        assert_eq!(summary.sections[0].entries, None);
    }

    #[test]
    fn refuses_a_top_level_that_is_not_a_dict() {
        let data = encode(&Value::List(vec![]), false).unwrap();
        assert_eq!(summarize(&data), Err(InspectError::NotADict));
    }

    #[test]
    fn refuses_trailing_bytes() {
        let mut data = encode(&sample_root(), false).unwrap();
        data.push(0x01);
        assert_eq!(summarize(&data), Err(InspectError::ExtraData));
    }

    #[test]
    fn refuses_unused_shared_slots() {
        // Version 0 header declaring one shared slot, an empty dict, then
        // the one-entry map. Nothing claims the slot.
        let data = [0x7E, 1, 0, 0, 0, 22, 0, 1, 0, 0, 0];
        assert_eq!(summarize(&data), Err(InspectError::ExtraData));
    }

    #[test]
    fn refuses_damaged_data_with_the_decoder_reason() {
        let data = encode(&sample_root(), false).unwrap();
        let cut = &data[..data.len() / 2];
        assert_eq!(
            summarize(cut),
            Err(InspectError::Decode(MarshalError::UnexpectedEnd))
        );
        assert_eq!(
            summarize(b"hello"),
            Err(InspectError::Decode(MarshalError::BadSignature))
        );
    }

    #[test]
    fn key_labels_cover_the_key_kinds_settings_use() {
        assert_eq!(key_label(&bytes("ui")), "ui");
        assert_eq!(key_label(&Value::Str("näme".into())), "näme");
        assert_eq!(key_label(&Value::Int(-4)), "-4");
        assert_eq!(
            key_label(&Value::Long(Long::from_i64(60_003_760))),
            "60003760"
        );
        assert_eq!(
            key_label(&Value::Long(Long::from_le_bytes(&[1; 12]))),
            "(large number)"
        );
        assert_eq!(key_label(&Value::None), "(none)");
        assert_eq!(key_label(&Value::Tuple(vec![])), "(non-text key)");
        // Invalid UTF-8 is shown, not refused: this is only a label.
        assert_eq!(key_label(&Value::Bytes(vec![b'a', 0xFF])), "a\u{FFFD}");
    }

    #[test]
    fn reads_a_file_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core_user_1.dat");
        let data = encode(&sample_root(), false).unwrap();
        fs::write(&path, &data).unwrap();
        assert_eq!(read_settings_bytes(&path).unwrap(), data);
        assert_eq!(inspect_file(&path).unwrap().sections.len(), 5);
    }

    #[test]
    fn a_file_over_the_size_limit_is_refused_and_one_at_the_limit_is_read() {
        assert_eq!(MAX_FILE_BYTES, 67_108_864);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core_user_1.dat");
        fs::write(&path, [7_u8; 10]).unwrap();
        assert_eq!(read_at_most(&path, 10).unwrap(), [7_u8; 10]);
        assert_eq!(read_at_most(&path, 11).unwrap().len(), 10);
        assert_eq!(read_at_most(&path, 9), Err(InspectError::TooLarge));
        assert_eq!(read_at_most(&path, 0), Err(InspectError::TooLarge));
        fs::write(&path, b"").unwrap();
        assert_eq!(read_at_most(&path, 0).unwrap(), b"");
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = inspect_file(&dir.path().join("core_user_404.dat"));
        assert!(matches!(result, Err(InspectError::Read(_))), "{result:?}");
    }
}
