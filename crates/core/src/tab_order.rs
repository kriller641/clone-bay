//! The order of tabs: overview tabs in an account file, chat tabs in a
//! character file.
//!
//! An account file stores which overview tabs sit in which overview window,
//! in display order, under `overview -> tabsByWindowInstanceID` as a list of
//! lists of tab IDs. The tab that is selected when the game opens is the
//! index under `tabgroups -> overviewTabs`; every overview window shares it.
//!
//! A character file stores, for each window that is part of a stack of tabs,
//! which stack (`windows -> stacksWindows`) and its position there
//! (`windows -> preferredIdxInStack3`, `{stack: {window: position}}`). A chat
//! channel's window is `chatchannel_` followed by the first field of its
//! entry in `chatchannels`. The same window is keyed by a byte string in some
//! of these dicts and by text in others, so keys are matched by their text.
//!
//! Only the order is changed. Saving goes through the same checks, backup
//! and self-check as any other edit.

use std::path::{Path, PathBuf};

use clone_bay_marshal::{Long, Value};
use thiserror::Error;

use crate::discovery::FileKind;
use crate::edit::{open, save_with, stored_setting, EditError, Write};
use crate::platform::EveStatus;
use crate::protect::key_name;
use crate::safe_write::{BackupSession, Fingerprint};

const OVERVIEW: &str = "overview";
const OVERVIEW_WINDOWS: &str = "tabsByWindowInstanceID";
const OVERVIEW_TABS: &str = "tabsettings_new";
const TAB_GROUPS: &str = "tabgroups";
const OVERVIEW_SELECTED: &str = "overviewTabs";
const UI: &str = "ui";
const CHAT_CHANNELS: &str = "chatchannels";
const WINDOWS: &str = "windows";
const STACKS: &str = "stacksWindows";
const POSITIONS: &str = "preferredIdxInStack3";
const CHAT_WINDOW_PREFIX: &str = "chatchannel_";
/// The stack the game puts chat channels in unless the user moves them.
const MAIN_CHAT_STACK: &str = "ChatWindowStack";

/// What a tab is in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabId {
    /// An overview tab's ID.
    Overview(i64),
    /// A window's ID, as text.
    Window(String),
}

/// One tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// What the user sees.
    pub label: String,
    id: TabId,
}

/// The tabs of one window, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabGroup {
    /// What the user sees, for example "Overview window 2".
    pub title: String,
    pub tabs: Vec<Tab>,
    /// For chat tabs, the stack these windows are in.
    stack: Option<String>,
}

/// Every group of tabs in a file whose order can be changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabLayout {
    pub kind: FileKind,
    pub groups: Vec<TabGroup>,
    /// For an account file: the position of the tab that is selected when
    /// the game opens, counting from 0. `None` when the file doesn't say, or
    /// says it in a form this app doesn't understand.
    pub opens_on: Option<i64>,
}

/// Why tab order can't be read or saved.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TabOrderError {
    #[error("the tab order is stored in a form this app doesn't understand")]
    Unreadable,
    #[error(
        "the tabs in the file are no longer the ones shown here. Reopen the file and try again. \
         Nothing was changed"
    )]
    TabsChanged,
    #[error("{0}")]
    Edit(#[from] EditError),
}

impl TabLayout {
    /// Whether a tab can be moved from one group to another. Overview tabs
    /// can; a chat channel stays in the window the game has it in.
    pub fn can_move_between_groups(&self) -> bool {
        self.kind == FileKind::Account
    }

    /// Whether there is any order to change.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Moves a tab one place towards the start of its group. Does nothing
    /// if it can't move.
    pub fn move_up(&mut self, group: usize, tab: usize) {
        if let Some(tabs) = self.groups.get_mut(group).map(|group| &mut group.tabs) {
            if tab > 0 && tab < tabs.len() {
                tabs.swap(tab - 1, tab);
            }
        }
    }

    /// Moves a tab one place towards the end of its group. Does nothing if
    /// it can't move.
    pub fn move_down(&mut self, group: usize, tab: usize) {
        if let Some(tabs) = self.groups.get_mut(group).map(|group| &mut group.tabs) {
            if tab + 1 < tabs.len() {
                tabs.swap(tab, tab + 1);
            }
        }
    }

    /// Whether [`Self::move_to_group`] would move this tab. A window always
    /// keeps at least one tab.
    pub fn can_move_to_group(&self, group: usize, tab: usize, to: usize) -> bool {
        self.can_move_between_groups()
            && group != to
            && to < self.groups.len()
            && self
                .groups
                .get(group)
                .is_some_and(|from| from.tabs.len() > 1 && tab < from.tabs.len())
    }

    /// Moves a tab to the end of another group.
    pub fn move_to_group(&mut self, group: usize, tab: usize, to: usize) {
        if self.can_move_to_group(group, tab, to) {
            let moved = self.groups[group].tabs.remove(tab);
            self.groups[to].tabs.push(moved);
        }
    }

    /// Makes the game open on the first tab.
    pub fn open_on_first_tab(&mut self) {
        if self.opens_on.is_some() {
            self.opens_on = Some(0);
        }
    }
}

/// The value stored under `key` in `section`, after the timestamp.
/// `Ok(None)` when it isn't stored.
fn stored<'a>(
    root: &'a Value,
    section: &str,
    key: &str,
) -> Result<Option<&'a Value>, TabOrderError> {
    stored_setting(root, section, key).map_err(|()| TabOrderError::Unreadable)
}

