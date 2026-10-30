//! The ship and structure types built into the app, so their names are
//! known, and they can be searched for, without a connection.
//!
//! The list is generated from CCP's static data by `tools/make_type_list.py`.
//! Anything not in it (a ship added to the game since) is still shown by
//! the name ESI gives, when that can be looked up.

use std::sync::OnceLock;

const DATA: &str = include_str!("data/bundled_types.tsv");

/// Every bundled type as `(type ID, name)`, in alphabetical order.
pub fn bundled_types() -> &'static [(u64, &'static str)] {
    static TYPES: OnceLock<Vec<(u64, &'static str)>> = OnceLock::new();
    TYPES.get_or_init(|| parse(DATA))
}

/// Reads `id<TAB>name` lines. Comment lines and anything malformed are
/// skipped.
fn parse(data: &'static str) -> Vec<(u64, &'static str)> {
    data.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            let name = name.trim();
            let id = id.parse().ok()?;
            (!name.is_empty()).then_some((id, name))
        })
        .collect()
}

/// The name of a bundled type.
pub fn bundled_type_name(id: u64) -> Option<&'static str> {
    bundled_types()
        .iter()
        .find(|(type_id, _)| *type_id == id)
        .map(|(_, name)| *name)
}

/// Up to `limit` bundled types whose names contain `query`, whatever the
/// capitals. Names that start with it come first; each group is
/// alphabetical. An empty query finds nothing.
pub fn search_bundled_types(query: &str, limit: usize) -> Vec<(u64, &'static str)> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let matching = |starts: bool| {
        let query = query.clone();
        bundled_types().iter().copied().filter(move |(_, name)| {
            let name = name.to_lowercase();
            name.contains(&query) && name.starts_with(&query) == starts
        })
    };
    matching(true).chain(matching(false)).take(limit).collect()
}

const GROUP_DATA: &str = include_str!("data/scan_groups.tsv");

/// A kind of thing a probe scan can find: one of the game's item groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanGroup {
    pub id: i64,
    /// The category it is listed under, such as "Ship" or "Drone".
    pub category: &'static str,
    pub name: &'static str,
    /// Whether the game shows this group to players. Only these are
    /// offered for adding to a filter; the rest are here so that one a
    /// filter already holds can be shown by name.
    pub published: bool,
}

/// Every bundled scan group, by category and then name. The list is
/// generated from CCP's static data by `tools/make_group_list.py`.
pub fn scan_groups() -> &'static [ScanGroup] {
    static GROUPS: OnceLock<Vec<ScanGroup>> = OnceLock::new();
    GROUPS.get_or_init(|| parse_groups(GROUP_DATA))
}

/// Reads `id<TAB>category<TAB>published<TAB>name` lines. Comment lines and
/// anything malformed are skipped.
fn parse_groups(data: &'static str) -> Vec<ScanGroup> {
    data.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let id = fields.next()?.parse().ok()?;
            let category = fields.next()?.trim();
            let published = match fields.next()? {
                "1" => true,
                "0" => false,
                _ => return None,
            };
            let name = fields.next()?.trim();
            (id > 0 && !category.is_empty() && !name.is_empty()).then_some(ScanGroup {
                id,
                category,
                name,
                published,
            })
        })
        .collect()
}

