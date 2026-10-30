//! Copying only chosen groups of settings.
//!
//! Every setting in a file belongs to exactly one owner: a named group, the
//! chat channel list, "everything else", or the protected data. A selective
//! copy takes the chosen owners from the source and leaves every other owner
//! exactly as the target had it. Protected data always stays the target's.

use std::collections::BTreeSet;

use clone_bay_marshal::{decode, encode, Value};
use thiserror::Error;

use crate::discovery::FileKind;
use crate::protect::{key_is_protected, key_name, section_is_protected, top_level, ProtectError};

/// One piece of a settings file that a group covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A whole top-level section.
    Section(&'static str),
    /// Every key in the `ui` section whose name starts with this.
    UiPrefix(&'static str),
}

/// A named group of settings the user can choose to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub id: &'static str,
    /// What the user sees.
    pub label: &'static str,
    pub parts: &'static [Part],
}

const UI: &str = "ui";
const CHAT_KEY: &str = "chatchannels";
/// Channel kinds that belong to the character itself and are never copied.
const OWN_CHANNEL_KINDS: [&str; 3] = ["corp", "alliance", "local"];

const ACCOUNT_GROUPS: &[Group] = &[
    Group {
        id: "overview",
        label: "Overview",
        parts: &[Part::Section("overview"), Part::Section("defaultoverview")],
    },
    Group {
        id: "keybinds",
        label: "Keyboard shortcuts",
        parts: &[Part::Section("cmd")],
    },
    Group {
        id: "probes",
        label: "Probe scanning",
        parts: &[
            Part::UiPrefix("probescanning."),
            Part::UiPrefix("probeScannerFilters"),
        ],
    },
    Group {
        id: "suppress",
        label: "\"Don't show again\" choices",
        parts: &[Part::Section("suppress")],
    },
    Group {
        id: "audio",
        label: "Sound",
        parts: &[Part::Section("audio")],
    },
    Group {
        id: "camera_graphics",
        label: "Camera and graphics",
        parts: &[
            Part::UiPrefix("camera"),
            Part::UiPrefix("spaceMouse"),
            Part::UiPrefix("offsetUIwithCamera"),
            Part::UiPrefix("invertCameraZoom"),
            Part::UiPrefix("advancedCamera"),
            Part::UiPrefix("missilesEnabled"),
            Part::UiPrefix("turretsEnabled"),
            Part::UiPrefix("trailsEnabled"),
            Part::UiPrefix("effectsEnabled"),
            Part::UiPrefix("explosionEffectsEnabled"),
            Part::UiPrefix("gpuParticlesEnabled"),
            Part::UiPrefix("droneModelsEnabled"),
            Part::UiPrefix("modelSkinsInSpaceEnabled"),
            Part::UiPrefix("UI_ASTEROID_"),
        ],
    },
    Group {
        id: "market",
        label: "Market",
        parts: &[
            Part::UiPrefix("market_"),
            Part::UiPrefix("minEdit_market"),
            Part::UiPrefix("maxEdit_market"),
            Part::UiPrefix("quickbar"),
            Part::UiPrefix("contracts_search_"),
            Part::UiPrefix("mycontracts_"),
            Part::UiPrefix("pricehistorytype"),
        ],
    },
    Group {
        id: "tabgroups",
        label: "Tab groups",
        parts: &[Part::Section("tabgroups")],
    },
];

const CHARACTER_GROUPS: &[Group] = &[
    Group {
        id: "windows",
        label: "Window layout",
        parts: &[Part::Section("windows")],
    },
    Group {
        id: "neocom",
        label: "Neocom",
        parts: &[Part::UiPrefix("neocomButtonRawData")],
    },
    Group {
        id: "infopanels",
        label: "Info panels",
        parts: &[Part::UiPrefix("InfoPanelModes_")],
    },
    Group {
        id: "dockpanels",
        label: "Dock panels",
        parts: &[Part::Section("dockPanels")],
    },
    Group {
        id: "distances",
        label: "Orbit and keep-at-range distances",
        parts: &[
            Part::UiPrefix("defaultTypeOrbitDist"),
            Part::UiPrefix("defaultTypeKeepAtRangeDist"),
        ],
    },
];