fn text(value: &Value) -> Option<String> {
    key_name(value).map(|text| text.into_owned())
}

/// A name without the game's formatting tags, for showing.
pub fn plain_label(name: &str) -> String {
    let mut plain = String::new();
    let mut in_tag = false;
    for c in name.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain.trim().to_owned()
}

/// The name of an overview tab, if the file gives it one that can be shown.
fn overview_tab_name(tabs: Option<&Value>, id: i64) -> Option<String> {
    let Some(Value::Dict(tabs)) = tabs else {
        return None;
    };
    let (_, settings) = tabs.iter().find(|(key, _)| *key == Value::Int(id))?;
    let Value::Dict(settings) = settings else {
        return None;
    };
    let (_, name) = settings
        .iter()
        .find(|(key, _)| key_name(key).as_deref() == Some("name"))?;
    let name = plain_label(&text(name)?);
    (!name.is_empty()).then_some(name)
}

fn read_overview(root: &Value) -> Result<TabLayout, TabOrderError> {
    let names = stored(root, OVERVIEW, OVERVIEW_TABS).unwrap_or(None);
    let mut groups = Vec::new();
    match stored(root, OVERVIEW, OVERVIEW_WINDOWS)? {
        None => {}
        Some(Value::List(windows)) => {
            let mut seen = Vec::new();
            for (index, window) in windows.iter().enumerate() {
                let Value::List(ids) = window else {
                    return Err(TabOrderError::Unreadable);
                };
                let mut tabs = Vec::new();
                for id in ids {
                    let Value::Int(id) = id else {
                        return Err(TabOrderError::Unreadable);
                    };
                    // A tab listed twice can't be told apart from itself.
                    if seen.contains(id) {
                        return Err(TabOrderError::Unreadable);
                    }
                    seen.push(*id);
                    tabs.push(Tab {
                        label: overview_tab_name(names, *id).unwrap_or_else(|| format!("Tab {id}")),
                        id: TabId::Overview(*id),
                    });
                }
                groups.push(TabGroup {
                    title: format!("Overview window {}", index + 1),
                    tabs,
                    stack: None,
                });
            }
        }
        Some(_) => return Err(TabOrderError::Unreadable),
    }
    let opens_on = match stored(root, TAB_GROUPS, OVERVIEW_SELECTED) {
        Ok(Some(Value::Int(index))) => Some(*index),
        _ => None,
    };
    Ok(TabLayout {
        kind: FileKind::Account,
        groups,
        opens_on,
    })
}

/// The value held for the one key in `pairs` whose text is `wanted`.
/// `Ok(None)` when no key matches; an error when more than one does, since
/// the game would see them as the same key and keep only one.
fn by_text<'a>(
    pairs: &'a [(Value, Value)],
    wanted: &str,
) -> Result<Option<&'a Value>, TabOrderError> {
    let mut found = pairs
        .iter()
        .filter(|(key, _)| key_name(key).as_deref() == Some(wanted));
    match (found.next(), found.next()) {
        (None, _) => Ok(None),
        (Some((_, value)), None) => Ok(Some(value)),
        _ => Err(TabOrderError::Unreadable),
    }
}

/// A chat tab with the position stored for it, if one is.
type Placed = (Option<i64>, Tab);

fn read_chat(root: &Value) -> Result<TabLayout, TabOrderError> {
    let empty = TabLayout {
        kind: FileKind::Character,
        groups: Vec::new(),
        opens_on: None,
    };
    let channels = match stored(root, UI, CHAT_CHANNELS)? {
        None => return Ok(empty),
        Some(Value::List(channels) | Value::Tuple(channels)) => channels,
        Some(_) => return Err(TabOrderError::Unreadable),
    };
    let stacks = match stored(root, WINDOWS, STACKS)? {
        None => return Ok(empty),
        Some(Value::Dict(stacks)) => stacks,
        Some(_) => return Err(TabOrderError::Unreadable),
    };
    let positions: &[(Value, Value)] = match stored(root, WINDOWS, POSITIONS)? {
        None => &[],
        Some(Value::Dict(positions)) => positions,
        Some(_) => return Err(TabOrderError::Unreadable),
    };

    // Each stack with the channels in it: (position, tab), in list order.
    let mut found: Vec<(String, Vec<Placed>)> = Vec::new();
    for channel in channels {
        let Value::Tuple(fields) = channel else {
            return Err(TabOrderError::Unreadable);
        };
        let [kind, _, name] = fields.as_slice() else {
            return Err(TabOrderError::Unreadable);
        };
        let (Some(kind), Some(name)) = (text(kind), text(name)) else {
            return Err(TabOrderError::Unreadable);
        };
        let window = format!("{CHAT_WINDOW_PREFIX}{kind}");
        // A channel in no stack is a window of its own: nothing to order.
        let stack = match by_text(stacks, &window)? {
            None | Some(Value::None) => continue,
            Some(stack) => text(stack).ok_or(TabOrderError::Unreadable)?,
        };
        let position = match by_text(positions, &stack)? {
            None => None,
            Some(Value::Dict(in_stack)) => match by_text(in_stack, &window)? {
                None => None,
                Some(Value::Int(position)) => Some(*position),
                Some(_) => return Err(TabOrderError::Unreadable),
            },
            Some(_) => return Err(TabOrderError::Unreadable),
        };
        let tab = Tab {
            label: name,
            id: TabId::Window(window),
        };
        match found.iter_mut().find(|(name, _)| *name == stack) {
            Some((_, tabs)) => {
                // The same window twice can't be told apart from itself.
                if tabs.iter().any(|(_, other)| other.id == tab.id) {
                    return Err(TabOrderError::Unreadable);
                }
                tabs.push((position, tab));
            }
            None => found.push((stack, vec![(position, tab)])),
        }
    }
    // The main chat window first, then the others as first met.
    found.sort_by_key(|(stack, _)| stack != MAIN_CHAT_STACK);
    let mut others = 0;
    let groups = found
        .into_iter()
        .map(|(stack, mut tabs)| {
            // Channels with no position, or the same one, keep list order.
            tabs.sort_by_key(|(position, _)| position.unwrap_or(i64::MAX));
            let title = if stack == MAIN_CHAT_STACK {
                "Main chat window".to_owned()
            } else {
                others += 1;
                format!("Other chat window {others}")
            };
            TabGroup {
                title,
                tabs: tabs.into_iter().map(|(_, tab)| tab).collect(),
                stack: Some(stack),
            }
        })
        .collect();
    Ok(TabLayout {
        kind: FileKind::Character,
        groups,
        opens_on: None,
    })
}

