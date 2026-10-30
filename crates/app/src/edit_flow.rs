//! Editing the individual settings of one file: what is shown, what has
//! been changed, and saving. No drawing happens here.

use clone_bay_core::bundled::{bundled_type_name, scan_group, scan_groups};
use clone_bay_core::discovery::SettingsFileEntry;
use clone_bay_core::edit::{
    open, options, read, save, settings_for, timestamp_now, Change, Content, EditError, Kind,
    Reading, ScanFilter, Setting,
};
use clone_bay_core::names::NameBook;
use clone_bay_core::safe_write::Fingerprint;
use clone_bay_core::tab_order::plain_label;

use crate::saving::Saving;
use crate::services::Services;
use crate::state::Wake;

/// One setting on the edit screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub setting: &'static Setting,
    /// What the file held when it was opened or last saved.
    pub original: Reading,
    /// The value shown in the control. `None` when the setting can't be
    /// edited because it is stored in a form the app doesn't understand.
    pub draft: Option<Content>,
    /// Whether the user has changed the control. For a setting the file
    /// doesn't store, this is what decides whether it gets written.
    pub touched: bool,
    /// For a distance list: the ship name being typed into "Add ship".
    pub add_input: String,
    /// For a distance list: why the last "Add ship" did nothing.
    pub add_note: Option<String>,
    /// For a distance list: whether "Clear all" is waiting for a yes or no.
    pub confirm_clear: bool,
    /// For a setting chosen from what the file holds (an overview preset):
    /// the names it may be set to.
    pub options: Vec<String>,
    /// For the scan filters: the filter whose kinds of result are being
    /// chosen, by ID.
    pub open_filter: Option<i64>,
}

impl Row {
    fn new(setting: &'static Setting, original: Reading, options: Vec<String>) -> Self {
        let draft = match &original {
            Reading::Stored(content) => Some(content.clone()),
            Reading::NotStored => Some(starting_value(setting.kind)),
            Reading::Unreadable => None,
        };
        Self {
            setting,
            original,
            draft,
            touched: false,
            add_input: String::new(),
            add_note: None,
            confirm_clear: false,
            options,
            open_filter: None,
        }
    }

    /// Adds a scan filter with nothing ticked and opens it, so the user
    /// can name it and choose what it shows. Its ID is above every ID the
    /// file holds or the list has, so a removed filter's ID is never used
    /// again.
    pub fn add_filter(&mut self) {
        let held = match &self.original {
            Reading::Stored(Content::Filters(filters)) => filters.as_slice(),
            _ => &[],
        };
        let Some(Content::Filters(filters)) = &mut self.draft else {
            return;
        };
        let highest = held
            .iter()
            .chain(filters.iter())
            .map(|filter| filter.id)
            .max()
            .unwrap_or(0);
        let id = highest.max(0) + 1;
        filters.push(ScanFilter {
            id,
            name: NEW_FILTER_NAME.to_owned(),
            groups: Vec::new(),
        });
        self.open_filter = Some(id);
        self.touched = true;
    }

