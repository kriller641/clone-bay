//! What the file list shows: filtering, sorting and the text in each cell.
//! Nothing here draws; it is plain data so it can be tested.

use std::cmp::Ordering;
use std::time::{Duration, SystemTime};

use clone_bay_core::discovery::{FileKind, SettingsFileEntry};
use clone_bay_core::inspect::{FileSummary, InspectError};
use clone_bay_core::names::NameBook;

/// Whether a file has been read yet, and how that went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Pending,
    Readable(FileSummary),
    Unreadable(InspectError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub entry: SettingsFileEntry,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KindFilter {
    #[default]
    All,
    Characters,
    Accounts,
}

impl KindFilter {
    fn allows(self, kind: FileKind) -> bool {
        match self {
            Self::All => true,
            Self::Characters => kind == FileKind::Character,
            Self::Accounts => kind == FileKind::Account,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Kind,
    Name,
    Id,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: SortColumn,
    pub descending: bool,
}

impl Default for Sort {
    /// Most recently used first, which is usually what the user is after.
    fn default() -> Self {
        Self {
            column: SortColumn::Modified,
            descending: true,
        }
    }
}

impl Sort {
    /// The sort after the user clicks a column header: the same column
    /// flips direction, a new column starts ascending.
    pub fn clicked(self, column: SortColumn) -> Self {
        Self {
            column,
            descending: self.column == column && !self.descending,
        }
    }
}

/// The name to show for a file: a character's name, or the label the user
/// gave an account.
pub fn display_name(names: &NameBook, kind: FileKind, id: u64) -> Option<&str> {
    match kind {
        FileKind::Character => names.character(id),
        FileKind::Account => names.account_label(id),
    }
}

/// Whether a file matches what was typed in a search box: anywhere in its
/// ID, or anywhere in its name whatever the capitals.
pub fn matches_search(search: &str, id: u64, name: Option<&str>) -> bool {
    let search = search.trim();
    search.is_empty()
        || id.to_string().contains(search)
        || name.is_some_and(|name| name.to_lowercase().contains(&search.to_lowercase()))
}

/// Indices into `rows` of the rows to show, in display order.
pub fn visible_rows(
    rows: &[Row],
    filter: KindFilter,
    search: &str,
    sort: Sort,
    names: &NameBook,
) -> Vec<usize> {
    let name = |entry: &SettingsFileEntry| display_name(names, entry.kind, entry.id);
    let mut visible: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| filter.allows(row.entry.kind))
        .filter(|(_, row)| matches_search(search, row.entry.id, name(&row.entry)))
        .map(|(index, _)| index)
        .collect();
    visible.sort_by(|&a, &b| {
        let (a, b) = (&rows[a].entry, &rows[b].entry);
        compare_entries((a, name(a)), (b, name(b)), sort)
    });
    visible
}

/// How two files, each with the name shown for it, are ordered by `sort`.
///
/// Ties keep kind-then-ID order so a list never shuffles between frames.
pub fn compare_entries(
    (a, name_a): (&SettingsFileEntry, Option<&str>),
    (b, name_b): (&SettingsFileEntry, Option<&str>),
    sort: Sort,
) -> Ordering {
    let primary = match sort.column {
        SortColumn::Kind => a.kind.cmp(&b.kind),
        SortColumn::Name => compare_names(name_a, name_b),
        SortColumn::Id => a.id.cmp(&b.id),
        SortColumn::Size => a.size.cmp(&b.size),
        SortColumn::Modified => a.modified.cmp(&b.modified),
    };
    let primary = if sort.descending {
        primary.reverse()
    } else {
        primary
    };
    primary.then_with(|| tie_break(a, b))
}

/// Alphabetical whatever the capitals, with unnamed files after named ones.
fn compare_names(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Whether a file is the one the user marked as their main character or
/// main account.
pub fn is_main(names: &NameBook, kind: FileKind, id: u64) -> bool {
    let main = match kind {
        FileKind::Character => names.main_character(),
        FileKind::Account => names.main_account(),
    };
    main == Some(id)
}

/// `visible` with the user's main character and main account moved to the
/// top, everything else left in the order it was in.
pub fn mains_first(rows: &[Row], visible: Vec<usize>, names: &NameBook) -> Vec<usize> {
    let main = |index: &usize| {
        let entry = &rows[*index].entry;
        is_main(names, entry.kind, entry.id)
    };
    let (mains, others): (Vec<usize>, Vec<usize>) = visible.into_iter().partition(main);
    mains.into_iter().chain(others).collect()
}

/// The text of the name column: the character's name or the account's
/// label, or the ID when there is neither and the file is a main one, with
/// "(Main)" after it for the user's main character or account. `None` when
/// there is nothing to show.
pub fn name_cell(names: &NameBook, kind: FileKind, id: u64) -> Option<String> {
    let name = display_name(names, kind, id);
    if is_main(names, kind, id) {
        let shown = name.map_or_else(|| id.to_string(), str::to_owned);
        Some(format!("{shown} (Main)"))
    } else {
        name.map(str::to_owned)
    }
}

fn tie_break(a: &SettingsFileEntry, b: &SettingsFileEntry) -> Ordering {
    (a.kind, a.id).cmp(&(b.kind, b.id))
}

pub fn kind_label(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Character => "Character",
        FileKind::Account => "Account",
    }
}

/// A count with the right form of its noun: "1 file", "2 files".
pub fn counted(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

pub fn status_label(status: &Status) -> String {
    match status {
        Status::Pending => "Checking…".to_owned(),
        Status::Readable(summary) => {
            format!(
                "OK, {}",
                counted(summary.sections.len(), "section", "sections")
            )
        }
        Status::Unreadable(error) => format!("Can't use: {error}"),
    }
}

/// A file size the way Windows Explorer rounds it.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else {
        // One decimal place, rounded down.
        let tenths = bytes / (MB / 10);
        format!("{}.{} MB", tenths / 10, tenths % 10)
    }
}

/// How long ago a file was modified, in the largest sensible unit.
pub fn format_age(modified: Option<SystemTime>, now: SystemTime) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const YEAR: u64 = 365 * DAY;
    let Some(modified) = modified else {
        return "unknown".to_owned();
    };
    // A file dated in the future (clock changes) counts as just now.
    let seconds = now
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    if seconds < MINUTE {
        "just now".to_owned()
    } else if seconds < HOUR {
        plural(seconds / MINUTE, "minute")
    } else if seconds < DAY {
        plural(seconds / HOUR, "hour")
    } else if seconds < YEAR {
        plural(seconds / DAY, "day")
    } else {
        plural(seconds / YEAR, "year")
    }
}