/// A copy of an account's settings with the overview set to open on its
/// first tab. `None` when it already does, or the file doesn't store the
/// choice in the form this app understands (in which case it is left alone).
///
/// A copy uses this on its source, so that a source last closed on another
/// tab doesn't make every file copied onto open on that tab. The setting
/// keeps its timestamp.
pub fn opening_on_first_tab(root: &Value) -> Option<Value> {
    match stored(root, TAB_GROUPS, OVERVIEW_SELECTED) {
        Ok(Some(Value::Int(index))) if *index != 0 => {}
        _ => return None,
    }
    // `stored` has checked there is one such section, key and pair.
    let Value::Dict(sections) = root else {
        return None;
    };
    let mut sections = sections.clone();
    let (_, section) = sections
        .iter_mut()
        .find(|(name, _)| key_name(name).as_deref() == Some(TAB_GROUPS))?;
    let Value::Dict(entries) = section else {
        return None;
    };
    let (_, setting) = entries
        .iter_mut()
        .find(|(name, _)| key_name(name).as_deref() == Some(OVERVIEW_SELECTED))?;
    let Value::Tuple(pair) = setting else {
        return None;
    };
    *pair.get_mut(1)? = Value::Int(0);
    Some(Value::Dict(sections))
}

/// Reads the tab order out of a decoded settings file.
pub fn read(root: &Value, kind: FileKind) -> Result<TabLayout, TabOrderError> {
    match kind {
        FileKind::Account => read_overview(root),
        FileKind::Character => read_chat(root),
    }
}

/// Reads the tab order of the settings file at `path`.
pub fn read_file(path: &Path) -> Result<TabLayout, TabOrderError> {
    open_file(path).map(|(layout, _)| layout)
}

/// Reads the tab order of the settings file at `path` for editing, with
/// the fingerprint [`save`] needs.
pub fn open_file(path: &Path) -> Result<(TabLayout, Fingerprint), TabOrderError> {
    let opened = open(path)?;
    Ok((read(&opened.root, opened.kind)?, opened.fingerprint))
}

fn sorted_ids(groups: &[&TabGroup]) -> Vec<String> {
    let mut ids: Vec<String> = groups
        .iter()
        .flat_map(|group| &group.tabs)
        .map(|tab| match &tab.id {
            TabId::Overview(id) => id.to_string(),
            TabId::Window(id) => id.clone(),
        })
        .collect();
    ids.sort();
    ids
}

/// Checks that `wanted` is a reordering of what the file holds: the same
/// groups and the same tabs, with nothing added, lost or repeated.
fn check_same_tabs(held: &TabLayout, wanted: &TabLayout) -> Result<(), TabOrderError> {
    let same_groups = held.kind == wanted.kind
        && held.groups.len() == wanted.groups.len()
        && held
            .groups
            .iter()
            .zip(&wanted.groups)
            .all(|(a, b)| a.stack == b.stack);
    if !same_groups {
        return Err(TabOrderError::TabsChanged);
    }
    let same_tabs = if held.can_move_between_groups() {
        let all = |layout: &TabLayout| sorted_ids(&layout.groups.iter().collect::<Vec<_>>());
        // A window that had tabs must keep at least one.
        let none_emptied = held
            .groups
            .iter()
            .zip(&wanted.groups)
            .all(|(a, b)| a.tabs.is_empty() || !b.tabs.is_empty());
        all(held) == all(wanted) && none_emptied
    } else {
        held.groups
            .iter()
            .zip(&wanted.groups)
            .all(|(a, b)| sorted_ids(&[a]) == sorted_ids(&[b]))
    };
    if same_tabs {
        Ok(())
    } else {
        Err(TabOrderError::TabsChanged)
    }
}

