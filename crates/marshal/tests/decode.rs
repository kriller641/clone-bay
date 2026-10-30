//! Byte-level decoder tests: every tag, the shared-object rules, and every
//! way a stream is refused.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use clone_bay_marshal::{
    decode, Long, MarshalError, Reduce, Value, MAX_DECODED_WEIGHT, MAX_DEPTH, STRING_TABLE,
};
use common::{bytes, long};

const SHARED: u8 = 0x40;

/// A version 1 stream holding `body`.
fn v1(body: &[u8]) -> Vec<u8> {
    [&[0x7D, 1][..], body].concat()
}

/// A version 0 stream holding `body`, with the given shared-object map.
fn v0(body: &[u8], map: &[i32]) -> Vec<u8> {
    let mut stream = vec![0x7E];
    stream.extend_from_slice(&(map.len() as i32).to_le_bytes());
    stream.extend_from_slice(body);
    for entry in map {
        stream.extend_from_slice(&entry.to_le_bytes());
    }
    stream
}

fn value(body: &[u8]) -> Value {
    let decoded = decode(&v1(body)).unwrap_or_else(|e| panic!("{body:02X?}: {e}"));
    assert_eq!(decoded.trailing_bytes, 0, "{body:02X?}");
    decoded.value
}

fn error(body: &[u8]) -> MarshalError {
    decode(&v1(body)).expect_err("stream should be refused")
}

fn tuple(items: &[Value]) -> Value {
    Value::Tuple(items.to_vec())
}

fn list(items: &[Value]) -> Value {
    Value::List(items.to_vec())
}

#[test]
fn simple_constants() {
    assert_eq!(value(&[1]), Value::None);
    assert_eq!(value(&[31]), Value::Bool(true));
    assert_eq!(value(&[32]), Value::Bool(false));
    assert_eq!(value(&[7]), Value::Int(-1));
    assert_eq!(value(&[8]), Value::Int(0));
    assert_eq!(value(&[9]), Value::Int(1));
    assert_eq!(value(&[11]), Value::Float(0.0));
    assert_eq!(value(&[14]), Value::Bytes(vec![]));
    assert_eq!(value(&[40]), Value::Str(String::new()));
    assert_eq!(value(&[36]), tuple(&[]));
    assert_eq!(value(&[38]), list(&[]));
}

#[test]
fn integers_of_each_width_are_signed_little_endian() {
    assert_eq!(value(&[6, 0x80]), Value::Int(-128));
    assert_eq!(value(&[6, 0x7F]), Value::Int(127));
    assert_eq!(value(&[5, 0x00, 0x80]), Value::Int(-32_768));
    assert_eq!(value(&[5, 0x34, 0x12]), Value::Int(0x1234));
    assert_eq!(
        value(&[4, 0x00, 0x00, 0x00, 0x80]),
        Value::Int(-2_147_483_648)
    );
    assert_eq!(value(&[4, 0x78, 0x56, 0x34, 0x12]), Value::Int(0x1234_5678));
    assert_eq!(value(&[3, 0, 0, 0, 0, 0, 0, 0, 0x80]), Value::Int(i64::MIN));
    assert_eq!(
        value(&[3, 8, 7, 6, 5, 4, 3, 2, 1]),
        Value::Int(0x0102_0304_0506_0708)
    );
}

#[test]
fn floats_keep_their_exact_bits() {
    let body = |x: f64| [&[10][..], &x.to_le_bytes()].concat();
    assert_eq!(value(&body(1.5)), Value::Float(1.5));
    assert_eq!(value(&body(-0.0)), Value::Float(-0.0));
    assert_ne!(value(&body(-0.0)), Value::Float(0.0));
    assert_eq!(value(&body(f64::NAN)), Value::Float(f64::NAN));
}

#[test]
fn longs_are_the_same_number_however_they_are_padded() {
    assert_eq!(value(&[47, 0]), long(0));
    assert_eq!(value(&[47, 1, 0]), long(0));
    assert_eq!(value(&[47, 1, 0xFF]), long(-1));
    assert_eq!(value(&[47, 3, 0xFF, 0xFF, 0xFF]), long(-1));
    assert_eq!(value(&[47, 2, 0x80, 0x00]), long(128));
    // A FILETIME timestamp, as every setting carries.
    let filetime = 134_360_688_266_310_454_i64;
    let body = [&[47, 8][..], &filetime.to_le_bytes()].concat();
    assert_eq!(value(&body), long(filetime));
}

