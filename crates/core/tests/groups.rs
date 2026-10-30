//! Copying only chosen groups of settings.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use clone_bay_core::discovery::FileKind;
use clone_bay_core::groups::{
    build_selected, channels_added, chat_channels, groups_for, has_chat_channels, same_settings,
    verify_selected, GroupError, Part, Selection,
};
use clone_bay_core::protect::{build_protected_copy, protected_parts, ProtectError, PROTECTED};
use clone_bay_marshal::{encode, Long, Value};

fn b(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

fn dict(pairs: &[(Value, Value)]) -> Value {
    Value::Dict(pairs.to_vec())
}

fn stamped(stamp: i64, value: Value) -> Value {
    Value::Tuple(vec![Value::Long(Long::from_i64(stamp)), value])
}

/// A setting whose value says which file it came from.
fn from(tag: &str) -> Value {
    stamped(1, s(tag))
}

fn chosen(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn only(groups: &[&str]) -> Selection {
    Selection {
        groups: chosen(groups),
        ..Selection::default()
    }
}

/// An account file where every value is `tag`, with `extra` as one more
/// probe-scanning key.
fn account(tag: &str, extra_probe_key: &str) -> Value {
    dict(&[
        (b("cmd"), dict(&[(b("customCmds"), from(tag))])),
        (b("overview"), dict(&[(b("profile"), from(tag))])),
        (b("defaultoverview"), dict(&[(b("x"), from(tag))])),
        (b("audio"), dict(&[(b("volume"), from(tag))])),
        (b("notifications"), dict(&[(b("n"), from(tag))])),
        (
            b("ui"),
            dict(&[
                (b("probescanning.selectedFormationID"), from(tag)),
                (b(extra_probe_key), from(tag)),
                (b("cameraShakeEnabled"), from(tag)),
                (b("market_filter_price"), from(tag)),
                (b("defaultwarpto"), from(tag)),
                (Value::Int(7), from(tag)),
                (b("slotOrder"), from(tag)),
                (b("linkedWeapons_groupsDict"), from(tag)),
            ]),
        ),
    ])
}

fn channel(kind: &str, id: &str, name: &str) -> Value {
    Value::Tuple(vec![b(kind), s(id), s(name)])
}

fn player(id: &str, name: &str) -> Value {
    // The game stores a player channel's ID in both of the first two fields.
    Value::Tuple(vec![s(id), s(id), s(name)])
}

fn character(tag: &str, channels: Option<&[Value]>) -> Value {
    let mut ui = vec![
        (b("neocomButtonRawData"), from(tag)),
        (b("InfoPanelModes_inflight"), from(tag)),
        (b("defaultTypeOrbitDist"), from(tag)),
        (b("listgroups"), from(tag)),
    ];
    if let Some(channels) = channels {
        ui.push((
            b("chatchannels"),
            stamped(5, Value::List(channels.to_vec())),
        ));
    }
    dict(&[
        (b("autorepeat"), dict(&[(Value::Int(1), from(tag))])),
        (b("windows"), dict(&[(b("open"), from(tag))])),
        (b("dockPanels"), dict(&[(b("d"), from(tag))])),
        (b("notepad"), dict(&[(b("n"), from(tag))])),
        (b("ui"), Value::Dict(ui)),
    ])
}

/// Which file each named setting came from, e.g. `("ui", "defaultwarpto")`.
fn origin(root: &Value, section: &str, key: &str) -> Option<String> {
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    let (_, section) = sections.iter().find(|(name, _)| *name == b(section))?;
    let Value::Dict(inner) = section else {
        panic!("section isn't a dict");
    };
    let (_, value) = inner
        .iter()
        .find(|(name, _)| *name == b(key) || (key == "7" && *name == Value::Int(7)))?;
    let Value::Tuple(pair) = value else {
        panic!("not stamped");
    };
    match &pair[1] {
        Value::Str(tag) => Some(tag.clone()),
        _ => panic!("no tag"),
    }
}

fn check(source: &Value, target: &Value, kind: FileKind, selection: &Selection) -> Value {
    let built = build_selected(source, target, kind, selection).unwrap();
    let data = encode(&built, false).unwrap();
    verify_selected(&data, source, target, kind, selection).unwrap();
    built
}

#[test]
fn every_group_is_distinct_and_none_touches_protected_data() {
    for kind in [FileKind::Account, FileKind::Character] {
        let groups = groups_for(kind);
        let ids: BTreeSet<&str> = groups.iter().map(|group| group.id).collect();
        assert_eq!(ids.len(), groups.len(), "duplicate group id");
        let parts: Vec<&Part> = groups.iter().flat_map(|group| group.parts).collect();
        for (n, part) in parts.iter().enumerate() {
            assert!(!parts[..n].contains(part), "{part:?} is in two groups");
            match part {
                Part::Section(name) => {
                    assert!(!["ui", "autorepeat", "autoreload"].contains(name), "{name}");
                }
                Part::UiPrefix(prefix) => {
                    for protected in ["slotOrder", "linkedWeapons_"] {
                        assert!(
                            !protected.starts_with(prefix) && !prefix.starts_with(protected),
                            "{prefix} overlaps {protected}"
                        );
                    }
                    assert!(!"chatchannels".starts_with(prefix), "{prefix} covers chat");
                }
            }
        }
        assert!(groups
            .iter()
            .all(|group| !group.label.is_empty() && !group.parts.is_empty()));
    }
    assert!(has_chat_channels(FileKind::Character));
    assert!(!has_chat_channels(FileKind::Account));
}

#[test]
fn a_section_group_takes_just_that_section_from_the_source() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let built = check(&source, &target, FileKind::Account, &only(&["keybinds"]));
    assert_eq!(origin(&built, "cmd", "customCmds").as_deref(), Some("S"));
    for (section, key) in [
        ("overview", "profile"),
        ("audio", "volume"),
        ("notifications", "n"),
        ("ui", "probescanning.selectedFormationID"),
        ("ui", "cameraShakeEnabled"),
        ("ui", "defaultwarpto"),
        ("ui", "7"),
        ("ui", "slotOrder"),
    ] {
        assert_eq!(
            origin(&built, section, key).as_deref(),
            Some("T"),
            "{section}/{key}"
        );
    }
}

#[test]
fn a_group_can_span_two_sections() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let built = check(&source, &target, FileKind::Account, &only(&["overview"]));
    assert_eq!(origin(&built, "overview", "profile").as_deref(), Some("S"));
    assert_eq!(origin(&built, "defaultoverview", "x").as_deref(), Some("S"));
    assert_eq!(origin(&built, "cmd", "customCmds").as_deref(), Some("T"));
}

