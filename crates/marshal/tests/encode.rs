//! Byte-level encoder tests: the exact bytes written for each kind of value.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use clone_bay_marshal::{decode, encode, Long, MarshalError, Reduce, Value, MAX_DEPTH};
use common::{bytes, long};

/// The bytes written for `value`, without the two-byte header.
fn body(value: &Value) -> Vec<u8> {
    let stream = encode(value, false).unwrap();
    assert_eq!(&stream[..2], &[0x7D, 1], "version 1 header");
    stream[2..].to_vec()
}

#[test]
fn constants_use_their_one_byte_tags() {
    assert_eq!(body(&Value::None), [1]);
    assert_eq!(body(&Value::Bool(true)), [31]);
    assert_eq!(body(&Value::Bool(false)), [32]);
    assert_eq!(body(&Value::Int(-1)), [7]);
    assert_eq!(body(&Value::Int(0)), [8]);
    assert_eq!(body(&Value::Int(1)), [9]);
    assert_eq!(body(&Value::Float(0.0)), [11]);
    assert_eq!(body(&bytes("")), [14]);
    assert_eq!(body(&Value::Str(String::new())), [40]);
    assert_eq!(body(&Value::Tuple(vec![])), [36]);
    assert_eq!(body(&Value::List(vec![])), [38]);
}