    /// Shows the kinds of result of one filter, or hides them if they are
    /// showing. One filter is open at a time.
    pub fn toggle_filter(&mut self, id: i64) {
        self.open_filter = if self.open_filter == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    /// The value to save, if this setting has been changed.
    pub fn change(&self) -> Option<Change> {
        let draft = self.draft.as_ref()?;
        let changed = match &self.original {
            Reading::Stored(content) => draft != content,
            Reading::NotStored => self.touched,
            Reading::Unreadable => false,
        };
        changed.then(|| Change {
            setting: self.setting.id,
            content: draft.clone(),
        })
    }
}

/// What a control shows for a setting the file doesn't store. It is only a
/// starting point for the user; it is not written unless they change it.
fn starting_value(kind: Kind) -> Content {
    match kind {
        Kind::Bool | Kind::Flag => Content::Switch(false),
        Kind::Whole { min, .. } => Content::Whole(min),
        Kind::Choice(options) => Content::Whole(options.first().map_or(0, |(value, _)| *value)),
        Kind::Number { min, .. } => Content::Number(min),
        Kind::Distances { .. } => Content::Distances(Vec::new()),
        Kind::Word(options) => {
            Content::Word(options.first().map_or("", |(word, _)| *word).to_owned())
        }
        Kind::Systems => Content::Systems(Vec::new()),
        Kind::Scaled { min, .. } => Content::Number(min),
        Kind::Preset => Content::Word(String::new()),
        Kind::ScanFilters => Content::Filters(Vec::new()),
    }
}

/// What a newly added scan filter is called until the user names it.
pub const NEW_FILTER_NAME: &str = "New filter";

/// Ticks or unticks one kind of result in a filter. A kind that is ticked
/// goes to the end; one already there is not added twice.
pub fn set_group(filter: &mut ScanFilter, group: i64, on: bool) {
    if on {
        if !filter.groups.contains(&group) {
            filter.groups.push(group);
        }
    } else {
        filter.groups.retain(|other| *other != group);
    }
}

/// One kind of result offered for a filter: its group ID, its name and
/// whether the filter holds it.
pub type GroupChoice = (i64, String, bool);

/// The heading for kinds of result the app has no category for.
pub const OTHER_GROUPS: &str = "Other";

/// The kinds of result to show for a filter, by category: every kind the
/// app offers, plus any the filter already holds that it doesn't (shown by
/// name when the app knows it, and under "Other" by number when not).
pub fn group_choices(filter: &ScanFilter) -> Vec<(&'static str, Vec<GroupChoice>)> {
    let mut choices: Vec<(&'static str, Vec<GroupChoice>)> = Vec::new();
    for group in scan_groups() {
        let ticked = filter.groups.contains(&group.id);
        if !group.published && !ticked {
            continue;
        }
        let choice = (group.id, group.name.to_owned(), ticked);
        match choices.last_mut() {
            Some((category, list)) if *category == group.category => list.push(choice),
            _ => choices.push((group.category, vec![choice])),
        }
    }
    let unknown: Vec<GroupChoice> = filter
        .groups
        .iter()
        .filter(|id| scan_group(**id).is_none())
        .map(|id| (*id, format!("Group {id}"), true))
        .collect();
    if !unknown.is_empty() {
        choices.push((OTHER_GROUPS, unknown));
    }
    choices
}

/// The distance a newly added ship starts with, in metres.
pub const NEW_DISTANCE: i64 = 500;

pub struct EditFlow {
    pub entry: SettingsFileEntry,
    /// The character's name or the account's label, when there is one.
    pub name: Option<String>,
    pub rows: Vec<Row>,
    /// What the file held when it was opened or last saved. A save is
    /// refused once it holds anything else.
    opened_as: Option<Fingerprint>,
    /// Why the file couldn't be opened, or the last save was refused.
    pub problem: Option<String>,
    pub saving: Saving<EditError>,
}

impl EditFlow {
    /// Opens `entry` for editing. If it can't be read there are no rows and
    /// `problem` says why.
    pub fn new(
        entry: &SettingsFileEntry,
        name: Option<String>,
        services: Services,
        wake: Wake,
    ) -> Self {
        let mut flow = Self {
            entry: entry.clone(),
            name,
            rows: Vec::new(),
            opened_as: None,
            problem: None,
            saving: Saving::new(services, wake),
        };
        flow.reload();
        flow
    }

    /// Reads the file again, dropping any unsaved changes.
    fn reload(&mut self) {
        match open(&self.entry.path) {
            Ok(opened) => {
                let root = &opened.root;
                self.rows = settings_for(opened.kind)
                    .iter()
                    .map(|setting| Row::new(setting, read(root, setting), options(root, setting)))
                    .collect();
                self.opened_as = Some(opened.fingerprint);
            }
            Err(error) => {
                self.rows.clear();
                self.opened_as = None;
                self.problem = Some(error.to_string());
            }
        }
    }

    /// Everything the user has changed and not yet saved.
    pub fn changes(&self) -> Vec<Change> {
        self.rows.iter().filter_map(Row::change).collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.rows.iter().any(|row| row.change().is_some())
    }

    /// Whether a save is running, during which the screen can't be left.
    pub fn busy(&self) -> bool {
        self.saving.busy()
    }

    /// Whether "Save changes" may be pressed.
    pub fn can_save(&self) -> bool {
        self.is_dirty() && self.saving.ready()
    }

    /// Whether a save has changed the file since the screen was opened.
    pub fn changed_file(&self) -> bool {
        self.saving.changed_file()
    }

    /// Puts every control back to what the file holds.
    pub fn discard(&mut self) {
        if !self.busy() {
            self.problem = None;
            self.reload();
        }
    }

    /// Every ID on the screen that is shown by name: the types in the
    /// distance lists and the solar systems in the avoid list. Their IDs
    /// never overlap, so they are looked up together.
    pub fn type_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .rows
            .iter()
            .flat_map(|row| match &row.draft {
                Some(Content::Distances(entries)) => {
                    entries.iter().map(|(type_id, _)| *type_id).collect()
                }
                Some(Content::Systems(systems)) => systems.iter().map(|system| system.id).collect(),
                _ => Vec::new(),
            })
            .filter_map(|id| u64::try_from(id).ok())
            // Type 0 is the default for everything, not a real type.
            .filter(|id| *id != 0)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Saves the changes in the background.
    pub fn save(&mut self) {
        if !self.can_save() {
            return;
        }
        let Some(opened_as) = self.opened_as else {
            return;
        };
        let changes = self.changes();
        let path = self.entry.path.clone();
        self.problem = None;
        self.saving.start(move |context| {
            save(
                &path,
                opened_as,
                &changes,
                context.eve,
                context.user_confirmed_eve_closed,
                context.backups,
                &timestamp_now(),
            )
        });
    }

    /// Adds a ship (or structure) to a distance list at [`NEW_DISTANCE`],
    /// for the user to change. A type already in the list is not added
    /// twice.
    pub fn add_type(&mut self, row: usize, type_id: u64, name: &str) {
        let Some(target) = self.rows.get_mut(row) else {
            return;
        };
        let Some(Content::Distances(entries)) = &mut target.draft else {
            return;
        };
        let Ok(type_id) = i64::try_from(type_id) else {
            return;
        };
        if entries.iter().any(|(existing, _)| *existing == type_id) {
            target.add_note = Some(format!("{name} is already in the list."));
            return;
        }
        entries.push((type_id, NEW_DISTANCE));
        target.touched = true;
        target.add_input.clear();
        target.add_note = None;
    }

    /// Applies the result of a save, if it has finished.
    pub fn poll(&mut self) {
        match self.saving.poll() {
            // Show what the file holds now.
            Some(Ok(())) => self.reload(),
            Some(Err(error)) => self.problem = Some(error.to_string()),
            None => {}
        }
    }
}

/// The name to show for a type in a distance list.
pub fn type_label(names: &NameBook, type_id: i64) -> String {
    if type_id == 0 {
        return "Everything else (default)".to_owned();
    }
    match type_name(names, type_id) {
        Some(name) => name.to_owned(),
        None => format!("Type {type_id}"),
    }
}

/// A type's name: from the list built into the app, or else one looked up
/// earlier.
fn type_name(names: &NameBook, type_id: i64) -> Option<&str> {
    let id = u64::try_from(type_id).ok()?;
    bundled_type_name(id).or_else(|| names.item_type(id))
}

/// The name to show for a solar system in the avoid list.
pub fn system_label(names: &NameBook, system_id: i64) -> String {
    let name = u64::try_from(system_id)
        .ok()
        .and_then(|id| names.item_type(id));
    match name {
        Some(name) => name.to_owned(),
        None => format!("System {system_id}"),
    }
}

/// A distance in metres, as a player would say it.
pub fn distance_label(metres: i64) -> String {
    if metres >= 1000 && metres % 100 == 0 {
        let tenths = metres / 100;
        if tenths % 10 == 0 {
            format!("{} km", tenths / 10)
        } else {
            format!("{}.{} km", tenths / 10, tenths % 10)
        }
    } else {
        format!("{metres} m")
    }
}

/// What a preset is called on screen: its name without the game's
/// formatting tags, or the name as stored when nothing else is left.
pub fn preset_label(name: &str) -> String {
    let plain = plain_label(name);
    if plain.is_empty() {
        name.to_owned()
    } else {
        plain
    }
}

/// The presets to offer, as `(label, stored name)`, in alphabetical order
/// of what is shown.
pub fn preset_choices(options: &[String]) -> Vec<(String, &str)> {
    let mut choices: Vec<(String, &str)> = options
        .iter()
        .map(|name| (preset_label(name), name.as_str()))
        .collect();
    choices.sort_by_key(|(label, name)| (label.to_lowercase(), (*name).to_owned()));
    choices
}

/// Reads a distance typed by the user: a whole number of metres between
/// `min` and `max`. Spaces and commas between digits are allowed, as is a
/// trailing "m". Anything else is `None`.
pub fn parse_distance(text: &str, min: i64, max: i64) -> Option<i64> {
    let text = text.trim();
    let text = text.strip_suffix(['m', 'M']).unwrap_or(text);
    let digits: String = text.chars().filter(|c| !matches!(c, ' ' | ',')).collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits
        .parse()
        .ok()
        .filter(|metres| (min..=max).contains(metres))
}

/// Reads a whole number typed by the user, between `min` and `max`. It may
/// have a leading minus. Anything else is `None`.
pub fn parse_whole(text: &str, min: i64, max: i64) -> Option<i64> {
    let text = text.trim();
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|n| (min..=max).contains(n))
}

/// Reads a number typed by the user, between `min` and `max`: digits with
/// at most one leading minus and one decimal point. No exponents, "inf" or
/// "nan". Anything else is `None`.
pub fn parse_decimal(text: &str, min: f64, max: f64) -> Option<f64> {
    let text = text.trim();
    let sensible = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.'));
    if !sensible {
        return None;
    }
    text.parse()
        .ok()
        .filter(|x: &f64| x.is_finite() && (min..=max).contains(x))
}

/// A number setting as shown in its box: up to three decimal places,
/// without trailing zeros.
pub fn format_decimal(x: f64) -> String {
    let text = format!("{x:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        other => other.to_owned(),
    }
}

/// The entry of a distance list that holds the default for everything
/// without an entry of its own.
const DEFAULT_TYPE: i64 = 0;

/// Whether a distance list's entry for `type_id` may be removed: every one
/// but the default, which always stays.
pub fn is_removable(type_id: i64) -> bool {
    type_id != DEFAULT_TYPE
}

/// How many entries of a distance list can be removed.
pub fn removable(entries: &[(i64, i64)]) -> usize {
    entries
        .iter()
        .filter(|(type_id, _)| is_removable(*type_id))
        .count()
}

/// Takes every ship out of a distance list, leaving the default (if the
/// list has one) as it was.
pub fn clear_ships(entries: &mut Vec<(i64, i64)>) {
    entries.retain(|(type_id, _)| !is_removable(*type_id));
}

/// The order to show a distance list in: the default for everything else
/// first, then the types whose names are known in alphabetical order
/// whatever the capitals, then the rest by number. The stored order is left
/// alone; only the display is sorted.
pub fn distance_order(entries: &[(i64, i64)], names: &NameBook) -> Vec<usize> {
    let key = |type_id: i64| match (type_id, type_name(names, type_id)) {
        (0, _) => (0, String::new(), 0),
        (_, Some(name)) => (1, name.to_lowercase(), type_id),
        (_, None) => (2, String::new(), type_id),
    };
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&index| key(entries[index].0));
    order
}