/// The groups that exist for a kind of file. Chat channels (characters only)
/// and "everything else" are chosen separately; see [`Selection`].
pub fn groups_for(kind: FileKind) -> &'static [Group] {
    match kind {
        FileKind::Account => ACCOUNT_GROUPS,
        FileKind::Character => CHARACTER_GROUPS,
    }
}

/// Whether this kind of file has a chat channel list.
pub fn has_chat_channels(kind: FileKind) -> bool {
    kind == FileKind::Character
}

/// What the user chose to copy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// IDs of the chosen [`Group`]s.
    pub groups: BTreeSet<String>,
    /// Every setting that belongs to no group (and isn't protected).
    pub everything_else: bool,
    /// IDs of the source's chat channels to add to each target. `None`
    /// leaves chat channels alone.
    pub chat_channels: Option<BTreeSet<String>>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
            && !self.everything_else
            && self.chat_channels.as_ref().is_none_or(BTreeSet::is_empty)
    }
}

/// How much of the source to copy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scope {
    /// All of it (except protected data).
    #[default]
    Everything,
    Selected(Selection),
}

/// Why a file can't be handled exactly, and so is left alone.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GroupError {
    #[error("{0}")]
    Layout(#[from] ProtectError),
    #[error("its chat channel list has an unexpected structure")]
    ChatLayout,
    #[error("self-check failed: output can't be read back")]
    OutputUnreadable,
    #[error("self-check failed: {0} differ from the {1}")]
    Differs(String, &'static str),
}

/// Who a setting belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Owner {
    Protected,
    Group(&'static str),
    Chat,
    Else,
}

/// One setting (or one whole section) with where it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// The section's name, when its key is text.
    section: Option<String>,
    section_key: Value,
    /// The key inside the section; `None` when the item is a whole section.
    key: Option<Value>,
    value: Value,
}

fn section_owner(kind: FileKind, name: &str) -> Owner {
    if section_is_protected(name) {
        return Owner::Protected;
    }
    groups_for(kind)
        .iter()
        .find(|group| {
            let covers = |part: &Part| matches!(part, Part::Section(section) if *section == name);
            group.parts.iter().any(covers)
        })
        .map_or(Owner::Else, |group| Owner::Group(group.id))
}

fn ui_key_owner(kind: FileKind, key: &Value) -> Owner {
    // The chat channel list is protected like the rack layout, but when
    // groups are being chosen it is its own owner, because ticked channels
    // can be added to it. So it is recognised before the protection check.
    if has_chat_channels(kind) && key_name(key).as_deref() == Some(CHAT_KEY) {
        return Owner::Chat;
    }
    if key_is_protected(UI, key) {
        return Owner::Protected;
    }
    let Some(name) = key_name(key) else {
        return Owner::Else;
    };
    let covers = |part: &Part| matches!(part, Part::UiPrefix(prefix) if name.starts_with(prefix));
    groups_for(kind)
        .iter()
        .find(|group| group.parts.iter().any(covers))
        .map_or(Owner::Else, |group| Owner::Group(group.id))
}

/// Every setting in a file with its owner, in file order. The `ui` section
/// is split into its keys; every other section is one item.
fn partition(root: &Value, kind: FileKind) -> Result<Vec<(Owner, Item)>, GroupError> {
    let mut items = Vec::new();
    for (section_key, value) in top_level(root)? {
        let section = key_name(section_key).map(|name| name.into_owned());
        match (section.as_deref(), value) {
            // `top_level` has already refused a `ui` that isn't a dict.
            (Some(UI), Value::Dict(inner)) => {
                for (key, value) in inner {
                    items.push((
                        ui_key_owner(kind, key),
                        Item {
                            section: section.clone(),
                            section_key: section_key.clone(),
                            key: Some(key.clone()),
                            value: value.clone(),
                        },
                    ));
                }
            }
            (name, _) => items.push((
                name.map_or(Owner::Else, |name| section_owner(kind, name)),
                Item {
                    section,
                    section_key: section_key.clone(),
                    key: None,
                    value: value.clone(),
                },
            )),
        }
    }
    Ok(items)
}