/// The bundled scan group with this ID.
pub fn scan_group(id: i64) -> Option<&'static ScanGroup> {
    scan_groups().iter().find(|group| group.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_groups_cover_ships_and_what_else_probes_find() {
        let groups = scan_groups();
        assert!(groups.len() > 100, "{}", groups.len());
        let frigate = scan_group(25).unwrap();
        assert_eq!(
            (frigate.name, frigate.category, frigate.published),
            ("Frigate", "Ship", true)
        );
        assert_eq!(scan_group(1250).unwrap().name, "Mobile Tractor Unit");
        assert_eq!(scan_group(1250).unwrap().category, "Deployable");
        // A group the game doesn't show is known by name but marked.
        assert!(!scan_group(381).unwrap().published);
        assert_eq!(scan_group(0), None);
        assert_eq!(scan_group(18), None, "minerals are not found by probes");
        // No ID twice, and each category's groups sit together.
        let mut ids: Vec<i64> = groups.iter().map(|group| group.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), groups.len());
        let mut categories: Vec<&str> = groups.iter().map(|group| group.category).collect();
        categories.dedup();
        let mut distinct = categories.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(categories.len(), distinct.len());
        assert_eq!(categories[0], "Ship");
    }

    #[test]
    fn malformed_group_lines_are_skipped() {
        let data = "# comment\n5\tShip\t1\tA\nx\tShip\t1\tB\n6\tShip\t2\tC\n7\tShip\t0\n\
                    8\t\t1\tD\n0\tShip\t1\tE\n9\tDrone\t0\t F \n";
        let groups = parse_groups(data);
        assert_eq!(
            groups,
            [
                ScanGroup {
                    id: 5,
                    category: "Ship",
                    name: "A",
                    published: true
                },
                ScanGroup {
                    id: 9,
                    category: "Drone",
                    name: "F",
                    published: false
                },
            ]
        );
    }

    #[test]
    fn the_list_holds_hundreds_of_ships_and_some_structures() {
        let types = bundled_types();
        assert!(types.len() > 400, "{}", types.len());
        assert_eq!(bundled_type_name(638), Some("Raven"));
        assert_eq!(bundled_type_name(17_738), Some("Machariel"));
        assert_eq!(bundled_type_name(35_832), Some("Astrahus"));
        assert_eq!(bundled_type_name(0), None);
        assert_eq!(bundled_type_name(34), None, "Tritanium is not a ship");
        // No duplicates, no blanks.
        let mut ids: Vec<u64> = types.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), types.len());
        assert!(types.iter().all(|(_, name)| !name.trim().is_empty()));
    }

    #[test]
    fn lines_that_are_not_an_id_and_a_name_are_skipped() {
        let parsed = parse("# a comment\n638\tRaven\nnot a line\nx\tName\n7\t\n8\t Spaced \n");
        assert_eq!(parsed, [(638, "Raven"), (8, "Spaced")]);
    }

    #[test]
    fn searching_puts_names_that_start_with_the_text_first() {
        let names = |query: &str, limit: usize| -> Vec<&str> {
            search_bundled_types(query, limit)
                .into_iter()
                .map(|(_, name)| name)
                .collect()
        };
        let ravens = names("raven", 10);
        assert_eq!(ravens[0], "Raven");
        assert!(ravens.contains(&"Raven Navy Issue"), "{ravens:?}");
        // Whatever the capitals and stray spaces.
        assert_eq!(names("  RAVEN ", 10), ravens);
        // A match in the middle of a name comes after the ones at the start.
        let navy = names("navy", 200);
        assert!(navy.contains(&"Raven Navy Issue"));
        assert!(navy.iter().all(|name| name.to_lowercase().contains("navy")));
        // The limit is honoured, and nothing typed finds nothing.
        assert_eq!(names("a", 5).len(), 5);
        assert!(names("", 5).is_empty());
        assert!(names("   ", 5).is_empty());
        assert!(names("no ship is called this", 5).is_empty());
    }

    #[test]
    fn names_that_start_with_the_search_come_before_ones_that_only_contain_it() {
        let found = search_bundled_types("ap", usize::MAX);
        let starts: Vec<bool> = found
            .iter()
            .map(|(_, name)| name.to_lowercase().starts_with("ap"))
            .collect();
        let first_other = starts.iter().position(|starts| !starts).unwrap();
        assert!(first_other > 0, "nothing starts with it");
        assert!(
            starts[first_other..].iter().all(|starts| !starts),
            "a name that starts with it came after one that doesn't"
        );
        // Every match is listed once.
        let all = bundled_types()
            .iter()
            .filter(|(_, name)| name.to_lowercase().contains("ap"))
            .count();
        assert_eq!(found.len(), all);
    }
}