#[cfg(test)]
mod tests {
    use clone_bay_core::names::CharacterName;

    use super::*;

    // Written with the typing-only distance box and not yet run when committed.
    #[test]
    fn typed_distances_must_be_whole_metres_within_the_limits() {
        assert_eq!(parse_distance("500", 500, 1_000_000), Some(500));
        assert_eq!(parse_distance(" 7500 ", 500, 1_000_000), Some(7_500));
        assert_eq!(parse_distance("1,000,000", 500, 1_000_000), Some(1_000_000));
        assert_eq!(parse_distance("25 000 m", 500, 1_000_000), Some(25_000));
        assert_eq!(parse_distance("0", 0, 100_000), Some(0));
        // Outside the limits.
        assert_eq!(parse_distance("499", 500, 1_000_000), None);
        assert_eq!(parse_distance("1000001", 500, 1_000_000), None);
        assert_eq!(parse_distance("100001", 0, 100_000), None);
        // Half-typed or not a whole number of metres.
        for text in [
            "", " ", "m", "abc", "12a", "-600", "7.5", "7500.0", "5km", "1e4",
        ] {
            assert_eq!(parse_distance(text, 0, 1_000_000), None, "{text:?}");
        }
        // Too many digits to be a number at all.
        assert_eq!(parse_distance("99999999999999999999999", 0, i64::MAX), None);
    }