/// Whether the chosen copy takes this owner's settings from the source.
/// (Chat channels are combined, not taken, so they are never "from source".)
fn from_source(owner: Owner, selection: &Selection) -> bool {
    match owner {
        Owner::Protected | Owner::Chat => false,
        Owner::Group(id) => selection.groups.contains(id),
        Owner::Else => selection.everything_else,
    }
}

/// Puts items back together as a settings file.
fn assemble(items: Vec<Item>) -> Value {
    let mut sections: Vec<(Option<String>, Value, Value)> = Vec::new();
    for item in items {
        let Some(key) = item.key else {
            sections.push((item.section, item.section_key, item.value));
            continue;
        };
        let existing = sections
            .iter_mut()
            .find_map(|(name, _, value)| match value {
                Value::Dict(inner) if name.is_some() && *name == item.section => Some(inner),
                _ => None,
            });
        match existing {
            Some(inner) => inner.push((key, item.value)),
            None => sections.push((
                item.section,
                item.section_key,
                Value::Dict(vec![(key, item.value)]),
            )),
        }
    }
    Value::Dict(
        sections
            .into_iter()
            .map(|(_, key, value)| (key, value))
            .collect(),
    )
}

/// A chat channel a character has joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// `corp`, `alliance`, `local`, or for player channels the channel ID.
    pub kind: String,
    pub id: String,
    /// The name shown in game.
    pub name: String,
}

impl Channel {
    /// Whether the channel can be added to another character. Corp,
    /// alliance and local belong to the character that is in them.
    pub fn can_be_copied(&self) -> bool {
        !OWN_CHANNEL_KINDS.contains(&self.kind.as_str())
    }
}

fn text(value: &Value) -> Option<String> {
    key_name(value).map(|text| text.into_owned())
}

/// The entries of a `(timestamp, [channel, ...])` setting.
fn chat_entries(value: &Value) -> Result<&[Value], GroupError> {
    match value {
        Value::Tuple(pair) => match pair.as_slice() {
            [Value::Long(_), Value::List(entries) | Value::Tuple(entries)] => Ok(entries),
            _ => Err(GroupError::ChatLayout),
        },
        _ => Err(GroupError::ChatLayout),
    }
}

fn channel(entry: &Value) -> Result<Channel, GroupError> {
    let (Value::Tuple(fields) | Value::List(fields)) = entry else {
        return Err(GroupError::ChatLayout);
    };
    match fields.as_slice() {
        [kind, id, name, ..] => Ok(Channel {
            kind: text(kind).ok_or(GroupError::ChatLayout)?,
            id: text(id).ok_or(GroupError::ChatLayout)?,
            name: text(name).ok_or(GroupError::ChatLayout)?,
        }),
        _ => Err(GroupError::ChatLayout),
    }
}

fn chat_value(root: &Value, kind: FileKind) -> Result<Option<Item>, GroupError> {
    Ok(partition(root, kind)?
        .into_iter()
        .find(|(owner, _)| *owner == Owner::Chat)
        .map(|(_, item)| item))
}

/// The chat channels stored in a character file, in order.
pub fn chat_channels(root: &Value) -> Result<Vec<Channel>, GroupError> {
    match chat_value(root, FileKind::Character)? {
        Some(item) => chat_entries(&item.value)?.iter().map(channel).collect(),
        None => Ok(Vec::new()),
    }
}