#[test]
fn a_prefix_group_replaces_matching_keys_and_drops_the_targets_own() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let built = check(&source, &target, FileKind::Account, &only(&["probes"]));
    assert_eq!(
        origin(&built, "ui", "probescanning.selectedFormationID").as_deref(),
        Some("S")
    );
    assert_eq!(
        origin(&built, "ui", "probescanning.a").as_deref(),
        Some("S")
    );
    // The target's own probe key isn't in the source, so it goes.
    assert_eq!(origin(&built, "ui", "probescanning.b"), None);
    // Other keys in the same section are untouched.
    assert_eq!(
        origin(&built, "ui", "cameraShakeEnabled").as_deref(),
        Some("T")
    );
    assert_eq!(
        origin(&built, "ui", "market_filter_price").as_deref(),
        Some("T")
    );
    assert_eq!(origin(&built, "ui", "defaultwarpto").as_deref(), Some("T"));
}

#[test]
fn everything_else_covers_what_no_group_does_and_nothing_more() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let selection = Selection {
        everything_else: true,
        ..Selection::default()
    };
    let built = check(&source, &target, FileKind::Account, &selection);
    assert_eq!(origin(&built, "notifications", "n").as_deref(), Some("S"));
    assert_eq!(origin(&built, "ui", "defaultwarpto").as_deref(), Some("S"));
    assert_eq!(
        origin(&built, "ui", "7").as_deref(),
        Some("S"),
        "non-text keys are 'else'"
    );
    assert_eq!(origin(&built, "cmd", "customCmds").as_deref(), Some("T"));
    assert_eq!(
        origin(&built, "ui", "cameraShakeEnabled").as_deref(),
        Some("T")
    );
    assert_eq!(
        origin(&built, "ui", "probescanning.b").as_deref(),
        Some("T")
    );
    assert_eq!(
        origin(&built, "ui", "slotOrder").as_deref(),
        Some("T"),
        "protected"
    );
}