#[test]
fn integers_use_the_smallest_width_that_fits() {
    assert_eq!(body(&Value::Int(2)), [6, 2]);
    assert_eq!(body(&Value::Int(-2)), [6, 0xFE]);
    assert_eq!(body(&Value::Int(127)), [6, 0x7F]);
    assert_eq!(body(&Value::Int(-128)), [6, 0x80]);
    assert_eq!(body(&Value::Int(128)), [5, 0x80, 0x00]);
    assert_eq!(body(&Value::Int(-129)), [5, 0x7F, 0xFF]);
    assert_eq!(body(&Value::Int(32_767)), [5, 0xFF, 0x7F]);
    assert_eq!(body(&Value::Int(-32_768)), [5, 0x00, 0x80]);
    assert_eq!(body(&Value::Int(32_768)), [4, 0x00, 0x80, 0x00, 0x00]);
    assert_eq!(body(&Value::Int(-32_769)), [4, 0xFF, 0x7F, 0xFF, 0xFF]);
    assert_eq!(
        body(&Value::Int(i64::from(i32::MAX))),
        [4, 0xFF, 0xFF, 0xFF, 0x7F]
    );
    assert_eq!(
        body(&Value::Int(i64::from(i32::MIN))),
        [4, 0x00, 0x00, 0x00, 0x80]
    );
    assert_eq!(
        body(&Value::Int(i64::from(i32::MAX) + 1)),
        [3, 0x00, 0x00, 0x00, 0x80, 0, 0, 0, 0]
    );
    assert_eq!(
        body(&Value::Int(i64::from(i32::MIN) - 1)),
        [3, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    assert_eq!(
        body(&Value::Int(i64::MAX)),
        [3, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]
    );
    assert_eq!(body(&Value::Int(i64::MIN)), [3, 0, 0, 0, 0, 0, 0, 0, 0x80]);
}

#[test]
fn longs_are_written_in_minimal_form() {
    assert_eq!(body(&long(0)), [47, 0]);
    assert_eq!(body(&long(1)), [47, 1, 1]);
    assert_eq!(body(&long(-1)), [47, 1, 0xFF]);
    assert_eq!(body(&long(128)), [47, 2, 0x80, 0x00]);
    assert_eq!(
        body(&Value::Long(Long::from_le_bytes(&[5, 0, 0, 0]))),
        [47, 1, 5]
    );
    let filetime = 134_360_688_266_310_454_i64;
    assert_eq!(
        body(&long(filetime)),
        [&[47, 8][..], &filetime.to_le_bytes()].concat()
    );
}

#[test]
fn floats_keep_their_exact_bits_including_the_sign_of_zero() {
    assert_eq!(
        body(&Value::Float(1.5)),
        [&[10][..], &1.5_f64.to_le_bytes()].concat()
    );
    // Only positive zero has the short form. Negative zero is written in
    // full so its sign survives (the Python reference and the blue-marshal
    // crate both lose it).
    assert_eq!(body(&Value::Float(-0.0)), [10, 0, 0, 0, 0, 0, 0, 0, 0x80]);
    let stream = encode(&Value::Float(-0.0), false).unwrap();
    assert_eq!(decode(&stream).unwrap().value, Value::Float(-0.0));
    let nan = f64::from_bits(0x7FF8_0000_0000_1234);
    assert_eq!(
        body(&Value::Float(nan)),
        [&[10][..], &nan.to_le_bytes()].concat()
    );
}

#[test]
fn byte_strings_use_the_buffer_tag() {
    assert_eq!(body(&bytes("a")), [15, b'a']);
    assert_eq!(body(&bytes("ab")), [19, 2, b'a', b'b']);
    // Even a name in the string table is written out in full.
    assert_eq!(body(&bytes("name")), [19, 4, b'n', b'a', b'm', b'e']);
}

#[test]
fn lengths_switch_to_the_wide_form_at_255() {
    let short = body(&Value::Bytes(vec![7; 254]));
    assert_eq!(&short[..2], &[19, 254]);
    assert_eq!(short.len(), 2 + 254);

    let wide = body(&Value::Bytes(vec![7; 255]));
    assert_eq!(&wide[..6], &[19, 0xFF, 255, 0, 0, 0]);
    assert_eq!(wide.len(), 6 + 255);

    let wide = body(&Value::Bytes(vec![7; 70_000]));
    assert_eq!(&wide[..6], &[19, 0xFF, 0x70, 0x11, 0x01, 0x00]);

    let list = body(&Value::List(vec![Value::None; 255]));
    assert_eq!(&list[..6], &[21, 0xFF, 255, 0, 0, 0]);
    assert_eq!(list.len(), 6 + 255);
}

#[test]
fn text_is_written_as_utf8() {
    assert_eq!(body(&Value::Str("a".into())), [46, 1, b'a']);
    assert_eq!(body(&Value::Str("é".into())), [46, 2, 0xC3, 0xA9]);
    assert_eq!(body(&Value::Global("a.b".into())), [2, 3, b'a', b'.', b'b']);
    assert_eq!(body(&Value::Global(String::new())), [2, 0]);
}

#[test]
fn tuples_have_one_and_two_item_forms_and_lists_only_one() {
    let one = Value::Int(1);
    assert_eq!(body(&Value::Tuple(vec![one.clone()])), [37, 9]);
    assert_eq!(body(&Value::Tuple(vec![one.clone(); 2])), [44, 9, 9]);
    assert_eq!(body(&Value::Tuple(vec![one.clone(); 3])), [20, 3, 9, 9, 9]);
    assert_eq!(body(&Value::List(vec![one.clone()])), [39, 9]);
    assert_eq!(body(&Value::List(vec![one.clone(); 2])), [21, 2, 9, 9]);
    assert_eq!(body(&Value::List(vec![one; 3])), [21, 3, 9, 9, 9]);
}

#[test]
fn dict_values_are_written_before_their_keys_in_order() {
    let dict = Value::Dict(vec![
        (bytes("a"), Value::Int(1)),
        (bytes("b"), Value::Int(0)),
    ]);
    assert_eq!(body(&dict), [22, 2, 9, 15, b'a', 8, 15, b'b']);
    assert_eq!(body(&Value::Dict(vec![])), [22, 0]);
}

#[test]
fn instances_write_the_class_as_a_byte_string() {
    let instance = |class: &str| Value::Instance {
        class: class.into(),
        state: Box::new(Value::Int(1)),
    };
    assert_eq!(body(&instance("ab")), [23, 19, 2, b'a', b'b', 9]);
    assert_eq!(body(&instance("c")), [23, 15, b'c', 9]);
    assert_eq!(body(&instance("")), [23, 14, 9]);
}

fn reduce(callable: Option<Value>, state: Option<Value>) -> Reduce {
    Reduce {
        callable,
        args: Value::Tuple(vec![]),
        state,
        list_items: vec![],
        dict_items: vec![],
    }
}

#[test]
fn reduce_and_newobj_layouts() {
    let callable = Value::Global("f".into());
    let plain = Value::Reduce(Box::new(reduce(Some(callable.clone()), None)));
    assert_eq!(body(&plain), [34, 44, 2, 1, b'f', 36, 45, 45]);
    let with_state = Value::Reduce(Box::new(reduce(Some(callable), Some(Value::None))));
    assert_eq!(body(&with_state), [34, 20, 3, 2, 1, b'f', 36, 1, 45, 45]);

    let newobj = Value::Reduce(Box::new(reduce(None, None)));
    assert_eq!(body(&newobj), [35, 37, 36, 45, 45]);
    let newobj_state = Value::Reduce(Box::new(reduce(None, Some(Value::Int(1)))));
    assert_eq!(body(&newobj_state), [35, 44, 36, 9, 45, 45]);

    let mut with_items = reduce(None, None);
    with_items.list_items = vec![Value::Int(1), Value::Int(0)];
    with_items.dict_items = vec![(bytes("k"), Value::Int(-1))];
    // List items, a MARK, dict items as key then value, a MARK.
    assert_eq!(
        body(&Value::Reduce(Box::new(with_items))),
        [35, 37, 36, 9, 8, 45, 15, b'k', 7, 45]
    );
}

#[test]
fn callbacks_wrap_their_value() {
    assert_eq!(body(&Value::Callback(Box::new(Value::Int(1)))), [25, 9]);
}

#[test]
fn checksum_covers_everything_after_it() {
    let stream = encode(&bytes("hi"), true).unwrap();
    // adler32(b"\x13\x02hi") computed with zlib.
    assert_eq!(
        stream,
        [0x7D, 1, 28, 0xE7, 0x00, 0x8F, 0x01, 19, 2, b'h', b'i']
    );
    let decoded = decode(&stream).unwrap();
    assert!(decoded.had_checksum);
    assert_eq!(decoded.value, bytes("hi"));
}

#[test]
fn nesting_is_limited_when_writing_too() {
    let nested = |lists: usize| {
        let mut value = Value::None;
        for _ in 0..lists {
            value = Value::List(vec![value]);
        }
        value
    };
    assert_eq!(MAX_DEPTH, 100);
    let deepest = encode(&nested(99), false).unwrap();
    assert_eq!(deepest.len(), 2 + 99 + 1);
    assert_eq!(encode(&nested(100), false), Err(MarshalError::TooDeep));
}

#[test]
fn depth_is_released_after_each_value_when_writing() {
    let mut deep = Value::None;
    for _ in 0..49 {
        deep = Value::List(vec![deep]);
    }
    let wide = Value::List(vec![deep; 150]);
    assert!(encode(&wide, false).is_ok());
}

#[test]
fn a_reduce_counts_its_payload_tuple_as_a_level() {
    // The payload tuple is a level of its own, as in the reference codec:
    // reduce (1) > tuple (2) > args (3) > ... so 97 lists fit and 98 don't.
    let nested = |lists: usize| {
        let mut value = Value::None;
        for _ in 0..lists {
            value = Value::List(vec![value]);
        }
        let mut reduce = reduce(None, None);
        reduce.args = value;
        Value::Reduce(Box::new(reduce))
    };
    assert!(encode(&nested(97), false).is_ok());
    assert_eq!(encode(&nested(98), false), Err(MarshalError::TooDeep));
}
