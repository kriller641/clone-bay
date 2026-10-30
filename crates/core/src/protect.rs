//! Protected data: the per-ship and per-module settings a target always
//! keeps when settings are copied onto it.
//!
//! Most are keyed by the target's own ship and module item IDs, so copying
//! the source's would wipe the target's. The chat channel list is protected
//! too: a copy leaves each character in the channels
//! it was in. They are never copied and never
//! merged: the target keeps exactly what it had, including nothing.

use std::borrow::Cow;

use clone_bay_marshal::{decode, MarshalError, Value};
use thiserror::Error;

/// What part of a settings file a rule protects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// A whole top-level section.
    Section(&'static str),
    /// Every key in a top-level section whose name starts with a prefix.
    KeyPrefix {
        section: &'static str,
        prefix: &'static str,
    },
}

/// The rules every copy applies.
pub const PROTECTED: &[Rule] = &[
    // HUD module rack layout, per individual ship.
    Rule::KeyPrefix {
        section: "ui",
        prefix: "slotOrder",
    },
    // Weapon groups.
    Rule::KeyPrefix {
        section: "ui",
        prefix: "linkedWeapons_",
    },
    // Drone groups: lists of individual drone item IDs. The number in the
    // key is the client's own version of the setting, so match any.
    Rule::KeyPrefix {
        section: "ui",
        prefix: "droneBlah",
    },
    // Each ship type's favourite drone group, which names one of the above.
    Rule::KeyPrefix {
        section: "ui",
        prefix: FAVOURITE_DRONES,
    },
    // Auto-repeat and auto-reload, per individual module.
    Rule::Section("autorepeat"),
    Rule::Section("autoreload"),
    // The chat channels a character has joined. A copy never replaces
    // them; the user can choose channels to add (see `groups`).
    Rule::KeyPrefix {
        section: "ui",
        prefix: "chatchannels",
    },
];

const FAVOURITE_DRONES: &str = "drones_favoriteGroupID_";

/// Why a file can't be handled exactly, and so is left alone.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtectError {
    #[error("unexpected structure (top level isn't a dict)")]
    NotADict,
    #[error("file has more than one '{0}' section")]
    DuplicateSection(String),
    #[error("'{0}' section has an unexpected structure")]
    SectionNotADict(String),
    #[error("self-check failed: output doesn't decode ({0})")]
    OutputUnreadable(MarshalError),
    #[error("self-check failed: output has the wrong structure")]
    OutputMalformed,
    #[error("self-check failed: settings differ from the source")]
    SettingsDiffer,
    #[error("self-check failed: protected settings differ from the target")]
    ProtectedDiffer,
}

/// One protected item found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedPart {
    /// The name of the top-level section it is in (or is).
    pub section: String,
    /// The section's key exactly as stored, used when the section has to be
    /// created on the target's behalf.
    section_key: Value,
    /// The key inside the section, or `None` when the whole section is
    /// protected.
    pub key: Option<Value>,
    pub value: Value,
}

impl ProtectedPart {
    /// Whether two parts are the same data in the same place. How the
    /// section's own key is spelled (bytes or text) doesn't matter.
    fn same_as(&self, other: &Self) -> bool {
        self.section == other.section && self.key == other.key && self.value == other.value
    }
}

/// A key's name, when it is text. Most keys are byte strings.
pub fn key_name(key: &Value) -> Option<Cow<'_, str>> {
    match key {
        Value::Bytes(bytes) => Some(String::from_utf8_lossy(bytes)),
        Value::Str(text) => Some(Cow::Borrowed(text.as_str())),
        _ => None,
    }
}

fn is_whole_section(rules: &[Rule], name: &str) -> bool {
    rules
        .iter()
        .any(|rule| matches!(rule, Rule::Section(section) if *section == name))
}

/// The key prefixes protected inside the section called `name`.
fn prefixes_in(rules: &[Rule], name: &str) -> Vec<&'static str> {
    rules
        .iter()
        .filter_map(|rule| match rule {
            Rule::KeyPrefix { section, prefix } if *section == name => Some(*prefix),
            _ => None,
        })
        .collect()
}

fn is_protected_key(prefixes: &[&str], key: &Value) -> bool {
    key_name(key).is_some_and(|name| prefixes.iter().any(|prefix| name.starts_with(prefix)))
}