/// The stored positions with every tab in `groups` given its place in its
/// group, counting from 0. Everything else is left as it was.
fn chat_positions(root: &Value, groups: &[TabGroup]) -> Result<Value, TabOrderError> {
    let mut positions = match stored(root, WINDOWS, POSITIONS)? {
        None => Vec::new(),
        Some(Value::Dict(positions)) => positions.clone(),
        Some(_) => return Err(TabOrderError::Unreadable),
    };
    for group in groups {
        let Some(stack) = &group.stack else {
            return Err(TabOrderError::Unreadable);
        };
        // `read` has already refused a file where two keys match.
        let index = positions
            .iter()
            .position(|(key, _)| key_name(key).as_deref() == Some(stack.as_str()));
        let in_stack = match index {
            Some(index) => &mut positions[index].1,
            None => {
                positions.push((Value::Bytes(stack.as_bytes().to_vec()), Value::Dict(vec![])));
                match positions.last_mut() {
                    Some((_, in_stack)) => in_stack,
                    None => return Err(TabOrderError::Unreadable),
                }
            }
        };
        let Value::Dict(in_stack) = in_stack else {
            return Err(TabOrderError::Unreadable);
        };
        for (place, tab) in group.tabs.iter().enumerate() {
            let TabId::Window(window) = &tab.id else {
                return Err(TabOrderError::Unreadable);
            };
            let place = Value::Int(i64::try_from(place).map_err(|_| TabOrderError::Unreadable)?);
            match in_stack
                .iter_mut()
                .find(|(key, _)| key_name(key).as_deref() == Some(window.as_str()))
            {
                Some((_, slot)) => *slot = place,
                None => in_stack.push((Value::Str(window.clone()), place)),
            }
        }
    }
    Ok(Value::Dict(positions))
}