#[test]
fn protected_data_stays_with_the_target_whatever_is_chosen() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let everything = Selection {
        groups: groups_for(FileKind::Account)
            .iter()
            .map(|g| g.id.to_owned())
            .collect(),
        everything_else: true,
        chat_channels: None,
    };
    let built = check(&source, &target, FileKind::Account, &everything);
    assert_eq!(origin(&built, "ui", "slotOrder").as_deref(), Some("T"));
    assert_eq!(
        origin(&built, "ui", "linkedWeapons_groupsDict").as_deref(),
        Some("T")
    );
    assert_eq!(
        protected_parts(&built, PROTECTED).unwrap(),
        protected_parts(&target, PROTECTED).unwrap()
    );

    let (source, target) = (character("S", None), character("T", None));
    let everything = Selection {
        groups: groups_for(FileKind::Character)
            .iter()
            .map(|g| g.id.to_owned())
            .collect(),
        everything_else: true,
        chat_channels: None,
    };
    let built = check(&source, &target, FileKind::Character, &everything);
    assert_eq!(
        protected_parts(&built, PROTECTED).unwrap(),
        protected_parts(&target, PROTECTED).unwrap()
    );
}

#[test]
fn choosing_everything_matches_a_full_copy() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let everything = Selection {
        groups: groups_for(FileKind::Account)
            .iter()
            .map(|g| g.id.to_owned())
            .collect(),
        everything_else: true,
        chat_channels: None,
    };
    let selective = check(&source, &target, FileKind::Account, &everything);
    let full = build_protected_copy(&source, &target, PROTECTED).unwrap();
    assert!(same_settings(&selective, &full, FileKind::Account).unwrap());
    assert!(!same_settings(&selective, &target, FileKind::Account).unwrap());
}

#[test]
fn choosing_nothing_changes_nothing() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    assert!(Selection::default().is_empty());
    let built = check(&source, &target, FileKind::Account, &Selection::default());
    assert!(same_settings(&built, &target, FileKind::Account).unwrap());
    // An unknown group ID is ignored, not an error.
    let built = check(&source, &target, FileKind::Account, &only(&["nonsense"]));
    assert!(same_settings(&built, &target, FileKind::Account).unwrap());
    assert!(!only(&["keybinds"]).is_empty());
}

#[test]
fn account_groups_do_not_apply_to_character_files() {
    // 'windows' is a character group; in an account file it is "else".
    let mut source = account("S", "probescanning.a");
    let mut target = account("T", "probescanning.b");
    for (root, tag) in [(&mut source, "S"), (&mut target, "T")] {
        let Value::Dict(sections) = root else {
            unreachable!()
        };
        sections.push((b("windows"), dict(&[(b("open"), from(tag))])));
    }
    let built = check(&source, &target, FileKind::Account, &only(&["windows"]));
    assert_eq!(origin(&built, "windows", "open").as_deref(), Some("T"));
}

#[test]
fn character_groups_work_on_sections_and_prefixes() {
    let (source, target) = (character("S", None), character("T", None));
    let built = check(
        &source,
        &target,
        FileKind::Character,
        &only(&["windows", "distances"]),
    );
    assert_eq!(origin(&built, "windows", "open").as_deref(), Some("S"));
    assert_eq!(
        origin(&built, "ui", "defaultTypeOrbitDist").as_deref(),
        Some("S")
    );
    assert_eq!(
        origin(&built, "ui", "neocomButtonRawData").as_deref(),
        Some("T")
    );
    assert_eq!(
        origin(&built, "ui", "InfoPanelModes_inflight").as_deref(),
        Some("T")
    );
    assert_eq!(origin(&built, "ui", "listgroups").as_deref(), Some("T"));
    assert_eq!(origin(&built, "dockPanels", "d").as_deref(), Some("T"));
    assert_eq!(origin(&built, "notepad", "n").as_deref(), Some("T"));
    assert_eq!(origin(&built, "autorepeat", "1"), None);
}

fn source_channels() -> Vec<Value> {
    vec![
        channel("corp", "corp_1", "Corp"),
        channel("alliance", "alliance_1", "Alliance"),
        channel("local", "local_1", "Local"),
        player("player_aaa", "Traders"),
        player("player_bbb", "Scouts"),
    ]
}