/// The top-level entries of a settings file, after checking that the rules
/// can be applied to it without guessing.
fn sections<'a>(root: &'a Value, rules: &[Rule]) -> Result<&'a [(Value, Value)], ProtectError> {
    let Value::Dict(pairs) = root else {
        return Err(ProtectError::NotADict);
    };
    for rule in rules {
        let wanted = match rule {
            Rule::Section(section) | Rule::KeyPrefix { section, .. } => *section,
        };
        let mut found = pairs
            .iter()
            .filter(|(key, _)| key_name(key).as_deref() == Some(wanted));
        let first = found.next();
        if found.next().is_some() {
            return Err(ProtectError::DuplicateSection(wanted.to_owned()));
        }
        if let (Rule::KeyPrefix { .. }, Some((_, value))) = (rule, first) {
            if !matches!(value, Value::Dict(_)) {
                return Err(ProtectError::SectionNotADict(wanted.to_owned()));
            }
        }
    }
    Ok(pairs)
}

/// The top-level entries of a settings file, checked against the standard
/// rules. For other modules that need to walk a file the same way.
pub(crate) fn top_level(root: &Value) -> Result<&[(Value, Value)], ProtectError> {
    sections(root, PROTECTED)
}

/// Whether the whole section called `name` is protected.
pub(crate) fn section_is_protected(name: &str) -> bool {
    is_whole_section(PROTECTED, name)
}

/// Whether `key`, inside the section called `section`, is protected.
pub(crate) fn key_is_protected(section: &str, key: &Value) -> bool {
    is_protected_key(&prefixes_in(PROTECTED, section), key)
}

/// Every protected item in a file, in file order.
pub fn protected_parts(root: &Value, rules: &[Rule]) -> Result<Vec<ProtectedPart>, ProtectError> {
    let mut parts = Vec::new();
    for (section_key, value) in sections(root, rules)? {
        let Some(name) = key_name(section_key) else {
            continue;
        };
        if is_whole_section(rules, &name) {
            parts.push(ProtectedPart {
                section: name.into_owned(),
                section_key: section_key.clone(),
                key: None,
                value: value.clone(),
            });
            continue;
        }
        let prefixes = prefixes_in(rules, &name);
        let Value::Dict(inner) = value else {
            continue;
        };
        for (key, value) in inner {
            if is_protected_key(&prefixes, key) {
                parts.push(ProtectedPart {
                    section: name.clone().into_owned(),
                    section_key: section_key.clone(),
                    key: Some(key.clone()),
                    value: value.clone(),
                });
            }
        }
    }
    Ok(parts)
}

/// A file's settings with every protected item taken out.
///
/// A section that only held protected keys is left out altogether, so an
/// empty section and a missing one count as the same thing.
pub fn strip_protected(root: &Value, rules: &[Rule]) -> Result<Value, ProtectError> {
    let mut kept = Vec::new();
    for (section_key, value) in sections(root, rules)? {
        let name = key_name(section_key);
        let name = name.as_deref();
        if name.is_some_and(|name| is_whole_section(rules, name)) {
            continue;
        }
        let prefixes = name
            .map(|name| prefixes_in(rules, name))
            .unwrap_or_default();
        match value {
            Value::Dict(inner) if !prefixes.is_empty() => {
                let inner: Vec<(Value, Value)> = inner
                    .iter()
                    .filter(|(key, _)| !is_protected_key(&prefixes, key))
                    .cloned()
                    .collect();
                if !inner.is_empty() {
                    kept.push((section_key.clone(), Value::Dict(inner)));
                }
            }
            _ => kept.push((section_key.clone(), value.clone())),
        }
    }
    Ok(Value::Dict(kept))
}

/// The source's settings with the target's protected items in place of the
/// source's own. Neither input is modified.
pub fn build_protected_copy(
    source: &Value,
    target: &Value,
    rules: &[Rule],
) -> Result<Value, ProtectError> {
    let Value::Dict(mut result) = strip_protected(source, rules)? else {
        return Err(ProtectError::NotADict);
    };
    for part in protected_parts(target, rules)? {
        let Some(key) = part.key else {
            result.push((part.section_key, part.value));
            continue;
        };
        let existing = result
            .iter_mut()
            .find(|(section_key, _)| key_name(section_key).as_deref() == Some(&part.section));
        match existing {
            Some((_, Value::Dict(inner))) => inner.push((key, part.value)),
            Some(_) => return Err(ProtectError::SectionNotADict(part.section)),
            None => result.push((part.section_key, Value::Dict(vec![(key, part.value)]))),
        }
    }
    Ok(Value::Dict(result))
}