/// What has to be written to give the file the order in `wanted`. Nothing
/// is written for an order that hasn't changed.
fn writes(root: &Value, kind: FileKind, wanted: &TabLayout) -> Result<Vec<Write>, TabOrderError> {
    let held = read(root, kind)?;
    check_same_tabs(&held, wanted)?;
    let mut writes = Vec::new();
    if held.groups != wanted.groups {
        match kind {
            FileKind::Account => {
                let windows = wanted
                    .groups
                    .iter()
                    .map(|group| {
                        group
                            .tabs
                            .iter()
                            .map(|tab| match &tab.id {
                                TabId::Overview(id) => Ok(Value::Int(*id)),
                                TabId::Window(_) => Err(TabOrderError::Unreadable),
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .map(Value::List)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                writes.push((OVERVIEW, OVERVIEW_WINDOWS, Value::List(windows)));
            }
            FileKind::Character => {
                writes.push((WINDOWS, POSITIONS, chat_positions(root, &wanted.groups)?));
            }
        }
    }
    if let (Some(index), true) = (wanted.opens_on, held.opens_on != wanted.opens_on) {
        // Only a file that already stores the choice has it changed.
        if held.opens_on.is_none() || kind != FileKind::Account {
            return Err(TabOrderError::TabsChanged);
        }
        writes.push((TAB_GROUPS, OVERVIEW_SELECTED, Value::Int(index)));
    }
    Ok(writes)
}

/// Whether saving `wanted` would change the file at all.
pub fn differs(root: &Value, kind: FileKind, wanted: &TabLayout) -> Result<bool, TabOrderError> {
    Ok(!writes(root, kind, wanted)?.is_empty())
}

/// Saves a tab order to the settings file at `path`, with the same checks,
/// backup and self-check as any other edit. `opened_as` is the fingerprint
/// [`open_file`] gave; the save is refused if the file has changed since.
/// Returns the folder holding the backup.
pub fn save(
    path: &Path,
    opened_as: Fingerprint,
    wanted: &TabLayout,
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
    now: &Long,
) -> Result<PathBuf, TabOrderError> {
    let mut refused = None;
    let result = save_with(
        path,
        opened_as,
        eve,
        user_confirmed_eve_closed,
        backups,
        now,
        |kind, root| {
            writes(root, kind, wanted).map_err(|error| {
                refused = Some(error);
                EditError::NothingToSave
            })
        },
    );
    match (result, refused) {
        (_, Some(error)) => Err(error),
        (result, None) => Ok(result?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(text: &str) -> Value {
        Value::Bytes(text.as_bytes().to_vec())
    }

    fn s(text: &str) -> Value {
        Value::Str(text.to_owned())
    }

    fn dict(pairs: &[(Value, Value)]) -> Value {
        Value::Dict(pairs.to_vec())
    }

    fn stamped(value: Value) -> Value {
        Value::Tuple(vec![Value::Long(Long::from_i64(7)), value])
    }

    fn ints(ids: &[i64]) -> Value {
        Value::List(ids.iter().map(|id| Value::Int(*id)).collect())
    }

    fn overview_tab(id: i64, name: Value) -> (Value, Value) {
        (Value::Int(id), dict(&[(b("name"), name)]))
    }

    /// An account file with two overview windows.
    fn account(windows: &[&[i64]], selected: Option<Value>) -> Value {
        let mut sections = vec![(
            b("overview"),
            dict(&[
                (
                    b("tabsByWindowInstanceID"),
                    stamped(Value::List(windows.iter().map(|ids| ints(ids)).collect())),
                ),
                (
                    b("tabsettings_new"),
                    stamped(dict(&[
                        overview_tab(0, s(" Combat")),
                        overview_tab(1, b("<color=0xFFFF6666><b>  Travel  </b></color>")),
                        overview_tab(2, s("")),
                    ])),
                ),
            ]),
        )];
        if let Some(selected) = selected {
            sections.push((
                b("tabgroups"),
                dict(&[(b("overviewTabs"), stamped(selected))]),
            ));
        }
        dict(&sections)
    }

    fn labels(layout: &TabLayout) -> Vec<Vec<&str>> {
        layout
            .groups
            .iter()
            .map(|group| group.tabs.iter().map(|tab| tab.label.as_str()).collect())
            .collect()
    }

    fn held<'a>(root: &'a Value, section: &str, key: &str) -> &'a Value {
        stored(root, section, key).unwrap().unwrap()
    }

    fn apply(root: &Value, kind: FileKind, wanted: &TabLayout) -> Value {
        let writes = writes(root, kind, wanted).unwrap();
        crate::edit::write_pairs(root, &writes, &Long::from_i64(9)).unwrap()
    }

    #[test]
    fn overview_tabs_are_read_by_window_with_plain_names() {
        let root = account(&[&[1, 0], &[2, 5]], Some(Value::Int(1)));
        let layout = read(&root, FileKind::Account).unwrap();
        assert_eq!(
            labels(&layout),
            [vec!["Travel", "Combat"], vec!["Tab 2", "Tab 5"]]
        );
        assert_eq!(layout.groups[0].title, "Overview window 1");
        assert_eq!(layout.groups[1].title, "Overview window 2");
        assert_eq!(layout.opens_on, Some(1));
        assert!(layout.can_move_between_groups());
        assert!(!layout.is_empty());
    }

    #[test]
    fn an_account_with_no_overview_windows_stored_has_nothing_to_order() {
        let layout = read(&dict(&[]), FileKind::Account).unwrap();
        assert!(layout.is_empty());
        assert_eq!(layout.opens_on, None);
        assert_eq!(writes(&dict(&[]), FileKind::Account, &layout), Ok(vec![]));
    }

    #[test]
    fn overview_tabs_in_an_unexpected_form_are_refused() {
        let with = |windows: Value| {
            dict(&[(
                b("overview"),
                dict(&[(b("tabsByWindowInstanceID"), stamped(windows))]),
            )])
        };
        for odd in [
            with(Value::Int(1)),
            with(Value::List(vec![Value::Int(1)])),
            with(Value::List(vec![Value::List(vec![s("1")])])),
            // The same tab in two windows.
            with(Value::List(vec![ints(&[1, 2]), ints(&[2])])),
            // Not a (timestamp, value) pair.
            dict(&[(
                b("overview"),
                dict(&[(b("tabsByWindowInstanceID"), ints(&[1]))]),
            )]),
            // Two 'overview' sections.
            dict(&[(b("overview"), dict(&[])), (s("overview"), dict(&[]))]),
            dict(&[(b("overview"), Value::Int(1))]),
            Value::Int(1),
        ] {
            assert_eq!(
                read(&odd, FileKind::Account),
                Err(TabOrderError::Unreadable)
            );
        }
    }

    #[test]
    fn a_selected_tab_in_an_odd_form_is_left_alone() {
        let root = account(&[&[0, 1]], Some(s("first")));
        let mut layout = read(&root, FileKind::Account).unwrap();
        assert_eq!(layout.opens_on, None);
        layout.open_on_first_tab();
        assert_eq!(layout.opens_on, None);
        assert_eq!(writes(&root, FileKind::Account, &layout), Ok(vec![]));
    }

    #[test]
    fn moving_tabs_within_and_between_overview_windows() {
        let root = account(&[&[0, 1, 2], &[5]], Some(Value::Int(0)));
        let mut layout = read(&root, FileKind::Account).unwrap();

        layout.move_up(0, 0); // already first
        layout.move_down(0, 2); // already last
        layout.move_up(9, 0); // no such window
        layout.move_down(9, 0);
        layout.move_up(0, 9); // no such tab
        layout.move_up(0, 3); // just past the last tab
        assert_eq!(layout, read(&root, FileKind::Account).unwrap());

        layout.move_down(0, 0);
        assert_eq!(labels(&layout)[0], ["Travel", "Combat", "Tab 2"]);
        layout.move_up(0, 2);
        assert_eq!(labels(&layout)[0], ["Travel", "Tab 2", "Combat"]);

        // A window keeps its last tab, and a tab can't move to its own or
        // a missing window.
        assert!(!layout.can_move_to_group(1, 0, 0));
        assert!(!layout.can_move_to_group(0, 0, 0));
        assert!(!layout.can_move_to_group(0, 0, 2));
        assert!(!layout.can_move_to_group(0, 3, 1));
        assert!(!layout.can_move_to_group(7, 0, 1));
        layout.move_to_group(1, 0, 0);
        assert_eq!(labels(&layout)[1], ["Tab 5"]);

        assert!(layout.can_move_to_group(0, 0, 1));
        layout.move_to_group(0, 0, 1);
        assert_eq!(
            labels(&layout),
            [vec!["Tab 2", "Combat"], vec!["Tab 5", "Travel"]]
        );

        let saved = apply(&root, FileKind::Account, &layout);
        assert_eq!(
            held(&saved, "overview", "tabsByWindowInstanceID"),
            &Value::List(vec![ints(&[2, 0]), ints(&[5, 1])])
        );
        // The selected tab wasn't touched, so it keeps its timestamp.
        assert_eq!(
            stored(&saved, "tabgroups", "overviewTabs"),
            stored(&root, "tabgroups", "overviewTabs")
        );
        assert_eq!(read(&saved, FileKind::Account).unwrap(), layout);
    }

    #[test]
    fn opening_on_the_first_tab_writes_only_that() {
        let root = account(&[&[0, 1]], Some(Value::Int(1)));
        let mut layout = read(&root, FileKind::Account).unwrap();
        assert!(!differs(&root, FileKind::Account, &layout).unwrap());
        layout.open_on_first_tab();
        assert_eq!(layout.opens_on, Some(0));
        assert!(differs(&root, FileKind::Account, &layout).unwrap());
        assert_eq!(
            writes(&root, FileKind::Account, &layout).unwrap(),
            [("tabgroups", "overviewTabs", Value::Int(0))]
        );
    }

    #[test]
    fn a_copy_source_is_made_to_open_on_its_first_tab() {
        let root = account(&[&[0, 1]], Some(Value::Int(2)));
        let first = opening_on_first_tab(&root).unwrap();
        // Only that one value differs, and it keeps its timestamp.
        assert_eq!(first, account(&[&[0, 1]], Some(Value::Int(0))));
        assert_eq!(read(&first, FileKind::Account).unwrap().opens_on, Some(0));
        // Nothing to do, or nothing this app understands: left alone.
        for untouched in [
            account(&[&[0, 1]], Some(Value::Int(0))),
            account(&[&[0, 1]], None),
            account(&[&[0, 1]], Some(s("second"))),
            dict(&[(b("tabgroups"), dict(&[(b("overviewTabs"), Value::Int(3))]))]),
            dict(&[
                (
                    b("tabgroups"),
                    dict(&[(b("overviewTabs"), stamped(Value::Int(3)))]),
                ),
                (s("tabgroups"), dict(&[])),
            ]),
            dict(&[(b("tabgroups"), Value::Int(1))]),
            Value::Int(1),
        ] {
            assert_eq!(opening_on_first_tab(&untouched), None);
        }
    }

    #[test]
    fn an_order_for_different_tabs_is_refused() {
        let root = account(&[&[0, 1], &[2]], Some(Value::Int(0)));
        let layout = read(&root, FileKind::Account).unwrap();
        let refused = |other: &Value, kind| {
            assert_eq!(
                writes(other, kind, &layout),
                Err(TabOrderError::TabsChanged)
            );
        };
        // A tab was added, removed or swapped for another since.
        refused(&account(&[&[0, 1, 3], &[2]], None), FileKind::Account);
        refused(&account(&[&[0], &[2]], None), FileKind::Account);
        refused(&account(&[&[0, 4], &[2]], None), FileKind::Account);
        // A window was added or removed.
        refused(&account(&[&[0, 1], &[2], &[]], None), FileKind::Account);
        refused(&account(&[&[0, 1, 2]], None), FileKind::Account);
        // The wrong kind of file.
        refused(&character(&[("local", 0)], &[]), FileKind::Character);

        // A window emptied by hand, or a selected tab the file doesn't store.
        let mut emptied = layout.clone();
        let moved = emptied.groups[1].tabs.remove(0);
        emptied.groups[0].tabs.push(moved);
        assert_eq!(
            writes(&root, FileKind::Account, &emptied),
            Err(TabOrderError::TabsChanged)
        );
        let mut invented = read(&account(&[&[0, 1], &[2]], None), FileKind::Account).unwrap();
        invented.opens_on = Some(0);
        assert_eq!(
            writes(
                &account(&[&[0, 1], &[2]], None),
                FileKind::Account,
                &invented
            ),
            Err(TabOrderError::TabsChanged)
        );
        // The same tab twice in place of another.
        let mut doubled = layout.clone();
        doubled.groups[0].tabs[1] = doubled.groups[0].tabs[0].clone();
        assert_eq!(
            writes(&root, FileKind::Account, &doubled),
            Err(TabOrderError::TabsChanged)
        );
    }

    fn channel(kind: &str, name: &str) -> Value {
        let kind_value = if kind.starts_with("player_") {
            s(kind)
        } else {
            b(kind)
        };
        Value::Tuple(vec![kind_value, s(&format!("{kind}_1")), s(name)])
    }

    /// A character file whose channels are all in the main chat window at
    /// the given positions, plus whatever else `windows` holds.
    fn character(channels: &[(&str, i64)], extra_windows: &[(Value, Value)]) -> Value {
        let list = channels
            .iter()
            .map(|(kind, _)| channel(kind, &kind.to_uppercase()))
            .collect();
        let stacks: Vec<(Value, Value)> = channels
            .iter()
            .map(|(kind, _)| (s(&format!("chatchannel_{kind}")), b("ChatWindowStack")))
            .collect();
        // Positions are keyed by a byte string where stacks used text.
        let positions: Vec<(Value, Value)> = channels
            .iter()
            .map(|(kind, position)| (b(&format!("chatchannel_{kind}")), Value::Int(*position)))
            .chain([(b("chatchannel_long_gone"), Value::Int(0))])
            .collect();
        let mut windows = vec![
            (b("stacksWindows"), stamped(Value::Dict(stacks))),
            (
                b("preferredIdxInStack3"),
                stamped(dict(&[
                    (b("77"), dict(&[(b("chatchannel_local"), Value::Int(4))])),
                    (b("ChatWindowStack"), Value::Dict(positions)),
                ])),
            ),
        ];
        windows.extend_from_slice(extra_windows);
        dict(&[
            (
                b("ui"),
                dict(&[(b("chatchannels"), stamped(Value::List(list)))]),
            ),
            (b("windows"), Value::Dict(windows)),
        ])
    }

    #[test]
    fn chat_tabs_are_read_in_position_order_with_ties_in_list_order() {
        let root = character(
            &[("corp", 2), ("alliance", 1), ("local", 0), ("player_ab", 2)],
            &[],
        );
        let layout = read(&root, FileKind::Character).unwrap();
        assert_eq!(
            labels(&layout),
            [["LOCAL", "ALLIANCE", "CORP", "PLAYER_AB"]]
        );
        assert_eq!(layout.groups[0].title, "Main chat window");
        assert_eq!(layout.opens_on, None);
        assert!(!layout.can_move_between_groups());
        assert!(!layout.can_move_to_group(0, 0, 0));
    }

    #[test]
    fn reordering_chat_tabs_gives_each_its_own_position_and_keeps_the_rest() {
        let root = character(&[("corp", 2), ("local", 0), ("player_ab", 2)], &[]);
        let mut layout = read(&root, FileKind::Character).unwrap();
        assert_eq!(writes(&root, FileKind::Character, &layout), Ok(vec![]));
        assert!(!differs(&root, FileKind::Character, &layout).unwrap());

        layout.move_down(0, 0);
        assert_eq!(labels(&layout), [["CORP", "LOCAL", "PLAYER_AB"]]);
        let saved = apply(&root, FileKind::Character, &layout);
        assert_eq!(
            held(&saved, "windows", "preferredIdxInStack3"),
            &dict(&[
                // Another stack's record of the same window is left alone.
                (b("77"), dict(&[(b("chatchannel_local"), Value::Int(4))])),
                (
                    b("ChatWindowStack"),
                    dict(&[
                        (b("chatchannel_corp"), Value::Int(0)),
                        (b("chatchannel_local"), Value::Int(1)),
                        (b("chatchannel_player_ab"), Value::Int(2)),
                        // A channel no longer joined keeps what it had.
                        (b("chatchannel_long_gone"), Value::Int(0)),
                    ])
                ),
            ])
        );
        assert_eq!(
            stored(&saved, "windows", "stacksWindows"),
            stored(&root, "windows", "stacksWindows")
        );
        assert_eq!(read(&saved, FileKind::Character).unwrap(), layout);
        assert!(!differs(&saved, FileKind::Character, &layout).unwrap());
    }

    #[test]
    fn channels_are_grouped_by_the_window_they_are_in() {
        let list = Value::List(vec![
            channel("player_a", "Side one"),
            channel("local", "Local"),
            channel("fleet", "Fleet"),
            channel("player_b", "Side two"),
            channel("corp", "Corp"),
            channel("alliance", "Alliance"),
        ]);
        let root = dict(&[
            (b("ui"), dict(&[(b("chatchannels"), stamped(list))])),
            (
                b("windows"),
                dict(&[
                    (
                        b("stacksWindows"),
                        stamped(dict(&[
                            (s("chatchannel_player_a"), b("12")),
                            (b("chatchannel_local"), s("ChatWindowStack")),
                            (s("chatchannel_player_b"), b("12")),
                            (b("chatchannel_corp"), b("ChatWindowStack")),
                            // A window of its own, and one not recorded.
                            (s("chatchannel_fleet"), Value::None),
                        ])),
                    ),
                    (
                        b("preferredIdxInStack3"),
                        stamped(dict(&[(
                            b("ChatWindowStack"),
                            dict(&[(s("chatchannel_corp"), Value::Int(0))]),
                        )])),
                    ),
                ]),
            ),
        ]);
        let mut layout = read(&root, FileKind::Character).unwrap();
        assert_eq!(
            labels(&layout),
            [vec!["Corp", "Local"], vec!["Side one", "Side two"]]
        );
        assert_eq!(layout.groups[1].title, "Other chat window 1");

        // A stack with no positions stored gets them when it is reordered.
        layout.move_down(1, 0);
        let saved = apply(&root, FileKind::Character, &layout);
        assert_eq!(
            held(&saved, "windows", "preferredIdxInStack3"),
            &dict(&[
                (
                    b("ChatWindowStack"),
                    dict(&[
                        (s("chatchannel_corp"), Value::Int(0)),
                        (s("chatchannel_local"), Value::Int(1)),
                    ])
                ),
                (
                    b("12"),
                    dict(&[
                        (s("chatchannel_player_b"), Value::Int(0)),
                        (s("chatchannel_player_a"), Value::Int(1)),
                    ])
                ),
            ])
        );
        assert_eq!(read(&saved, FileKind::Character).unwrap(), layout);
    }

    #[test]
    fn a_character_with_no_channels_or_windows_stored_has_nothing_to_order() {
        assert!(read(&dict(&[]), FileKind::Character).unwrap().is_empty());
        let no_windows = dict(&[(
            b("ui"),
            dict(&[(
                b("chatchannels"),
                stamped(Value::List(vec![channel("local", "Local")])),
            )]),
        )]);
        assert!(read(&no_windows, FileKind::Character).unwrap().is_empty());
        // Channels in a tuple, and no positions stored at all.
        let root = dict(&[
            (
                b("ui"),
                dict(&[(
                    b("chatchannels"),
                    stamped(Value::Tuple(vec![
                        channel("local", "Local"),
                        channel("corp", "Corp"),
                    ])),
                )]),
            ),
            (
                b("windows"),
                dict(&[(
                    b("stacksWindows"),
                    stamped(dict(&[
                        (b("chatchannel_local"), b("ChatWindowStack")),
                        (b("chatchannel_corp"), b("ChatWindowStack")),
                    ])),
                )]),
            ),
        ]);
        let mut layout = read(&root, FileKind::Character).unwrap();
        assert_eq!(labels(&layout), [["Local", "Corp"]]);
        layout.move_up(0, 1);
        let saved = apply(&root, FileKind::Character, &layout);
        assert_eq!(
            held(&saved, "windows", "preferredIdxInStack3"),
            &dict(&[(
                b("ChatWindowStack"),
                dict(&[
                    (s("chatchannel_corp"), Value::Int(0)),
                    (s("chatchannel_local"), Value::Int(1)),
                ])
            )])
        );
    }

    #[test]
    fn chat_tabs_in_an_unexpected_form_are_refused() {
        let good = character(&[("corp", 0), ("local", 1)], &[]);
        assert!(read(&good, FileKind::Character).is_ok());
        let replace = |section: &str, key: &str, value: Value| {
            let Value::Dict(sections) = &good else {
                unreachable!()
            };
            Value::Dict(
                sections
                    .iter()
                    .map(|(name, entries)| {
                        let Value::Dict(entries) = entries else {
                            unreachable!()
                        };
                        let entries = entries
                            .iter()
                            .map(|(k, v)| {
                                if *name == b(section) && *k == b(key) {
                                    (k.clone(), value.clone())
                                } else {
                                    (k.clone(), v.clone())
                                }
                            })
                            .collect();
                        (name.clone(), Value::Dict(entries))
                    })
                    .collect(),
            )
        };
        let stack = |window: &str| (s(window), b("ChatWindowStack"));
        for odd in [
            replace("ui", "chatchannels", stamped(Value::Int(1))),
            replace(
                "ui",
                "chatchannels",
                stamped(Value::List(vec![Value::Int(1)])),
            ),
            replace(
                "ui",
                "chatchannels",
                stamped(Value::List(vec![Value::Tuple(vec![b("corp")])])),
            ),
            replace(
                "ui",
                "chatchannels",
                stamped(Value::List(vec![Value::Tuple(vec![
                    Value::Int(1),
                    s("x"),
                    s("y"),
                ])])),
            ),
            // The same channel twice.
            replace(
                "ui",
                "chatchannels",
                stamped(Value::List(vec![
                    channel("corp", "Corp"),
                    channel("corp", "Corp"),
                ])),
            ),
            replace("windows", "stacksWindows", stamped(Value::Int(1))),
            // One window under two keys the game would see as one.
            replace(
                "windows",
                "stacksWindows",
                stamped(dict(&[
                    stack("chatchannel_corp"),
                    (b("chatchannel_corp"), b("12")),
                ])),
            ),
            replace(
                "windows",
                "stacksWindows",
                stamped(dict(&[(s("chatchannel_corp"), Value::Int(3))])),
            ),
            replace("windows", "preferredIdxInStack3", stamped(Value::Int(1))),
            replace(
                "windows",
                "preferredIdxInStack3",
                stamped(dict(&[(b("ChatWindowStack"), Value::Int(1))])),
            ),
            replace(
                "windows",
                "preferredIdxInStack3",
                stamped(dict(&[(
                    b("ChatWindowStack"),
                    dict(&[(b("chatchannel_corp"), s("first"))]),
                )])),
            ),
            replace(
                "windows",
                "preferredIdxInStack3",
                stamped(dict(&[
                    (b("ChatWindowStack"), dict(&[])),
                    (s("ChatWindowStack"), dict(&[])),
                ])),
            ),
        ] {
            assert_eq!(
                read(&odd, FileKind::Character),
                Err(TabOrderError::Unreadable),
                "{odd:?}"
            );
        }
    }

    #[test]
    fn a_chat_order_for_other_channels_is_refused() {
        let root = character(&[("corp", 0), ("local", 1)], &[]);
        let layout = read(&root, FileKind::Character).unwrap();
        for other in [
            character(&[("corp", 0), ("alliance", 1)], &[]),
            character(&[("corp", 0)], &[]),
            character(&[("corp", 0), ("local", 1), ("alliance", 2)], &[]),
            dict(&[]),
        ] {
            assert_eq!(
                writes(&other, FileKind::Character, &layout),
                Err(TabOrderError::TabsChanged)
            );
        }
        assert_eq!(
            writes(&account(&[&[0]], None), FileKind::Account, &layout),
            Err(TabOrderError::TabsChanged)
        );
    }

    #[test]
    fn errors_are_worded_for_the_user() {
        assert_eq!(
            TabOrderError::Unreadable.to_string(),
            "the tab order is stored in a form this app doesn't understand"
        );
        assert!(TabOrderError::TabsChanged
            .to_string()
            .ends_with("Nothing was changed"));
        assert_eq!(
            TabOrderError::from(EditError::NothingToSave).to_string(),
            "there is nothing to save"
        );
    }

    #[test]
    fn labels_lose_their_formatting_tags() {
        assert_eq!(plain_label("<b>  Combat </b>"), "Combat");
        assert_eq!(plain_label("a > b"), "a > b");
        assert_eq!(plain_label("<color=0xFF>"), "");
        assert_eq!(plain_label("x <unclosed"), "x");
    }
}