#[test]
fn byte_strings_in_every_encoding() {
    assert_eq!(value(&[15, b'a']), bytes("a"));
    assert_eq!(value(&[15, 0xFF]), Value::Bytes(vec![0xFF]));
    assert_eq!(value(&[16, 3, b'a', b'b', b'c']), bytes("abc"));
    assert_eq!(value(&[16, 0]), bytes(""));
    assert_eq!(value(&[13, 2, b'h', b'i']), bytes("hi"));
    assert_eq!(value(&[19, 2, b'h', b'i']), bytes("hi"));
    assert_eq!(value(&[13, 0]), bytes(""));
}

#[test]
fn lengths_of_255_and_more_use_the_wide_form() {
    let payload = vec![b'x'; 300];
    let mut body = vec![19, 0xFF];
    body.extend_from_slice(&300_i32.to_le_bytes());
    body.extend_from_slice(&payload);
    assert_eq!(value(&body), Value::Bytes(payload));
    // The wide form is also legal for a small length.
    assert_eq!(value(&[19, 0xFF, 1, 0, 0, 0, b'z']), bytes("z"));
    // 254 is the largest single-byte length.
    let mut body = vec![19, 254];
    body.extend_from_slice(&[b'y'; 254]);
    assert_eq!(value(&body), Value::Bytes(vec![b'y'; 254]));
}

#[test]
fn string_table_entries_are_one_based() {
    assert_eq!(STRING_TABLE.len(), 195);
    assert_eq!(value(&[17, 1]), bytes("*corpid"));
    assert_eq!(value(&[17, 52]), bytes("name"));
    assert_eq!(value(&[17, 195]), bytes("agent.StorylineMissionDetails"));
    assert_eq!(error(&[17, 0]), MarshalError::BadStringTableIndex(0));
    assert_eq!(error(&[17, 196]), MarshalError::BadStringTableIndex(196));

    let decoded = decode(&v1(&[44, 17, 52, 17, 29])).unwrap();
    assert_eq!(decoded.value, tuple(&[bytes("name"), bytes("flag")]));
    assert_eq!(decoded.string_table_uses, 2);
    assert_eq!(decode(&v1(&[1])).unwrap().string_table_uses, 0);
}

#[test]
fn unicode_in_every_encoding() {
    assert_eq!(value(&[41, 0xE9, 0x00]), Value::Str("é".into()));
    assert_eq!(value(&[18, 2, b'h', 0, b'i', 0]), Value::Str("hi".into()));
    assert_eq!(value(&[18, 0]), Value::Str(String::new()));
    // A character outside the basic plane is two UTF-16 units.
    assert_eq!(
        value(&[18, 2, 0x3D, 0xD8, 0x80, 0xDE]),
        Value::Str("🚀".into())
    );
    assert_eq!(value(&[46, 2, 0xC3, 0xA9]), Value::Str("é".into()));
    assert_eq!(value(&[46, 0]), Value::Str(String::new()));
}

#[test]
fn invalid_text_is_refused() {
    assert_eq!(error(&[46, 1, 0xFF]), MarshalError::InvalidText);
    assert_eq!(error(&[2, 1, 0xFF]), MarshalError::InvalidText);
    // A lone surrogate isn't a character.
    assert_eq!(error(&[41, 0x00, 0xD8]), MarshalError::InvalidText);
    assert_eq!(error(&[18, 1, 0x00, 0xDC]), MarshalError::InvalidText);
}

#[test]
fn globals() {
    assert_eq!(
        value(&[2, 3, b'a', b'.', b'b']),
        Value::Global("a.b".into())
    );
}

#[test]
fn tuples_and_lists_in_every_encoding() {
    assert_eq!(value(&[37, 9]), tuple(&[Value::Int(1)]));
    assert_eq!(value(&[44, 9, 8]), tuple(&[Value::Int(1), Value::Int(0)]));
    assert_eq!(
        value(&[20, 3, 9, 8, 7]),
        tuple(&[Value::Int(1), Value::Int(0), Value::Int(-1)])
    );
    assert_eq!(value(&[20, 0]), tuple(&[]));
    assert_eq!(value(&[39, 9]), list(&[Value::Int(1)]));
    assert_eq!(value(&[21, 2, 9, 8]), list(&[Value::Int(1), Value::Int(0)]));
    assert_eq!(value(&[21, 0]), list(&[]));
}