/// Decodes what is about to be written and proves it holds exactly the
/// source's settings plus the target's protected items.
pub fn verify_output(
    data: &[u8],
    source: &Value,
    target: &Value,
    rules: &[Rule],
) -> Result<(), ProtectError> {
    let output = decode(data).map_err(ProtectError::OutputUnreadable)?;
    if output.trailing_bytes != 0 || output.unused_shared_slots != 0 {
        return Err(ProtectError::OutputMalformed);
    }
    let output = output.value;
    let stripped = strip_protected(&output, rules).map_err(|_| ProtectError::OutputMalformed)?;
    if stripped != strip_protected(source, rules)? {
        return Err(ProtectError::SettingsDiffer);
    }
    let kept = protected_parts(&output, rules).map_err(|_| ProtectError::OutputMalformed)?;
    let wanted = protected_parts(target, rules)?;
    if !same_parts(kept, &wanted) {
        return Err(ProtectError::ProtectedDiffer);
    }
    Ok(())
}

/// Whether two lists hold exactly the same protected items, each the same
/// number of times. The order doesn't matter: the items live in dicts, and a
/// copy stores them in a different order from the file they came from.
fn same_parts(mut kept: Vec<ProtectedPart>, wanted: &[ProtectedPart]) -> bool {
    if kept.len() != wanted.len() {
        return false;
    }
    for part in wanted {
        let Some(found) = kept.iter().position(|other| other.same_as(part)) else {
            return false;
        };
        kept.swap_remove(found);
    }
    kept.is_empty()
}

/// Plain-language descriptions of what a target keeps, for showing to the
/// user. Items that hold nothing are left out.
pub fn describe(parts: &[ProtectedPart]) -> Vec<String> {
    // There is one favourite drone group per ship type, each under its own
    // key, so they are counted into one line instead of listed.
    let (favourites, others): (Vec<_>, Vec<_>) = parts.iter().partition(|part| is_favourite(part));
    let mut lines: Vec<String> = others.into_iter().filter_map(describe_part).collect();
    match favourites.len() {
        0 => {}
        1 => lines.push("favourite drone group for 1 ship type".to_owned()),
        n => lines.push(format!("favourite drone group for {n} ship types")),
    }
    lines
}

fn is_favourite(part: &ProtectedPart) -> bool {
    part.section == "ui"
        && part
            .key
            .as_ref()
            .and_then(key_name)
            .is_some_and(|key| key.starts_with(FAVOURITE_DRONES))
}

fn describe_part(part: &ProtectedPart) -> Option<String> {
    let key = part.key.as_ref().and_then(key_name);
    let count = entry_count(&part.value);
    let counted = |what: &str, unit: &str| match count {
        Some(0) => None,
        Some(1) => Some(format!("{what} for 1 {unit}")),
        Some(n) => Some(format!("{what} for {n} {unit}s")),
        None => Some(what.to_owned()),
    };
    match (part.section.as_str(), key.as_deref()) {
        ("ui", Some("slotOrder")) => counted("module rack layout", "ship"),
        ("ui", Some("linkedWeapons_groupsDict")) => counted("weapon groups", "ship"),
        ("ui", Some(key)) if key.starts_with("droneBlah") => match count {
            Some(0) => None,
            Some(1) => Some("1 drone group".to_owned()),
            Some(n) => Some(format!("{n} drone groups")),
            None => Some("drone groups".to_owned()),
        },
        ("autorepeat", None) => counted("auto-repeat settings", "module"),
        ("autoreload", None) => counted("auto-reload settings", "module"),
        ("ui", Some("chatchannels")) => match list_length(&part.value) {
            Some(0) => None,
            Some(1) => Some("1 chat channel".to_owned()),
            Some(n) => Some(format!("{n} chat channels")),
            None => Some("chat channels".to_owned()),
        },
        (_, Some(key)) => Some(key.to_owned()),
        (section, None) => Some(format!("the '{section}' section")),
    }
}

/// How many items a `(timestamp, list)` setting holds.
fn list_length(value: &Value) -> Option<usize> {
    match value {
        Value::Tuple(items) => match items.as_slice() {
            [Value::Long(_), Value::List(entries) | Value::Tuple(entries)] => Some(entries.len()),
            _ => None,
        },
        _ => None,
    }
}