/// The target's chat channel setting with the chosen source channels added.
///
/// The target keeps every channel it has, in order. A chosen channel is
/// added at the end unless the target already has it, or it is one of the
/// source character's own (corp, alliance, local).
fn merge_chat(
    source: Option<&Item>,
    target: Option<&Item>,
    chosen: &BTreeSet<String>,
) -> Result<Option<Item>, GroupError> {
    let Some(source) = source else {
        return Ok(target.cloned());
    };
    let mut have: Vec<String> = match target {
        Some(target) => chat_entries(&target.value)?
            .iter()
            .map(|entry| channel(entry).map(|channel| channel.id))
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let mut added = Vec::new();
    for entry in chat_entries(&source.value)? {
        let channel = channel(entry)?;
        if channel.can_be_copied() && chosen.contains(&channel.id) && !have.contains(&channel.id) {
            have.push(channel.id);
            added.push(entry.clone());
        }
    }
    if added.is_empty() {
        return Ok(target.cloned());
    }
    let Some(target) = target else {
        // The target has no list yet: start one with just the added channels.
        let Value::Tuple(pair) = &source.value else {
            return Err(GroupError::ChatLayout);
        };
        let mut item = source.clone();
        item.value = Value::Tuple(vec![pair[0].clone(), Value::List(added)]);
        return Ok(Some(item));
    };
    let Value::Tuple(pair) = &target.value else {
        return Err(GroupError::ChatLayout);
    };
    let (Value::List(entries) | Value::Tuple(entries)) = &pair[1] else {
        return Err(GroupError::ChatLayout);
    };
    let combined: Vec<Value> = entries.iter().cloned().chain(added).collect();
    let combined = match &pair[1] {
        Value::Tuple(_) => Value::Tuple(combined),
        _ => Value::List(combined),
    };
    let mut item = target.clone();
    item.value = Value::Tuple(vec![pair[0].clone(), combined]);
    Ok(Some(item))
}

/// The target's settings with the chosen groups replaced by the source's.
/// Neither input is modified.
pub fn build_selected(
    source: &Value,
    target: &Value,
    kind: FileKind,
    selection: &Selection,
) -> Result<Value, GroupError> {
    let source_items = partition(source, kind)?;
    let target_items = partition(target, kind)?;
    let chat = match &selection.chat_channels {
        Some(chosen) => merge_chat(
            chat_value(source, kind)?.as_ref(),
            chat_value(target, kind)?.as_ref(),
            chosen,
        )?,
        None => chat_value(target, kind)?,
    };

    let kept = target_items
        .into_iter()
        .filter(|(owner, _)| *owner != Owner::Chat && !from_source(*owner, selection));
    let taken = source_items
        .into_iter()
        .filter(|(owner, _)| from_source(*owner, selection));
    let items = kept
        .chain(taken)
        .map(|(_, item)| item)
        .chain(chat)
        .collect();
    Ok(assemble(items))
}

/// One item as sortable bytes: its section's name, its key and its value.
type Canonical = (Option<String>, Vec<u8>, Vec<u8>);

/// An order-independent fingerprint of some items, for comparing two sets.
fn canonical(items: &[&Item]) -> Result<Vec<Canonical>, GroupError> {
    let bytes = |value: &Value| encode(value, false).map_err(|_| GroupError::OutputUnreadable);
    let mut all = Vec::with_capacity(items.len());
    for item in items {
        let key = match &item.key {
            Some(key) => bytes(key)?,
            None => bytes(&item.section_key)?,
        };
        all.push((item.section.clone(), key, bytes(&item.value)?));
    }
    all.sort();
    Ok(all)
}

fn owned_by(items: &[(Owner, Item)], owner: Owner) -> Vec<&Item> {
    items
        .iter()
        .filter(|(item_owner, _)| *item_owner == owner)
        .map(|(_, item)| item)
        .collect()
}

fn describe(owner: Owner, kind: FileKind) -> String {
    match owner {
        Owner::Protected => "protected settings".to_owned(),
        Owner::Chat => "chat channels".to_owned(),
        Owner::Else => "other settings".to_owned(),
        Owner::Group(id) => groups_for(kind)
            .iter()
            .find(|group| group.id == id)
            .map_or_else(
                || id.to_owned(),
                |group| format!("'{}' settings", group.label),
            ),
    }
}

/// Whether two files hold the same settings, whatever order they are in.
pub fn same_settings(a: &Value, b: &Value, kind: FileKind) -> Result<bool, GroupError> {
    let (a, b) = (partition(a, kind)?, partition(b, kind)?);
    let all = |items: &[(Owner, Item)]| {
        let items: Vec<&Item> = items.iter().map(|(_, item)| item).collect();
        canonical(&items)
    };
    Ok(all(&a)? == all(&b)?)
}

/// Decodes what is about to be written and proves, owner by owner, that it
/// holds the source's settings for what was chosen and the target's for
/// everything else.
pub fn verify_selected(
    data: &[u8],
    source: &Value,
    target: &Value,
    kind: FileKind,
    selection: &Selection,
) -> Result<(), GroupError> {
    let output = decode(data).map_err(|_| GroupError::OutputUnreadable)?;
    if output.trailing_bytes != 0 || output.unused_shared_slots != 0 {
        return Err(GroupError::OutputUnreadable);
    }
    let output_items = partition(&output.value, kind).map_err(|_| GroupError::OutputUnreadable)?;
    let source_items = partition(source, kind)?;
    let target_items = partition(target, kind)?;

    let mut owners = vec![Owner::Protected, Owner::Else];
    owners.extend(groups_for(kind).iter().map(|group| Owner::Group(group.id)));
    for owner in owners {
        let (expected, side) = if from_source(owner, selection) {
            (owned_by(&source_items, owner), "source")
        } else {
            (owned_by(&target_items, owner), "target")
        };
        if canonical(&owned_by(&output_items, owner))? != canonical(&expected)? {
            return Err(GroupError::Differs(describe(owner, kind), side));
        }
    }

    // Chat channels: nothing the target had may be lost or reordered, and
    // anything extra must be a chosen, copyable channel from the source.
    let entries = |items: &[(Owner, Item)]| -> Result<Vec<Value>, GroupError> {
        match owned_by(items, Owner::Chat).first() {
            Some(item) => Ok(chat_entries(&item.value)?.to_vec()),
            None => Ok(Vec::new()),
        }
    };
    let (now, before) = (entries(&output_items)?, entries(&target_items)?);
    let chat_differs = || GroupError::Differs(describe(Owner::Chat, kind), "target");
    if now.len() < before.len() || now[..before.len()] != before[..] {
        return Err(chat_differs());
    }
    let extra = &now[before.len()..];
    let chosen = selection.chat_channels.as_ref();
    let source_entries = entries(&source_items)?;
    for entry in extra {
        let added = channel(entry)?;
        let allowed = added.can_be_copied()
            && chosen.is_some_and(|chosen| chosen.contains(&added.id))
            && source_entries.contains(entry);
        if !allowed {
            return Err(chat_differs());
        }
    }
    // And every chosen channel the target lacked must have arrived.
    if let Some(chosen) = chosen {
        let has = |entries: &[Value], id: &str| {
            entries
                .iter()
                .any(|entry| channel(entry).is_ok_and(|channel| channel.id == id))
        };
        for entry in &source_entries {
            let wanted = channel(entry)?;
            if wanted.can_be_copied() && chosen.contains(&wanted.id) && !has(&now, &wanted.id) {
                return Err(GroupError::Differs(describe(Owner::Chat, kind), "source"));
            }
        }
    }
    Ok(())
}

/// How many chat channels `after` has that `before` didn't.
pub fn channels_added(before: &Value, after: &Value) -> usize {
    let count = |root: &Value| chat_channels(root).map_or(0, |channels| channels.len());
    count(after).saturating_sub(count(before))
}