    #[test]
    fn distances_read_naturally() {
        assert_eq!(distance_label(0), "0 m");
        assert_eq!(distance_label(500), "500 m");
        assert_eq!(distance_label(999), "999 m");
        assert_eq!(distance_label(1000), "1 km");
        // A sign is not part of a distance.
        assert_eq!(parse_distance("+5000", 0, 100_000), None);
        assert_eq!(parse_distance("-5000", 0, 100_000), None);
        assert_eq!(distance_label(1050), "1050 m");
        assert_eq!(distance_label(7500), "7.5 km");
        assert_eq!(distance_label(80_000), "80 km");
        assert_eq!(distance_label(200_000), "200 km");
    }

    #[test]
    fn plain_numbers_are_read_strictly_and_shown_tidily() {
        assert_eq!(parse_whole("3", 0, 5), Some(3));
        assert_eq!(parse_whole(" 5 ", 0, 5), Some(5));
        assert_eq!(parse_whole("0", 0, 5), Some(0));
        assert_eq!(parse_whole("-2", -5, 5), Some(-2));
        for text in ["", "-", "6", "-1", "1.5", "3m", "+3", "1e1", "--2", "2-"] {
            assert_eq!(parse_whole(text, 0, 5), None, "{text:?}");
        }

        assert_eq!(parse_decimal("0.75", 0.0, 1.0), Some(0.75));
        assert_eq!(parse_decimal(" 1 ", 0.0, 1.0), Some(1.0));
        assert_eq!(parse_decimal("0", 0.0, 1.0), Some(0.0));
        assert_eq!(parse_decimal("-0.5", -1.0, 1.0), Some(-0.5));
        assert_eq!(parse_decimal(".5", 0.0, 1.0), Some(0.5));
        for text in [
            "", "-", ".", "1.01", "-0.1", "1e-1", "inf", "nan", "half", "0.5.5", "+0.5", "0,5",
        ] {
            assert_eq!(parse_decimal(text, 0.0, 1.0), None, "{text:?}");
        }

        assert_eq!(format_decimal(0.75), "0.75");
        assert_eq!(format_decimal(1.0), "1");
        assert_eq!(format_decimal(0.0), "0");
        assert_eq!(format_decimal(-0.0), "0");
        assert_eq!(format_decimal(0.123_456), "0.123");
        assert_eq!(format_decimal(-0.000_1), "0");
        assert_eq!(format_decimal(12.5), "12.5");
    }