#[test]
fn dict_values_come_before_their_keys() {
    let body = [22, 2, 9, 15, b'a', 8, 15, b'b'];
    assert_eq!(
        value(&body),
        Value::Dict(vec![
            (bytes("a"), Value::Int(1)),
            (bytes("b"), Value::Int(0))
        ])
    );
    assert_eq!(value(&[22, 0]), Value::Dict(vec![]));
}

#[test]
fn instances_take_their_class_name_from_a_byte_or_unicode_string() {
    let expected = Value::Instance {
        class: "ab".into(),
        state: Box::new(Value::None),
    };
    assert_eq!(value(&[23, 16, 2, b'a', b'b', 1]), expected);
    assert_eq!(value(&[23, 46, 2, b'a', b'b', 1]), expected);
    assert_eq!(error(&[23, 9, 1]), MarshalError::InstanceClassNotString);
    // A global isn't a string. The Python reference accepts one here; the
    // blue-marshal crate doesn't, and neither do we.
    assert_eq!(
        error(&[23, 2, 2, b'a', b'b', 1]),
        MarshalError::InstanceClassNotString
    );
    assert_eq!(error(&[23, 15, 0xFF, 1]), MarshalError::InvalidText);
}

fn reduce(
    callable: Option<Value>,
    args: Value,
    state: Option<Value>,
    list_items: &[Value],
    dict_items: &[(Value, Value)],
) -> Value {
    Value::Reduce(Box::new(Reduce {
        callable,
        args,
        state,
        list_items: list_items.to_vec(),
        dict_items: dict_items.to_vec(),
    }))
}

#[test]
fn reduce_and_newobj() {
    // reduce: (callable, args), no state, no items
    assert_eq!(
        value(&[34, 44, 2, 1, b'f', 36, 45, 45]),
        reduce(Some(Value::Global("f".into())), tuple(&[]), None, &[], &[])
    );
    // reduce: (callable, args, state)
    assert_eq!(
        value(&[34, 20, 3, 2, 1, b'f', 36, 1, 45, 45]),
        reduce(
            Some(Value::Global("f".into())),
            tuple(&[]),
            Some(Value::None),
            &[],
            &[]
        )
    );
    // newobj: (args,)
    assert_eq!(
        value(&[35, 37, 36, 45, 45]),
        reduce(None, tuple(&[]), None, &[], &[])
    );
    // newobj: (args, state)
    assert_eq!(
        value(&[35, 44, 36, 9, 45, 45]),
        reduce(None, tuple(&[]), Some(Value::Int(1)), &[], &[])
    );
    // List items, then dict items as key then value.
    assert_eq!(
        value(&[35, 37, 36, 9, 8, 45, 15, b'k', 7, 45]),
        reduce(
            None,
            tuple(&[]),
            None,
            &[Value::Int(1), Value::Int(0)],
            &[(bytes("k"), Value::Int(-1))]
        )
    );
}

#[test]
fn malformed_reduce_payloads_are_refused() {
    assert_eq!(error(&[34, 38, 45, 45]), MarshalError::ReduceNotTuple);
    assert_eq!(error(&[35, 1, 45, 45]), MarshalError::ReduceNotTuple);
    assert_eq!(error(&[35, 36, 45, 45]), MarshalError::ReduceEmptyNewobj);
    assert_eq!(error(&[34, 36, 45, 45]), MarshalError::ReduceTooShort);
    assert_eq!(error(&[34, 37, 1, 45, 45]), MarshalError::ReduceTooShort);
    // The client would silently ignore the extra item.
    assert_eq!(
        error(&[35, 20, 3, 36, 1, 1, 45, 45]),
        MarshalError::ReduceTooLong
    );
    assert_eq!(
        error(&[34, 20, 4, 1, 36, 1, 1, 45, 45]),
        MarshalError::ReduceTooLong
    );
    // Missing terminators.
    assert_eq!(error(&[35, 37, 36]), MarshalError::UnexpectedEnd);
    assert_eq!(error(&[35, 37, 36, 45]), MarshalError::UnexpectedEnd);
    // A dict item with a key but no value.
    assert_eq!(
        error(&[35, 37, 36, 45, 9, 45]),
        MarshalError::UnsupportedTag(45)
    );
}

