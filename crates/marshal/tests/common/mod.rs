//! Helpers shared by the integration tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clone_bay_marshal::{Long, Reduce, Value};
use proptest::prelude::*;

pub fn bytes(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

pub fn long(n: i64) -> Value {
    Value::Long(Long::from_i64(n))
}

/// Random values covering every kind the format has, nested a few levels.
pub fn any_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::None),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        (-300_i64..300).prop_map(Value::Int),
        (-40_000_i64..40_000).prop_map(Value::Int),
        proptest::collection::vec(any::<u8>(), 0..20)
            .prop_map(|raw| Value::Long(Long::from_le_bytes(&raw))),
        any::<f64>().prop_map(Value::Float),
        prop_oneof![Just(0.0), Just(-0.0), Just(f64::NAN), Just(f64::INFINITY)]
            .prop_map(Value::Float),
        proptest::collection::vec(any::<u8>(), 0..6).prop_map(Value::Bytes),
        proptest::collection::vec(any::<u8>(), 250..260).prop_map(Value::Bytes),
        "\\PC{0,12}".prop_map(Value::Str),
        "[a-z]{1,6}(\\.[A-Za-z]{1,6})?".prop_map(Value::Global),
    ];
    leaf.prop_recursive(5, 48, 6, |inner| {
        let pairs = proptest::collection::vec((inner.clone(), inner.clone()), 0..4);
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::Tuple),
            proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::List),
            pairs.clone().prop_map(Value::Dict),
            ("[a-zA-Z.]{0,10}", inner.clone()).prop_map(|(class, state)| Value::Instance {
                class,
                state: Box::new(state),
            }),
            (
                proptest::option::of(inner.clone()),
                inner.clone(),
                proptest::option::of(inner.clone()),
                proptest::collection::vec(inner.clone(), 0..3),
                pairs,
            )
                .prop_map(|(callable, args, state, list_items, dict_items)| {
                    Value::Reduce(Box::new(Reduce {
                        callable,
                        args,
                        state,
                        list_items,
                        dict_items,
                    }))
                }),
            inner.prop_map(|value| Value::Callback(Box::new(value))),
        ]
    })
}

/// Rewrites every `-0.0` as `0.0`.
///
/// The Python reference and the `blue-marshal` crate both drop the sign of a
/// negative zero when writing; this codec keeps it. Tests that compare
/// against them normalise first, so that one known difference doesn't hide
/// any other.
pub fn without_negative_zero(value: &Value) -> Value {
    let fix = without_negative_zero;
    let fix_pairs =
        |pairs: &[(Value, Value)]| pairs.iter().map(|(k, v)| (fix(k), fix(v))).collect();
    match value {
        Value::Float(x) if *x == 0.0 => Value::Float(0.0),
        Value::Tuple(items) => Value::Tuple(items.iter().map(fix).collect()),
        Value::List(items) => Value::List(items.iter().map(fix).collect()),
        Value::Dict(pairs) => Value::Dict(fix_pairs(pairs)),
        Value::Instance { class, state } => Value::Instance {
            class: class.clone(),
            state: Box::new(fix(state)),
        },
        Value::Reduce(reduce) => Value::Reduce(Box::new(Reduce {
            callable: reduce.callable.as_ref().map(fix),
            args: fix(&reduce.args),
            state: reduce.state.as_ref().map(fix),
            list_items: reduce.list_items.iter().map(fix).collect(),
            dict_items: fix_pairs(&reduce.dict_items),
        })),
        Value::Callback(inner) => Value::Callback(Box::new(fix(inner))),
        other => other.clone(),
    }
}

/// What the `blue-marshal` crate makes of a stream: decode it, then write it
/// back keeping the checksum flag.
pub fn crate_reencode(stream: &[u8]) -> Result<Vec<u8>, String> {
    let decoded = blue_marshal::decode(stream).map_err(|error| error.to_string())?;
    let options = blue_marshal::EncodeOptions {
        checksum: decoded.had_crc,
        ..Default::default()
    };
    blue_marshal::encode(&decoded.value, &options).map_err(|error| error.to_string())
}

pub fn from_hex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "odd-length hex");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}