/// A one-line count of what is listed, for the status line.
pub fn tally(rows: &[Row]) -> String {
    let count = |want: fn(&Status) -> bool| rows.iter().filter(|row| want(&row.status)).count();
    let pending = count(|status| matches!(status, Status::Pending));
    let unreadable = count(|status| matches!(status, Status::Unreadable(_)));
    let mut text = counted(rows.len(), "file", "files");
    if pending > 0 {
        text.push_str(&format!(", checking {pending}"));
    }
    if unreadable > 0 {
        text.push_str(&format!(", {unreadable} can't be used"));
    }
    text
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn row(kind: FileKind, id: u64, size: u64, modified: u64) -> Row {
        Row {
            entry: SettingsFileEntry {
                kind,
                id,
                path: PathBuf::from(format!("{id}.dat")),
                size,
                modified: Some(at(modified)),
            },
            status: Status::Pending,
        }
    }

    fn sample() -> Vec<Row> {
        vec![
            row(FileKind::Character, 2_100, 500, 30),
            row(FileKind::Account, 21, 900, 10),
            row(FileKind::Character, 95, 100, 20),
            row(FileKind::Account, 210, 900, 40),
        ]
    }

    fn ids(rows: &[Row], visible: &[usize]) -> Vec<u64> {
        visible.iter().map(|&i| rows[i].entry.id).collect()
    }

    fn sorted(column: SortColumn, descending: bool) -> Vec<u64> {
        let rows = sample();
        let sort = Sort { column, descending };
        ids(
            &rows,
            &visible_rows(&rows, KindFilter::All, "", sort, &NameBook::default()),
        )
    }

    #[test]
    fn default_sort_is_newest_first() {
        let rows = sample();
        let visible = visible_rows(
            &rows,
            KindFilter::All,
            "",
            Sort::default(),
            &NameBook::default(),
        );
        assert_eq!(ids(&rows, &visible), [210, 2_100, 95, 21]);
    }

    #[test]
    fn sorts_by_each_column_in_both_directions() {
        assert_eq!(sorted(SortColumn::Id, false), [21, 95, 210, 2_100]);
        assert_eq!(sorted(SortColumn::Id, true), [2_100, 210, 95, 21]);
        assert_eq!(sorted(SortColumn::Modified, false), [21, 95, 2_100, 210]);
        // Characters sort before accounts; ties fall back to ID order.
        assert_eq!(sorted(SortColumn::Kind, false), [95, 2_100, 21, 210]);
        assert_eq!(sorted(SortColumn::Kind, true), [21, 210, 95, 2_100]);
        // The two 900-byte files tie, and keep ID order either way.
        assert_eq!(sorted(SortColumn::Size, false), [95, 2_100, 21, 210]);
        assert_eq!(sorted(SortColumn::Size, true), [21, 210, 2_100, 95]);
    }

    #[test]
    fn filters_by_kind() {
        let rows = sample();
        let sort = Sort {
            column: SortColumn::Id,
            descending: false,
        };
        let characters = visible_rows(
            &rows,
            KindFilter::Characters,
            "",
            sort,
            &NameBook::default(),
        );
        assert_eq!(ids(&rows, &characters), [95, 2_100]);
        let accounts = visible_rows(&rows, KindFilter::Accounts, "", sort, &NameBook::default());
        assert_eq!(ids(&rows, &accounts), [21, 210]);
    }

    #[test]
    fn search_matches_anywhere_in_the_id_and_ignores_surrounding_spaces() {
        let rows = sample();
        let sort = Sort {
            column: SortColumn::Id,
            descending: false,
        };
        let found = visible_rows(&rows, KindFilter::All, " 21 ", sort, &NameBook::default());
        assert_eq!(ids(&rows, &found), [21, 210, 2_100]);
        let found = visible_rows(
            &rows,
            KindFilter::Characters,
            "21",
            sort,
            &NameBook::default(),
        );
        assert_eq!(ids(&rows, &found), [2_100]);
        assert!(visible_rows(&rows, KindFilter::All, "x", sort, &NameBook::default()).is_empty());
    }

    fn named() -> NameBook {
        use clone_bay_core::names::CharacterName;
        let mut names = NameBook::default();
        names.set_character(2_100, CharacterName::Known("Zed".to_owned()));
        names.set_character(95, CharacterName::Known("anna Bell".to_owned()));
        names.set_account_label(210, "Main account");
        // An account and a character can share an ID without sharing a name.
        names.set_character(21, CharacterName::Known("Wrong".to_owned()));
        names
    }

    #[test]
    fn names_come_from_the_right_place_for_each_kind() {
        let names = named();
        assert_eq!(
            display_name(&names, FileKind::Character, 95),
            Some("anna Bell")
        );
        assert_eq!(
            display_name(&names, FileKind::Account, 210),
            Some("Main account")
        );
        assert_eq!(display_name(&names, FileKind::Account, 21), None);
        assert_eq!(display_name(&names, FileKind::Character, 210), None);
    }

    #[test]
    fn search_matches_names_whatever_the_capitals() {
        assert!(matches_search("", 5, None));
        assert!(matches_search("  ", 5, None));
        assert!(matches_search("5", 1_500, None));
        assert!(!matches_search("6", 1_500, None));
        assert!(matches_search("BELL", 1, Some("anna Bell")));
        assert!(matches_search(" na b ", 1, Some("anna Bell")));
        assert!(!matches_search("bella", 1, Some("anna Bell")));
        assert!(!matches_search("anna", 1, None));

        let rows = sample();
        let sort = Sort {
            column: SortColumn::Id,
            descending: false,
        };
        let found = visible_rows(&rows, KindFilter::All, "a", sort, &named());
        assert_eq!(ids(&rows, &found), [95, 210]);
    }

    #[test]
    fn sorting_by_name_ignores_capitals_and_puts_unnamed_files_last() {
        let rows = sample();
        let by_name = |descending| {
            let sort = Sort {
                column: SortColumn::Name,
                descending,
            };
            ids(
                &rows,
                &visible_rows(&rows, KindFilter::All, "", sort, &named()),
            )
        };
        // anna Bell (95), Main account (210), Zed (2100), then unnamed 21.
        assert_eq!(by_name(false), [95, 210, 2_100, 21]);
        assert_eq!(by_name(true), [21, 2_100, 210, 95]);
        assert_eq!(compare_names(None, None), Ordering::Equal);
        assert_eq!(compare_names(Some("b"), Some("A")), Ordering::Greater);
    }

    // Written with the "mark as main" feature and not yet run when committed.
    #[test]
    fn mains_go_to_the_top_and_are_labelled() {
        let rows = sample();
        let mut names = named();
        names.set_main_character(Some(95));
        names.set_main_account(Some(21));
        assert!(is_main(&names, FileKind::Character, 95));
        assert!(
            !is_main(&names, FileKind::Account, 95),
            "the kinds are separate"
        );
        assert!(is_main(&names, FileKind::Account, 21));
        assert!(!is_main(&names, FileKind::Character, 2_100));

        let sort = Sort {
            column: SortColumn::Id,
            descending: true,
        };
        let visible = visible_rows(&rows, KindFilter::All, "", sort, &names);
        assert_eq!(ids(&rows, &visible), [2_100, 210, 95, 21]);
        // The two mains come first, in the order they were in; the rest follow.
        let pinned = mains_first(&rows, visible, &names);
        assert_eq!(ids(&rows, &pinned), [95, 21, 2_100, 210]);
        // With no main marked nothing moves.
        let plain = visible_rows(&rows, KindFilter::All, "", sort, &named());
        assert_eq!(mains_first(&rows, plain.clone(), &named()), plain);

        assert_eq!(
            name_cell(&names, FileKind::Character, 95).as_deref(),
            Some("anna Bell (Main)")
        );
        // An account with no label shows its ID.
        assert_eq!(
            name_cell(&names, FileKind::Account, 21).as_deref(),
            Some("21 (Main)")
        );
        assert_eq!(
            name_cell(&names, FileKind::Account, 210).as_deref(),
            Some("Main account")
        );
        assert_eq!(name_cell(&names, FileKind::Account, 999), None);
    }

    #[test]
    fn clicking_a_header_flips_or_switches_the_sort() {
        let start = Sort {
            column: SortColumn::Id,
            descending: false,
        };
        let flipped = start.clicked(SortColumn::Id);
        assert_eq!((flipped.column, flipped.descending), (SortColumn::Id, true));
        let back = flipped.clicked(SortColumn::Id);
        assert_eq!((back.column, back.descending), (SortColumn::Id, false));
        let switched = flipped.clicked(SortColumn::Size);
        assert_eq!(
            (switched.column, switched.descending),
            (SortColumn::Size, false)
        );
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1025), "2 KB");
        assert_eq!(format_size(1024 * 1024 - 1), "1024 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1_342_177), "1.2 MB");
        assert_eq!(format_size(25 * 1024 * 1024), "25.0 MB");
    }

    #[test]
    fn ages_use_the_largest_unit() {
        let now = at(10 * 365 * 86_400);
        let ago = |seconds: u64| format_age(Some(now - Duration::from_secs(seconds)), now);
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(3_599), "59 minutes ago");
        assert_eq!(ago(3_600), "1 hour ago");
        assert_eq!(ago(86_399), "23 hours ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(364 * 86_400), "364 days ago");
        assert_eq!(ago(365 * 86_400), "1 year ago");
        assert_eq!(ago(800 * 86_400), "2 years ago");
    }

    #[test]
    fn odd_times_do_not_break_the_age() {
        let now = at(1_000);
        assert_eq!(format_age(None, now), "unknown");
        assert_eq!(format_age(Some(at(5_000)), now), "just now");
    }

    #[test]
    fn labels_describe_each_state() {
        assert_eq!(kind_label(FileKind::Character), "Character");
        assert_eq!(kind_label(FileKind::Account), "Account");
        assert_eq!(status_label(&Status::Pending), "Checking…");
        let summary = FileSummary {
            format_version: 0,
            had_checksum: false,
            sections: Vec::new(),
        };
        assert_eq!(status_label(&Status::Readable(summary)), "OK, 0 sections");
        assert_eq!(
            status_label(&Status::Unreadable(InspectError::ExtraData)),
            "Can't use: unexpected extra data in the file"
        );
    }

    #[test]
    fn counts_use_the_singular_only_for_one() {
        assert_eq!(counted(0, "entry", "entries"), "0 entries");
        assert_eq!(counted(1, "entry", "entries"), "1 entry");
        assert_eq!(counted(2, "entry", "entries"), "2 entries");
        let one_section = FileSummary {
            format_version: 0,
            had_checksum: false,
            sections: vec![clone_bay_core::inspect::SectionSummary {
                name: "ui".to_owned(),
                entries: None,
            }],
        };
        assert_eq!(
            status_label(&Status::Readable(one_section)),
            "OK, 1 section"
        );
    }

    #[test]
    fn tally_counts_what_needs_attention() {
        let mut rows = sample();
        assert_eq!(tally(&rows), "4 files, checking 4");
        for row in &mut rows {
            row.status = Status::Unreadable(InspectError::NotADict);
        }
        assert_eq!(tally(&rows), "4 files, 4 can't be used");
        rows.truncate(1);
        rows[0].status = Status::Readable(FileSummary {
            format_version: 0,
            had_checksum: false,
            sections: Vec::new(),
        });
        assert_eq!(tally(&rows), "1 file");
        assert_eq!(tally(&[]), "0 files");
    }
}