fn target_channels() -> Vec<Value> {
    vec![
        channel("corp", "corp_2", "Corp"),
        channel("local", "local_2", "Local"),
        player("player_bbb", "Scouts"),
    ]
}

fn chat(ids: &[&str]) -> Selection {
    Selection {
        chat_channels: Some(chosen(ids)),
        ..Selection::default()
    }
}

fn channel_ids(root: &Value) -> Vec<String> {
    chat_channels(root)
        .unwrap()
        .into_iter()
        .map(|channel| channel.id)
        .collect()
}

#[test]
fn chat_channels_are_listed_with_whether_they_can_be_copied() {
    let root = character("S", Some(&source_channels()));
    let channels = chat_channels(&root).unwrap();
    let summary: Vec<(&str, &str, bool)> = channels
        .iter()
        .map(|channel| {
            (
                channel.id.as_str(),
                channel.name.as_str(),
                channel.can_be_copied(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("corp_1", "Corp", false),
            ("alliance_1", "Alliance", false),
            ("local_1", "Local", false),
            ("player_aaa", "Traders", true),
            ("player_bbb", "Scouts", true),
        ]
    );
    assert!(chat_channels(&character("S", None)).unwrap().is_empty());
    assert!(chat_channels(&character("S", Some(&[])))
        .unwrap()
        .is_empty());
}

#[test]
fn chosen_channels_are_added_after_the_targets_own() {
    let source = character("S", Some(&source_channels()));
    let target = character("T", Some(&target_channels()));
    let selection = chat(&["player_aaa", "player_bbb"]);
    let built = check(&source, &target, FileKind::Character, &selection);
    // The target's list, in order, then the one channel it lacked.
    assert_eq!(
        channel_ids(&built),
        ["corp_2", "local_2", "player_bbb", "player_aaa"]
    );
    assert_eq!(channels_added(&target, &built), 1);
    // Nothing else moved.
    assert_eq!(origin(&built, "windows", "open").as_deref(), Some("T"));
    assert_eq!(origin(&built, "ui", "listgroups").as_deref(), Some("T"));
}

#[test]
fn only_chosen_channels_are_added_and_never_the_sources_corp_alliance_or_local() {
    let source = character("S", Some(&source_channels()));
    let target = character("T", Some(&target_channels()));
    let greedy = chat(&["corp_1", "alliance_1", "local_1", "player_bbb"]);
    let built = check(&source, &target, FileKind::Character, &greedy);
    assert_eq!(channel_ids(&built), ["corp_2", "local_2", "player_bbb"]);
    assert!(same_settings(&built, &target, FileKind::Character).unwrap());
    assert_eq!(channels_added(&target, &built), 0);

    // Ticking the group with no channels chosen adds nothing either.
    let built = check(&source, &target, FileKind::Character, &chat(&[]));
    assert!(same_settings(&built, &target, FileKind::Character).unwrap());
    assert!(chat(&[]).is_empty());
}

#[test]
fn a_target_with_no_channel_list_gets_one_with_just_the_chosen_channels() {
    let source = character("S", Some(&source_channels()));
    let target = character("T", None);
    let built = check(
        &source,
        &target,
        FileKind::Character,
        &chat(&["player_aaa"]),
    );
    assert_eq!(channel_ids(&built), ["player_aaa"]);
    assert_eq!(channels_added(&target, &built), 1);

    // A source with no list has nothing to add.
    let source = character("S", None);
    let target = character("T", Some(&target_channels()));
    let built = check(
        &source,
        &target,
        FileKind::Character,
        &chat(&["player_aaa"]),
    );
    assert!(same_settings(&built, &target, FileKind::Character).unwrap());
}

#[test]
fn a_channel_list_stored_as_a_tuple_stays_a_tuple() {
    let source = character("S", Some(&source_channels()));
    let mut target = character("T", None);
    let Value::Dict(sections) = &mut target else {
        unreachable!()
    };
    let Value::Dict(ui) = &mut sections[4].1 else {
        unreachable!()
    };
    ui.push((
        b("chatchannels"),
        stamped(5, Value::Tuple(target_channels())),
    ));

    let built = check(
        &source,
        &target,
        FileKind::Character,
        &chat(&["player_aaa"]),
    );
    assert_eq!(
        channel_ids(&built),
        ["corp_2", "local_2", "player_bbb", "player_aaa"]
    );
    let Value::Dict(sections) = &built else {
        unreachable!()
    };
    let stored = sections
        .iter()
        .filter_map(|(_, section)| match section {
            Value::Dict(inner) => inner.iter().find(|(key, _)| *key == b("chatchannels")),
            _ => None,
        })
        .map(|(_, value)| value)
        .next()
        .unwrap();
    let Value::Tuple(pair) = stored else {
        panic!("not a (timestamp, list) pair");
    };
    assert!(
        matches!(&pair[1], Value::Tuple(entries) if entries.len() == 4),
        "{:?}",
        pair[1]
    );
}

#[test]
fn leaving_chat_unticked_keeps_the_targets_list_even_when_all_else_is_copied() {
    let source = character("S", Some(&source_channels()));
    let target = character("T", Some(&target_channels()));
    let selection = Selection {
        groups: groups_for(FileKind::Character)
            .iter()
            .map(|g| g.id.to_owned())
            .collect(),
        everything_else: true,
        chat_channels: None,
    };
    let built = check(&source, &target, FileKind::Character, &selection);
    assert_eq!(channel_ids(&built), ["corp_2", "local_2", "player_bbb"]);
    assert_eq!(origin(&built, "ui", "listgroups").as_deref(), Some("S"));
}

#[test]
fn a_channel_list_with_an_unexpected_shape_is_refused() {
    let odd_lists = [
        from("not a list"),
        stamped(5, Value::List(vec![Value::Int(1)])),
        stamped(5, Value::List(vec![Value::Tuple(vec![b("corp")])])),
        stamped(
            5,
            Value::List(vec![Value::Tuple(vec![b("x"), Value::Int(3), s("n")])]),
        ),
        Value::List(vec![]),
    ];
    for odd in odd_lists {
        let mut broken = character("X", None);
        let Value::Dict(sections) = &mut broken else {
            unreachable!()
        };
        let Value::Dict(ui) = &mut sections[4].1 else {
            unreachable!()
        };
        ui.push((b("chatchannels"), odd.clone()));
        assert_eq!(
            chat_channels(&broken),
            Err(GroupError::ChatLayout),
            "{odd:?}"
        );

        let good = character("G", Some(&source_channels()));
        let selection = chat(&["player_aaa"]);
        assert_eq!(
            build_selected(&good, &broken, FileKind::Character, &selection),
            Err(GroupError::ChatLayout)
        );
        assert_eq!(
            build_selected(&broken, &good, FileKind::Character, &selection),
            Err(GroupError::ChatLayout)
        );
        // With chat left alone, an odd list is simply carried along.
        assert!(build_selected(&good, &broken, FileKind::Character, &only(&["windows"])).is_ok());
    }
}

#[test]
fn files_with_an_odd_layout_are_refused() {
    let good = account("S", "probescanning.a");
    let two_ui = dict(&[(b("ui"), dict(&[])), (b("ui"), dict(&[]))]);
    let expected = GroupError::Layout(ProtectError::DuplicateSection("ui".into()));
    let selection = only(&["keybinds"]);
    assert_eq!(
        build_selected(&good, &two_ui, FileKind::Account, &selection),
        Err(expected.clone())
    );
    assert_eq!(
        build_selected(&two_ui, &good, FileKind::Account, &selection),
        Err(expected)
    );
    assert!(build_selected(&good, &Value::None, FileKind::Account, &selection).is_err());
}

#[test]
fn inputs_are_not_modified() {
    let source = character("S", Some(&source_channels()));
    let target = character("T", Some(&target_channels()));
    let (source_before, target_before) = (source.clone(), target.clone());
    let selection = Selection {
        groups: chosen(&["windows"]),
        everything_else: true,
        chat_channels: Some(chosen(&["player_aaa"])),
    };
    build_selected(&source, &target, FileKind::Character, &selection).unwrap();
    assert_eq!((source, target), (source_before, target_before));
}

#[test]
fn the_same_settings_in_a_different_order_count_as_the_same() {
    let a = account("S", "probescanning.a");
    let Value::Dict(mut sections) = a.clone() else {
        unreachable!()
    };
    sections.reverse();
    let Value::Dict(ui) = &mut sections[0].1 else {
        unreachable!()
    };
    ui.reverse();
    let reordered = Value::Dict(sections);
    assert_ne!(a, reordered);
    assert!(same_settings(&a, &reordered, FileKind::Account).unwrap());
    assert!(!same_settings(&a, &account("T", "probescanning.a"), FileKind::Account).unwrap());
    assert!(!same_settings(&a, &account("S", "probescanning.z"), FileKind::Account).unwrap());
}

#[test]
fn verification_catches_every_kind_of_wrong_output() {
    let (source, target) = (
        account("S", "probescanning.a"),
        account("T", "probescanning.b"),
    );
    let selection = only(&["keybinds"]);
    let verify = |output: &Value| {
        let data = encode(output, false).unwrap();
        verify_selected(&data, &source, &target, FileKind::Account, &selection)
    };
    let good = build_selected(&source, &target, FileKind::Account, &selection).unwrap();
    assert_eq!(verify(&good), Ok(()));

    let differs = |what: &str, side| Err(GroupError::Differs(what.to_owned(), side));
    // The chosen group wasn't copied.
    assert_eq!(
        verify(&target),
        differs("'Keyboard shortcuts' settings", "source")
    );
    // Far too much was copied: the first thing noticed is the layout.
    assert_eq!(verify(&source), differs("protected settings", "target"));
    // An unchosen group was copied as well.
    let too_much = build_selected(
        &source,
        &target,
        FileKind::Account,
        &only(&["keybinds", "audio"]),
    );
    assert_eq!(
        verify(&too_much.unwrap()),
        differs("'Sound' settings", "target")
    );
    // "Everything else" was copied as well.
    let mut greedy = selection.clone();
    greedy.everything_else = true;
    let too_much = build_selected(&source, &target, FileKind::Account, &greedy).unwrap();
    assert_eq!(verify(&too_much), differs("other settings", "target"));

    // Output that isn't a clean settings file.
    let mut data = encode(&good, false).unwrap();
    data.push(1);
    let check =
        |data: &[u8]| verify_selected(data, &source, &target, FileKind::Account, &selection);
    assert_eq!(check(&data), Err(GroupError::OutputUnreadable));
    assert_eq!(check(&data[..40]), Err(GroupError::OutputUnreadable));
    let list = encode(&Value::List(vec![]), false).unwrap();
    assert_eq!(check(&list), Err(GroupError::OutputUnreadable));
}

#[test]
fn verification_guards_the_chat_channel_list() {
    let source = character("S", Some(&source_channels()));
    let target = character("T", Some(&target_channels()));
    let selection = chat(&["player_aaa"]);
    let verify = |channels: &[Value], selection: &Selection| {
        let output = character("T", Some(channels));
        let data = encode(&output, false).unwrap();
        verify_selected(&data, &source, &target, FileKind::Character, selection)
    };
    let own = target_channels();
    let with = |extra: &[Value]| [&own[..], extra].concat();
    let differs = |side| Err(GroupError::Differs("chat channels".to_owned(), side));

    assert_eq!(
        verify(&with(&[player("player_aaa", "Traders")]), &selection),
        Ok(())
    );
    // The chosen channel never arrived.
    assert_eq!(verify(&own, &selection), differs("source"));
    // One of the target's own channels was lost, or moved.
    assert_eq!(verify(&own[1..], &selection), differs("target"));
    let mut moved = with(&[player("player_aaa", "Traders")]);
    moved.swap(0, 1);
    assert_eq!(verify(&moved, &selection), differs("target"));
    // A channel was added that wasn't chosen, isn't the source's, or is the
    // source's own corp.
    let unchosen = with(&[
        player("player_aaa", "Traders"),
        player("player_zzz", "Other"),
    ]);
    assert_eq!(verify(&unchosen, &selection), differs("target"));
    let renamed = with(&[player("player_aaa", "Not what the source had")]);
    assert_eq!(verify(&renamed, &selection), differs("target"));
    let corp = chat(&["player_aaa", "corp_1"]);
    let stolen = with(&[
        player("player_aaa", "Traders"),
        channel("corp", "corp_1", "Corp"),
    ]);
    assert_eq!(verify(&stolen, &corp), differs("target"));
    // With chat left alone, any addition is wrong.
    assert_eq!(
        verify(&with(&[player("player_aaa", "Traders")]), &only(&[])),
        differs("target")
    );
    assert_eq!(verify(&own, &only(&[])), Ok(()));
}