/// How many entries a value holds: a dict, or a `(timestamp, dict)` setting.
fn entry_count(value: &Value) -> Option<usize> {
    match value {
        Value::Dict(pairs) => Some(pairs.len()),
        Value::Tuple(items) => match items.as_slice() {
            [Value::Long(_), Value::Dict(pairs)] => Some(pairs.len()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use clone_bay_marshal::{encode, Long};

    use super::*;

    fn b(text: &str) -> Value {
        Value::Bytes(text.as_bytes().to_vec())
    }

    fn dict(pairs: &[(Value, Value)]) -> Value {
        Value::Dict(pairs.to_vec())
    }

    fn stamped(stamp: i64, value: Value) -> Value {
        Value::Tuple(vec![Value::Long(Long::from_i64(stamp)), value])
    }

    /// A rack layout for the given ship item IDs.
    fn layout(stamp: i64, ships: &[i64]) -> Value {
        let slots = Value::List((11..35).map(Value::Int).collect());
        let ships: Vec<(Value, Value)> = ships
            .iter()
            .map(|&ship| (Value::Long(Long::from_i64(ship)), slots.clone()))
            .collect();
        stamped(stamp, Value::Dict(ships))
    }

    fn modules(ids: &[i64]) -> Value {
        Value::Dict(
            ids.iter()
                .map(|&id| {
                    (
                        Value::Long(Long::from_i64(id)),
                        stamped(9, Value::Int(1000)),
                    )
                })
                .collect(),
        )
    }

    /// An account-style file: protected keys inside `ui`.
    fn account(tag: i64, ships: &[i64]) -> Value {
        dict(&[
            (
                b("overview"),
                dict(&[(b("profile"), stamped(tag, Value::Int(tag)))]),
            ),
            (
                b("ui"),
                dict(&[
                    (b("defaultwarpto"), stamped(tag, Value::Int(tag))),
                    (b("slotOrder"), layout(tag, ships)),
                    (Value::Int(7), stamped(tag, Value::None)),
                    (b("linkedWeapons_groupsDict"), stamped(tag, dict(&[]))),
                ]),
            ),
            (
                b("audio"),
                dict(&[(b("volume"), stamped(tag, Value::Float(0.5)))]),
            ),
        ])
    }

    /// A character-style file: protected top-level sections.
    fn character(tag: i64, repeat: &[i64], reload: &[i64]) -> Value {
        dict(&[
            (b("autoreload"), modules(reload)),
            (
                b("windows"),
                dict(&[(b("open"), stamped(tag, Value::Int(tag)))]),
            ),
            (b("autorepeat"), modules(repeat)),
            (
                b("ui"),
                dict(&[(b("neocom"), stamped(tag, Value::Int(tag)))]),
            ),
        ])
    }

    fn part_names(root: &Value) -> Vec<(String, Option<String>)> {
        protected_parts(root, PROTECTED)
            .unwrap()
            .into_iter()
            .map(|part| {
                let key = part.key.as_ref().and_then(key_name).map(Cow::into_owned);
                (part.section, key)
            })
            .collect()
    }

    fn check(source: &Value, target: &Value) -> Value {
        let built = build_protected_copy(source, target, PROTECTED).unwrap();
        let data = encode(&built, false).unwrap();
        verify_output(&data, source, target, PROTECTED).unwrap();
        built
    }

    #[test]
    fn finds_protected_keys_and_sections_in_file_order() {
        assert_eq!(
            part_names(&account(1, &[5])),
            [
                ("ui".to_owned(), Some("slotOrder".to_owned())),
                ("ui".to_owned(), Some("linkedWeapons_groupsDict".to_owned())),
            ]
        );
        assert_eq!(
            part_names(&character(1, &[5], &[6])),
            [
                ("autoreload".to_owned(), None),
                ("autorepeat".to_owned(), None)
            ]
        );
    }

    #[test]
    fn prefixes_match_the_start_of_a_key_only() {
        let root = dict(&[(
            b("ui"),
            dict(&[
                (b("slotOrderExtra"), Value::Int(1)),
                (Value::Str("linkedWeapons_other".into()), Value::Int(2)),
                (b("myslotOrder"), Value::Int(3)),
                (b("linkedWeapons"), Value::Int(4)),
                (Value::Int(5), Value::Int(5)),
            ]),
        )]);
        let keys: Vec<Option<String>> = part_names(&root).into_iter().map(|(_, k)| k).collect();
        assert_eq!(
            keys,
            [
                Some("slotOrderExtra".to_owned()),
                Some("linkedWeapons_other".to_owned())
            ]
        );
    }

    #[test]
    fn a_protected_name_in_another_section_is_not_protected() {
        let root = dict(&[
            (b("windows"), dict(&[(b("slotOrder"), Value::Int(1))])),
            (b("ui"), dict(&[(b("autorepeat"), Value::Int(2))])),
        ]);
        assert!(part_names(&root).is_empty());
        assert_eq!(strip_protected(&root, PROTECTED).unwrap(), root);
    }

    #[test]
    fn stripping_removes_exactly_the_protected_parts() {
        let stripped = strip_protected(&account(1, &[5]), PROTECTED).unwrap();
        assert_eq!(
            stripped,
            dict(&[
                (
                    b("overview"),
                    dict(&[(b("profile"), stamped(1, Value::Int(1)))])
                ),
                (
                    b("ui"),
                    dict(&[
                        (b("defaultwarpto"), stamped(1, Value::Int(1))),
                        (Value::Int(7), stamped(1, Value::None)),
                    ]),
                ),
                (
                    b("audio"),
                    dict(&[(b("volume"), stamped(1, Value::Float(0.5)))])
                ),
            ])
        );
        let stripped = strip_protected(&character(1, &[5], &[6]), PROTECTED).unwrap();
        assert_eq!(
            stripped,
            dict(&[
                (
                    b("windows"),
                    dict(&[(b("open"), stamped(1, Value::Int(1)))])
                ),
                (b("ui"), dict(&[(b("neocom"), stamped(1, Value::Int(1)))])),
            ])
        );
    }

    #[test]
    fn a_section_left_empty_by_stripping_is_dropped() {
        let root = dict(&[
            (b("ui"), dict(&[(b("slotOrder"), layout(1, &[5]))])),
            (b("x"), dict(&[])),
        ]);
        assert_eq!(
            strip_protected(&root, PROTECTED).unwrap(),
            dict(&[(b("x"), dict(&[]))])
        );
        // An already-empty 'ui' counts the same as a missing one.
        let root = dict(&[(b("ui"), dict(&[]))]);
        assert_eq!(strip_protected(&root, PROTECTED).unwrap(), dict(&[]));
    }

    #[test]
    fn an_account_copy_takes_the_source_settings_and_keeps_the_target_layout() {
        let (source, target) = (account(1, &[10, 11]), account(2, &[20]));
        let built = check(&source, &target);
        assert_eq!(
            strip_protected(&built, PROTECTED).unwrap(),
            strip_protected(&source, PROTECTED).unwrap()
        );
        let kept = protected_parts(&built, PROTECTED).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].value, layout(2, &[20]));
        assert_eq!(kept[1].value, stamped(2, dict(&[])));
    }

    #[test]
    fn a_character_copy_keeps_the_target_module_settings() {
        let (source, target) = (character(1, &[10, 11], &[12]), character(2, &[20], &[]));
        let built = check(&source, &target);
        let kept = protected_parts(&built, PROTECTED).unwrap();
        assert_eq!(
            kept.iter()
                .map(|part| (part.section.as_str(), &part.value))
                .collect::<Vec<_>>(),
            [
                ("autoreload", &modules(&[])),
                ("autorepeat", &modules(&[20]))
            ]
        );
        // The source's own module settings are gone, not merged in.
        let text = format!("{built:?}");
        assert!(!text.contains("Long([10])"), "source module leaked: {text}");
    }

    #[test]
    fn a_target_with_nothing_protected_ends_up_with_nothing_protected() {
        let source = account(1, &[10]);
        let target = dict(&[(b("ui"), dict(&[(b("other"), Value::Int(1))]))]);
        let built = check(&source, &target);
        assert!(protected_parts(&built, PROTECTED).unwrap().is_empty());

        let source = character(1, &[10], &[11]);
        let built = check(&source, &dict(&[]));
        assert!(protected_parts(&built, PROTECTED).unwrap().is_empty());
        assert_eq!(built, strip_protected(&source, PROTECTED).unwrap());
    }

    #[test]
    fn a_source_with_nothing_protected_still_keeps_the_target_parts() {
        let source = dict(&[(b("audio"), dict(&[(b("volume"), Value::Int(1))]))]);
        // The source has no 'ui' at all, so one is created for the target's keys.
        let built = check(&source, &account(2, &[20]));
        assert_eq!(
            built,
            dict(&[
                (b("audio"), dict(&[(b("volume"), Value::Int(1))])),
                (
                    b("ui"),
                    dict(&[
                        (b("slotOrder"), layout(2, &[20])),
                        (b("linkedWeapons_groupsDict"), stamped(2, dict(&[]))),
                    ]),
                ),
            ])
        );
        let built = check(&source, &character(2, &[20], &[21]));
        assert_eq!(part_names(&built).len(), 2);
    }

    #[test]
    fn target_only_settings_do_not_survive_a_copy() {
        let source = account(1, &[10]);
        let Value::Dict(mut pairs) = account(2, &[20]) else {
            unreachable!()
        };
        pairs.push((b("targetOnly"), dict(&[(b("x"), Value::Int(1))])));
        let built = check(&source, &Value::Dict(pairs));
        assert!(!format!("{built:?}").contains("targetOnly"));
    }

    #[test]
    fn inputs_are_not_modified() {
        let (source, target) = (account(1, &[10]), account(2, &[20]));
        let (source_before, target_before) = (source.clone(), target.clone());
        build_protected_copy(&source, &target, PROTECTED).unwrap();
        assert_eq!((source, target), (source_before, target_before));
    }

    #[test]
    fn odd_layouts_are_refused() {
        let not_a_dict = Value::List(vec![]);
        let two_ui = dict(&[(b("ui"), dict(&[])), (Value::Str("ui".into()), dict(&[]))]);
        let two_repeat = dict(&[(b("autorepeat"), dict(&[])), (b("autorepeat"), dict(&[]))]);
        let ui_list = dict(&[(b("ui"), Value::List(vec![]))]);
        let good = account(1, &[1]);
        let cases = [
            (&not_a_dict, ProtectError::NotADict),
            (&two_ui, ProtectError::DuplicateSection("ui".into())),
            (
                &two_repeat,
                ProtectError::DuplicateSection("autorepeat".into()),
            ),
            (&ui_list, ProtectError::SectionNotADict("ui".into())),
        ];
        for (bad, expected) in cases {
            assert_eq!(protected_parts(bad, PROTECTED), Err(expected.clone()));
            assert_eq!(strip_protected(bad, PROTECTED), Err(expected.clone()));
            assert_eq!(
                build_protected_copy(bad, &good, PROTECTED),
                Err(expected.clone())
            );
            assert_eq!(build_protected_copy(&good, bad, PROTECTED), Err(expected));
        }
        // A protected whole section may be any shape; it is kept as it is.
        let odd = dict(&[(b("autorepeat"), Value::Int(3))]);
        assert_eq!(
            protected_parts(&odd, PROTECTED).unwrap()[0].value,
            Value::Int(3)
        );
        // Two unrelated sections with the same name are not our business.
        let twins = dict(&[(b("audio"), dict(&[])), (b("audio"), dict(&[]))]);
        assert_eq!(strip_protected(&twins, PROTECTED).unwrap(), twins);
    }

    #[test]
    fn verification_catches_every_kind_of_difference() {
        let (source, target) = (account(1, &[10]), account(2, &[20]));
        let good = build_protected_copy(&source, &target, PROTECTED).unwrap();
        let verify = |output: &Value| {
            verify_output(&encode(output, false).unwrap(), &source, &target, PROTECTED)
        };
        assert_eq!(verify(&good), Ok(()));

        // The source itself has the wrong layout; the target has the wrong settings.
        assert_eq!(verify(&source), Err(ProtectError::ProtectedDiffer));
        assert_eq!(verify(&target), Err(ProtectError::SettingsDiffer));
        // A layout from some third file.
        let other = build_protected_copy(&source, &account(3, &[30]), PROTECTED).unwrap();
        assert_eq!(verify(&other), Err(ProtectError::ProtectedDiffer));
        // A protected item missing, and one extra.
        let missing = build_protected_copy(&source, &dict(&[]), PROTECTED).unwrap();
        assert_eq!(verify(&missing), Err(ProtectError::ProtectedDiffer));
        let Value::Dict(mut pairs) = good.clone() else {
            unreachable!()
        };
        pairs.push((b("autorepeat"), modules(&[1])));
        assert_eq!(
            verify(&Value::Dict(pairs)),
            Err(ProtectError::ProtectedDiffer)
        );
        // One unprotected setting changed.
        let Value::Dict(mut pairs) = good.clone() else {
            unreachable!()
        };
        pairs[0].1 = dict(&[(b("profile"), stamped(1, Value::Int(99)))]);
        assert_eq!(
            verify(&Value::Dict(pairs)),
            Err(ProtectError::SettingsDiffer)
        );
        // One unprotected section missing.
        let Value::Dict(mut pairs) = good.clone() else {
            unreachable!()
        };
        pairs.remove(0);
        assert_eq!(
            verify(&Value::Dict(pairs)),
            Err(ProtectError::SettingsDiffer)
        );
    }

    #[test]
    fn verification_refuses_output_that_is_not_a_clean_settings_file() {
        let (source, target) = (account(1, &[10]), account(2, &[20]));
        let good = build_protected_copy(&source, &target, PROTECTED).unwrap();
        let data = encode(&good, false).unwrap();
        let verify = |data: &[u8]| verify_output(data, &source, &target, PROTECTED);

        assert_eq!(
            verify(&data[..data.len() - 1]),
            Err(ProtectError::OutputUnreadable(MarshalError::UnexpectedEnd))
        );
        let mut trailing = data.clone();
        trailing.push(1);
        assert_eq!(verify(&trailing), Err(ProtectError::OutputMalformed));
        // Version 0 with a declared shared slot nothing uses.
        let unused = [&[0x7E, 1, 0, 0, 0][..], &data[2..], &[1, 0, 0, 0]].concat();
        assert_eq!(verify(&unused), Err(ProtectError::OutputMalformed));
        let list = encode(&Value::List(vec![]), false).unwrap();
        assert_eq!(verify(&list), Err(ProtectError::OutputMalformed));
    }

    #[test]
    fn a_section_key_spelled_as_text_still_matches() {
        // The target spells 'ui' as text, the source as bytes.
        let source = account(1, &[10]);
        let target = dict(&[(
            Value::Str("ui".into()),
            dict(&[(b("slotOrder"), layout(2, &[20]))]),
        )]);
        let built = check(&source, &target);
        let kept = protected_parts(&built, PROTECTED).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].value, layout(2, &[20]));
    }

    #[test]
    fn descriptions_are_plain_language_and_skip_empty_items() {
        let parts = protected_parts(&account(1, &[5, 6]), PROTECTED).unwrap();
        // The empty weapon groups are kept but not worth mentioning.
        assert_eq!(describe(&parts), ["module rack layout for 2 ships"]);

        let parts = protected_parts(&character(1, &[5], &[6, 7]), PROTECTED).unwrap();
        assert_eq!(
            describe(&parts),
            [
                "auto-reload settings for 2 modules",
                "auto-repeat settings for 1 module"
            ]
        );

        let odd = dict(&[
            (
                b("ui"),
                dict(&[
                    (
                        b("linkedWeapons_groupsDict"),
                        stamped(1, dict(&[(Value::Int(1), Value::None)])),
                    ),
                    (b("linkedWeapons_new"), Value::Int(1)),
                    (b("slotOrder"), Value::Int(1)),
                ]),
            ),
            (b("autorepeat"), Value::Int(1)),
        ]);
        let parts = protected_parts(&odd, PROTECTED).unwrap();
        assert_eq!(
            describe(&parts),
            [
                "weapon groups for 1 ship",
                "linkedWeapons_new",
                "module rack layout",
                "auto-repeat settings"
            ]
        );
        let custom = [Rule::Section("zz")];
        let parts = protected_parts(&dict(&[(b("zz"), Value::None)]), &custom).unwrap();
        assert_eq!(describe(&parts), ["the 'zz' section"]);
    }

    #[test]
    fn drone_groups_and_favourites_stay_the_targets() {
        let drones = |tag: i64, ids: &[i64]| {
            stamped(
                tag,
                dict(&[(
                    Value::Str("heavies".into()),
                    dict(&[(
                        b("droneIDs"),
                        Value::List(ids.iter().map(|id| Value::Int(*id)).collect()),
                    )]),
                )]),
            )
        };
        let favourite = |tag: i64| stamped(tag, Value::Str("heavies".into()));
        let file = |tag: i64, ids: &[i64], ship: &str| {
            dict(&[(
                b("ui"),
                dict(&[
                    (b("droneBlah2"), drones(tag, ids)),
                    (b(ship), favourite(tag)),
                    (b("dronesOther"), Value::Int(tag)),
                ]),
            )])
        };
        let source = file(1, &[10, 11], "drones_favoriteGroupID_100");
        let target = file(2, &[20], "drones_favoriteGroupID_200");
        let built = check(&source, &target);

        let ui = top_level(&built).unwrap();
        let ui = match &ui.iter().find(|(key, _)| *key == b("ui")).unwrap().1 {
            Value::Dict(pairs) => pairs.clone(),
            other => panic!("ui is {other:?}"),
        };
        let get = |name: &str| {
            ui.iter()
                .find(|(key, _)| *key == b(name))
                .map(|pair| &pair.1)
        };
        assert_eq!(get("droneBlah2"), Some(&drones(2, &[20])));
        assert_eq!(get("drones_favoriteGroupID_200"), Some(&favourite(2)));
        // The source's favourite names a group the target doesn't have.
        assert_eq!(get("drones_favoriteGroupID_100"), None);
        // A key that merely starts with "drones" is an ordinary setting.
        assert_eq!(get("dronesOther"), Some(&Value::Int(1)));

        let parts = protected_parts(&built, PROTECTED).unwrap();
        assert_eq!(
            describe(&parts),
            ["1 drone group", "favourite drone group for 1 ship type"]
        );
        let two = dict(&[(
            b("ui"),
            dict(&[
                (b("drones_favoriteGroupID_1"), favourite(1)),
                (b("drones_favoriteGroupID_2"), favourite(1)),
                (b("droneBlah2"), stamped(1, dict(&[]))),
                (b("droneBlah3"), Value::Int(1)),
            ]),
        )]);
        let parts = protected_parts(&two, PROTECTED).unwrap();
        assert_eq!(
            describe(&parts),
            ["drone groups", "favourite drone group for 2 ship types"]
        );
    }

    #[test]
    fn chat_channels_are_described_by_how_many_there_are() {
        let described = |value: Value| {
            let root = dict(&[(b("ui"), dict(&[(b("chatchannels"), value)]))]);
            describe(&protected_parts(&root, PROTECTED).unwrap())
        };
        let channels = |count: i64| Value::List((0..count).map(Value::Int).collect());
        assert_eq!(described(stamped(1, channels(3))), ["3 chat channels"]);
        assert_eq!(described(stamped(1, channels(1))), ["1 chat channel"]);
        // An empty list holds nothing worth mentioning.
        assert!(described(stamped(1, channels(0))).is_empty());
        // The list may be stored as a tuple.
        let as_tuple = Value::Tuple(vec![Value::Int(7), Value::Int(8)]);
        assert_eq!(described(stamped(1, as_tuple)), ["2 chat channels"]);
        // Anything else is still named, without a count.
        assert_eq!(described(Value::Int(5)), ["chat channels"]);
        assert_eq!(described(stamped(1, Value::Int(5))), ["chat channels"]);
        assert_eq!(
            described(Value::Tuple(vec![channels(2), channels(2)])),
            ["chat channels"]
        );
        assert_eq!(described(channels(2)), ["chat channels"]);
    }

    /// A character whose module settings are stored before its `ui` section
    /// has its protected items in a different order from the copy built for
    /// it, where whole sections are added last. Found with a real file on
    /// 2026-10-10: the self-check compared them in order and refused.
    #[test]
    fn the_order_protected_items_are_stored_in_does_not_matter() {
        let chat = |stamp: i64| stamped(stamp, Value::List(vec![Value::Int(stamp)]));
        let source = dict(&[
            (
                b("ui"),
                dict(&[(b("chatchannels"), chat(1)), (b("other"), Value::Int(1))]),
            ),
            (b("autorepeat"), modules(&[1])),
        ]);
        let target = dict(&[
            (b("autorepeat"), modules(&[2])),
            (b("autoreload"), modules(&[3])),
            (
                b("ui"),
                dict(&[(b("other"), Value::Int(2)), (b("chatchannels"), chat(2))]),
            ),
        ]);
        let output = build_protected_copy(&source, &target, PROTECTED).unwrap();
        let data = encode(&output, false).unwrap();
        verify_output(&data, &source, &target, PROTECTED).unwrap();

        // Order aside, the check is as strict as before: one item changed,
        // missing, or there twice is still refused.
        let refused = |output: &Value| {
            let data = encode(output, false).unwrap();
            verify_output(&data, &source, &target, PROTECTED) == Err(ProtectError::ProtectedDiffer)
        };
        let Value::Dict(sections) = &output else {
            unreachable!()
        };
        let without = |name: &str| {
            Value::Dict(
                sections
                    .iter()
                    .filter(|(key, _)| key != &b(name))
                    .cloned()
                    .collect(),
            )
        };
        assert!(refused(&without("autoreload")));
        let mut changed = sections.clone();
        let reload = changed.iter_mut().find(|(key, _)| key == &b("autoreload"));
        reload.unwrap().1 = modules(&[2]);
        assert!(refused(&Value::Dict(changed)));
        // The target's auto-repeat settings in place of its auto-reload
        // ones: the right number of items, but not the right items.
        let mut swapped = sections.clone();
        let reload = swapped.iter_mut().find(|(key, _)| key == &b("autoreload"));
        *reload.unwrap() = (b("autoreload"), modules(&[2]));
        assert!(refused(&Value::Dict(swapped)));
    }
}