#[test]
fn a_mark_with_the_shared_flag_is_not_a_terminator() {
    assert_eq!(
        error(&[35, 37, 36, 45 | SHARED, 45]),
        MarshalError::UnsupportedTag(45)
    );
    assert_eq!(
        error(&[35, 37, 36, 45, 45 | SHARED]),
        MarshalError::UnsupportedTag(45)
    );
}

#[test]
fn callbacks() {
    assert_eq!(value(&[25, 9]), Value::Callback(Box::new(Value::Int(1))));
}

#[test]
fn version_1_references_count_shared_objects_from_zero() {
    let body = [
        20,
        4,
        19 | SHARED,
        2,
        b'a',
        b'a',
        19 | SHARED,
        2,
        b'b',
        b'b',
        27,
        1,
        27,
        0,
    ];
    assert_eq!(
        value(&body),
        tuple(&[bytes("aa"), bytes("bb"), bytes("bb"), bytes("aa")])
    );
}

#[test]
fn a_container_takes_its_slot_before_its_children() {
    // The list is flagged first, so it is slot 0 and the string inside it
    // is slot 1, even though the string is finished first.
    let body = [20, 3, 39 | SHARED, 19 | SHARED, 2, b'x', b'y', 27, 1, 27, 0];
    assert_eq!(
        value(&body),
        tuple(&[list(&[bytes("xy")]), bytes("xy"), list(&[bytes("xy")])])
    );
}

#[test]
fn every_shareable_kind_can_be_referenced() {
    let cases: Vec<(Vec<u8>, Value)> = vec![
        (vec![13 | SHARED, 1, b'a'], bytes("a")),
        (vec![19 | SHARED, 1, b'a'], bytes("a")),
        (vec![47 | SHARED, 1, 5], long(5)),
        (vec![2 | SHARED, 1, b'g'], Value::Global("g".into())),
        (vec![37 | SHARED, 9], tuple(&[Value::Int(1)])),
        (
            vec![44 | SHARED, 9, 8],
            tuple(&[Value::Int(1), Value::Int(0)]),
        ),
        (vec![20 | SHARED, 1, 9], tuple(&[Value::Int(1)])),
        (vec![39 | SHARED, 9], list(&[Value::Int(1)])),
        (vec![21 | SHARED, 1, 9], list(&[Value::Int(1)])),
        (
            vec![22 | SHARED, 1, 9, 8],
            Value::Dict(vec![(Value::Int(0), Value::Int(1))]),
        ),
        (
            vec![23 | SHARED, 15, b'c', 1],
            Value::Instance {
                class: "c".into(),
                state: Box::new(Value::None),
            },
        ),
        (
            vec![35 | SHARED, 37, 36, 45, 45],
            reduce(None, tuple(&[]), None, &[], &[]),
        ),
        (
            vec![34 | SHARED, 44, 1, 36, 45, 45],
            reduce(Some(Value::None), tuple(&[]), None, &[], &[]),
        ),
    ];
    for (encoded, expected) in cases {
        // (shared object, reference to slot 0)
        let body = [&[44][..], &encoded, &[27, 0]].concat();
        assert_eq!(
            value(&body),
            tuple(&[expected.clone(), expected]),
            "{encoded:02X?}"
        );
    }
}