    #[test]
    fn clearing_a_distance_list_keeps_the_default() {
        let mut entries = vec![(638, 20_000), (0, 5_000), (999, 750)];
        assert_eq!(removable(&entries), 2);
        clear_ships(&mut entries);
        assert_eq!(entries, [(0, 5_000)]);
        assert_eq!(removable(&entries), 0);
        // A list with no default ends up empty; none is made up.
        let mut entries = vec![(638, 20_000)];
        assert_eq!(removable(&entries), 1);
        clear_ships(&mut entries);
        assert!(entries.is_empty());
        assert_eq!(removable(&[]), 0);
        assert!(!is_removable(0));
        assert!(is_removable(638));
    }

    #[test]
    fn types_are_shown_by_name_when_known() {
        let mut names = NameBook::default();
        names.set_item_type(638, CharacterName::Known("Raven".to_owned()));
        names.set_character(999, CharacterName::Known("A Character".to_owned()));
        assert_eq!(type_label(&names, 638), "Raven");
        assert_eq!(type_label(&names, 0), "Everything else (default)");
        // A character with the same ID is not the type's name.
        assert_eq!(type_label(&names, 999), "Type 999");
        // A ship in the list built into the app needs no lookup at all.
        assert_eq!(type_label(&NameBook::default(), 11_393), "Retribution");
        assert_eq!(type_label(&names, -5), "Type -5");
    }
}