#[test]
fn the_shared_flag_is_ignored_on_kinds_that_cannot_be_shared() {
    let flagged: Vec<(Vec<u8>, Value)> = vec![
        (vec![1], Value::None),
        (vec![31], Value::Bool(true)),
        (vec![32], Value::Bool(false)),
        (vec![7], Value::Int(-1)),
        (vec![8], Value::Int(0)),
        (vec![9], Value::Int(1)),
        (vec![6, 5], Value::Int(5)),
        (vec![5, 5, 0], Value::Int(5)),
        (vec![4, 5, 0, 0, 0], Value::Int(5)),
        (vec![3, 5, 0, 0, 0, 0, 0, 0, 0], Value::Int(5)),
        (vec![11], Value::Float(0.0)),
        (vec![14], bytes("")),
        (vec![15, b'a'], bytes("a")),
        (vec![16, 1, b'a'], bytes("a")),
        (vec![17, 52], bytes("name")),
        (vec![40], Value::Str(String::new())),
        (vec![41, b'a', 0], Value::Str("a".into())),
        (vec![18, 1, b'a', 0], Value::Str("a".into())),
        (vec![46, 1, b'a'], Value::Str("a".into())),
        (vec![36], tuple(&[])),
        (vec![38], list(&[])),
        (vec![25, 1], Value::Callback(Box::new(Value::None))),
    ];
    for (mut encoded, expected) in flagged {
        encoded[0] |= SHARED;
        // In version 0 with no shared slots declared, a kind that did take
        // a slot would fail with SharedOverflow.
        let decoded = decode(&v0(&encoded, &[])).unwrap_or_else(|e| panic!("{encoded:02X?}: {e}"));
        assert_eq!(decoded.value, expected, "{encoded:02X?}");
    }
}

#[test]
fn references_to_unfinished_or_missing_objects_are_refused() {
    // A list that contains a reference to itself.
    assert_eq!(
        error(&[39 | SHARED, 27, 0]),
        MarshalError::IncompleteReference
    );
    assert_eq!(error(&[27, 0]), MarshalError::InvalidReference(0));
    assert_eq!(
        error(&[44, 19 | SHARED, 1, b'a', 27, 1]),
        MarshalError::InvalidReference(1)
    );
    assert_eq!(
        error(&[44, 19 | SHARED, 1, b'a', 27, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        MarshalError::InvalidReference(-1)
    );
}

#[test]
fn version_0_assigns_slots_through_the_map() {
    // The first shared object goes to slot 2, the second to slot 1.
    let body = [
        20,
        4,
        19 | SHARED,
        2,
        b'a',
        b'a',
        19 | SHARED,
        2,
        b'b',
        b'b',
        27,
        1,
        27,
        2,
    ];
    let decoded = decode(&v0(&body, &[2, 1])).unwrap();
    assert_eq!(
        decoded.value,
        tuple(&[bytes("aa"), bytes("bb"), bytes("bb"), bytes("aa")])
    );
    assert_eq!(decoded.version, 0);
    assert_eq!(decoded.unused_shared_slots, 0);
    assert_eq!(decoded.trailing_bytes, 0);
}

#[test]
fn version_0_reference_ids_are_one_based() {
    let body = [44, 19 | SHARED, 1, b'a', 27, 0];
    assert_eq!(
        decode(&v0(&body, &[1])),
        Err(MarshalError::InvalidReference(0))
    );
    let body = [44, 19 | SHARED, 1, b'a', 27, 2];
    assert_eq!(
        decode(&v0(&body, &[1])),
        Err(MarshalError::InvalidReference(2))
    );
    // Slot 2 exists but nothing has filled it.
    let body = [44, 19 | SHARED, 1, b'a', 27, 2];
    assert_eq!(
        decode(&v0(&body, &[1, 2])),
        Err(MarshalError::IncompleteReference)
    );
}

#[test]
fn version_0_header_problems_are_refused() {
    // Negative count.
    assert_eq!(
        decode(&[0x7E, 0xFF, 0xFF, 0xFF, 0xFF, 1]),
        Err(MarshalError::NegativeMapCount)
    );
    // A map that doesn't fit in the file at all, or would overlap the header.
    assert_eq!(
        decode(&[0x7E, 9, 0, 0, 0, 1]),
        Err(MarshalError::TruncatedMap)
    );
    assert_eq!(
        decode(&[0x7E, 1, 0, 0, 0, 1, 0, 0]),
        Err(MarshalError::TruncatedMap)
    );
    assert_eq!(
        decode(&[0x7E, 0xFF, 0xFF, 0xFF, 0x7F, 1]),
        Err(MarshalError::TruncatedMap)
    );
    // Entries must be within 1..=count.
    assert_eq!(decode(&v0(&[1], &[0])), Err(MarshalError::BadMapEntry));
    assert_eq!(decode(&v0(&[1], &[2])), Err(MarshalError::BadMapEntry));
    assert_eq!(decode(&v0(&[1], &[-1])), Err(MarshalError::BadMapEntry));
    assert_eq!(decode(&v0(&[1], &[1, 3])), Err(MarshalError::BadMapEntry));
    // The header is cut short.
    assert_eq!(decode(&[0x7E, 0, 0]), Err(MarshalError::UnexpectedEnd));
}

#[test]
fn a_map_that_exactly_fills_the_file_leaves_no_room_for_content() {
    // Header (5 bytes) followed directly by a one-entry map: legal layout,
    // but there is no value to read.
    assert_eq!(
        decode(&[0x7E, 1, 0, 0, 0, 1, 0, 0, 0]),
        Err(MarshalError::UnexpectedEnd)
    );
}

#[test]
fn version_0_refuses_more_shared_objects_than_declared() {
    assert_eq!(
        decode(&v0(&[19 | SHARED, 1, b'a'], &[])),
        Err(MarshalError::SharedOverflow)
    );
    let body = [44, 19 | SHARED, 1, b'a', 19 | SHARED, 1, b'b'];
    assert_eq!(decode(&v0(&body, &[1])), Err(MarshalError::SharedOverflow));
}

#[test]
fn content_stops_where_the_version_0_map_starts() {
    // The value would need to read into the map to finish.
    assert_eq!(
        decode(&v0(&[19, 4], &[1])),
        Err(MarshalError::UnexpectedEnd)
    );
}

#[test]
fn unused_slots_and_trailing_bytes_are_reported() {
    let decoded = decode(&v0(&[1], &[1, 2])).unwrap();
    assert_eq!(decoded.unused_shared_slots, 2);
    let decoded = decode(&v0(&[44, 19 | SHARED, 1, b'a', 1], &[2, 1, 3])).unwrap();
    assert_eq!(decoded.unused_shared_slots, 2);

    let decoded = decode(&v1(&[1, 9, 9])).unwrap();
    assert_eq!(decoded.value, Value::None);
    assert_eq!(decoded.trailing_bytes, 2);
    let decoded = decode(&v0(&[1, 9], &[1])).unwrap();
    assert_eq!(decoded.trailing_bytes, 1);
}

#[test]
fn headers_and_versions() {
    assert_eq!(decode(&[]), Err(MarshalError::UnexpectedEnd));
    assert_eq!(decode(&[0x7D]), Err(MarshalError::UnexpectedEnd));
    assert_eq!(decode(&[0x7D, 1]), Err(MarshalError::UnexpectedEnd));
    assert_eq!(decode(&[0x00, 1, 1]), Err(MarshalError::BadSignature));
    assert_eq!(decode(&[0x7F, 1, 1]), Err(MarshalError::BadSignature));
    assert_eq!(decode(b"hello"), Err(MarshalError::BadSignature));

    let decoded = decode(&[0x7D, 2, 1]).unwrap();
    assert_eq!((decoded.version, decoded.value), (2, Value::None));
    assert!(!decoded.had_checksum);

    // An explicit version of 0 is read like the version 0 signature.
    let decoded = decode(&[0x7D, 0, 0, 0, 0, 0, 1]).unwrap();
    assert_eq!((decoded.version, decoded.value), (0, Value::None));
    let decoded = decode(&[0x7E, 0, 0, 0, 0, 9]).unwrap();
    assert_eq!((decoded.version, decoded.value), (0, Value::Int(1)));
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[test]
fn version_1_checksum_covers_the_value_only() {
    let body = [19, 2, b'h', b'i'];
    let stream =
        |sum: u32, tail: &[u8]| [&[0x7D, 1, 28][..], &sum.to_le_bytes(), &body, tail].concat();
    let good = adler32(&body);
    let decoded = decode(&stream(good, &[])).unwrap();
    assert_eq!(decoded.value, bytes("hi"));
    assert!(decoded.had_checksum);

    assert_eq!(
        decode(&stream(good ^ 1, &[])),
        Err(MarshalError::ChecksumMismatch)
    );
    // Bytes after the value are outside the checksum.
    let decoded = decode(&stream(good, &[0xAA])).unwrap();
    assert_eq!(decoded.trailing_bytes, 1);
    // Damage inside the value is caught.
    let mut damaged = stream(good, &[]);
    *damaged.last_mut().unwrap() = b'j';
    assert_eq!(decode(&damaged), Err(MarshalError::ChecksumMismatch));
}

#[test]
fn version_0_checksum_covers_the_map_too() {
    let body = [19 | SHARED, 2, b'h', b'i'];
    let map = 1_i32.to_le_bytes();
    let covered = [&body[..], &map].concat();
    let stream = |sum: u32| [&[0x7E, 1, 0, 0, 0, 28][..], &sum.to_le_bytes(), &covered].concat();
    let decoded = decode(&stream(adler32(&covered))).unwrap();
    assert_eq!(decoded.value, bytes("hi"));
    assert!(decoded.had_checksum);
    // A checksum of the value alone is wrong for version 0.
    assert_eq!(
        decode(&stream(adler32(&body))),
        Err(MarshalError::ChecksumMismatch)
    );
}

#[test]
fn nesting_is_limited() {
    assert_eq!(MAX_DEPTH, 100);
    let nested = |lists: usize| [vec![39_u8; lists], vec![1]].concat();
    // 99 lists around a None is 100 levels.
    let mut expected = Value::None;
    for _ in 0..99 {
        expected = list(&[expected]);
    }
    assert_eq!(value(&nested(99)), expected);
    assert_eq!(error(&nested(100)), MarshalError::TooDeep);
    assert_eq!(error(&nested(5_000)), MarshalError::TooDeep);
}

#[test]
fn depth_is_released_after_each_value() {
    // 150 values side by side, each 50 levels deep, must not add up.
    let one = [vec![39_u8; 49], vec![1]].concat();
    let mut body = vec![21, 150];
    for _ in 0..150 {
        body.extend_from_slice(&one);
    }
    let Value::List(items) = value(&body) else {
        panic!("expected a list");
    };
    assert_eq!(items.len(), 150);
}

#[test]
fn negative_and_overlong_lengths_are_refused() {
    let negative = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    for tag in [13, 19, 47, 2, 46, 18, 20, 21, 22] {
        let body = [&[tag][..], &negative].concat();
        assert_eq!(error(&body), MarshalError::NegativeLength, "tag {tag}");
    }
    let huge = [0xFF, 0xFF, 0xFF, 0xFF, 0x7F];
    for tag in [13, 19, 47, 2, 46, 18, 20, 21, 22] {
        let body = [&[tag][..], &huge].concat();
        assert_eq!(error(&body), MarshalError::UnexpectedEnd, "tag {tag}");
    }
}

#[test]
fn truncated_values_are_refused() {
    let truncated: &[&[u8]] = &[
        &[3, 0, 0, 0],
        &[4, 0],
        &[5, 0],
        &[6],
        &[10, 0, 0, 0, 0, 0, 0, 0],
        &[15],
        &[16],
        &[16, 3, b'a'],
        &[17],
        &[13, 2, b'a'],
        &[19, 0xFF, 1, 0],
        &[41, 0],
        &[18, 2, 0, 0],
        &[46, 2, b'a'],
        &[47, 2, 0],
        &[2, 2, b'a'],
        &[37],
        &[44, 1],
        &[20, 2, 1],
        &[39],
        &[21, 2, 1],
        &[22, 1, 1],
        &[23, 15, b'c'],
        &[23],
        &[25],
        &[27],
        &[28, 0, 0, 0],
        &[35],
    ];
    for body in truncated {
        assert_eq!(error(body), MarshalError::UnexpectedEnd, "{body:02X?}");
    }
}

#[test]
fn unsupported_tags_are_refused() {
    // DBRow (42), stream (43), pickle kinds (26, 33), a bare MARK (45) and
    // every unassigned tag.
    for tag in [0_u8, 12, 24, 26, 29, 30, 33, 42, 43, 45, 48, 49, 62, 63] {
        assert_eq!(
            error(&[tag]),
            MarshalError::UnsupportedTag(tag),
            "tag {tag}"
        );
        assert_eq!(
            error(&[tag | SHARED]),
            MarshalError::UnsupportedTag(tag),
            "tag {tag}"
        );
    }
}

#[test]
fn the_top_bit_of_a_tag_is_ignored() {
    // Only the low six bits are the type; 0x80 isn't part of it.
    assert_eq!(value(&[0x80 | 9]), Value::Int(1));
}

/// A tuple holding a shared 999,998-byte string, `references` references to
/// it, and `extra` `None`s. Its decoded weight is
/// `(references + 1) * 1,000,000 + extra`.
fn expanding_stream(references: usize, extra: usize) -> Vec<u8> {
    const PAYLOAD: usize = 999_998;
    let mut body = vec![20, (1 + references + extra) as u8];
    body.extend_from_slice(&[19 | SHARED, 0xFF]);
    body.extend_from_slice(&(PAYLOAD as i32).to_le_bytes());
    body.resize(body.len() + PAYLOAD, b'x');
    for _ in 0..references {
        body.extend_from_slice(&[27, 0]);
    }
    body.resize(body.len() + extra, 1);
    v1(&body)
}

#[test]
fn expansion_through_references_is_limited() {
    assert_eq!(MAX_DECODED_WEIGHT, 16_000_000);
    // Exactly at the limit is accepted.
    let decoded = decode(&expanding_stream(15, 0)).unwrap();
    let Value::Tuple(items) = decoded.value else {
        panic!("expected a tuple");
    };
    assert_eq!(items.len(), 16);
    assert!(items
        .iter()
        .all(|item| *item == Value::Bytes(vec![b'x'; 999_998])));
    // One more node is refused, as is one more whole copy.
    assert_eq!(
        decode(&expanding_stream(15, 1)),
        Err(MarshalError::TooLarge)
    );
    assert_eq!(
        decode(&expanding_stream(16, 0)),
        Err(MarshalError::TooLarge)
    );
}

#[test]
fn a_referenced_container_counts_its_whole_contents() {
    // A shared list holding the big string, then references to the list.
    // If only the list node itself were counted, this would be accepted.
    const PAYLOAD: usize = 999_997;
    let mut body = vec![20, 17, 39 | SHARED, 19, 0xFF];
    body.extend_from_slice(&(PAYLOAD as i32).to_le_bytes());
    body.resize(body.len() + PAYLOAD, b'x');
    for _ in 0..16 {
        body.extend_from_slice(&[27, 0]);
    }
    assert_eq!(decode(&v1(&body)), Err(MarshalError::TooLarge));
}

#[test]
fn a_huge_declared_count_does_not_reserve_memory() {
    // Claims two billion items but holds one. Must fail fast, not allocate.
    for tag in [20, 21, 22] {
        assert_eq!(
            error(&[tag, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 1]),
            MarshalError::UnexpectedEnd
        );
    }
}

#[test]
fn error_messages_are_plain_language() {
    assert_eq!(
        MarshalError::UnexpectedEnd.to_string(),
        "unexpected end of data"
    );
    assert_eq!(
        MarshalError::InvalidReference(7).to_string(),
        "invalid shared reference 7"
    );
    assert_eq!(
        MarshalError::UnsupportedTag(42).to_string(),
        "unsupported marshal type tag 42"
    );
    assert_eq!(
        MarshalError::BadStringTableIndex(0).to_string(),
        "bad string table index 0"
    );
    let all = [
        MarshalError::UnexpectedEnd,
        MarshalError::BadSignature,
        MarshalError::NegativeMapCount,
        MarshalError::TruncatedMap,
        MarshalError::BadMapEntry,
        MarshalError::SharedOverflow,
        MarshalError::InvalidReference(1),
        MarshalError::IncompleteReference,
        MarshalError::ChecksumMismatch,
        MarshalError::TooDeep,
        MarshalError::NegativeLength,
        MarshalError::BadStringTableIndex(1),
        MarshalError::InvalidText,
        MarshalError::InstanceClassNotString,
        MarshalError::ReduceNotTuple,
        MarshalError::ReduceEmptyNewobj,
        MarshalError::ReduceTooShort,
        MarshalError::ReduceTooLong,
        MarshalError::UnsupportedTag(1),
        MarshalError::TooLarge,
        MarshalError::LengthOverflow,
    ];
    let messages: std::collections::HashSet<String> = all.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages.len(),
        all.len(),
        "every error needs its own message"
    );
    assert!(messages.iter().all(|message| !message.is_empty()));
}

#[test]
fn long_helper_matches_the_decoder() {
    assert_eq!(
        value(&[47, 2, 0x2C, 0x01]),
        Value::Long(Long::from_i64(300))
    );
}
