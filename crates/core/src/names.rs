//! Names for the IDs in settings file names.
//!
//! Character names come from ESI, EVE's public API, and are remembered on
//! disk so the app works offline. Account IDs can't be looked up anywhere,
//! so accounts get a label the user types in.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

/// The most IDs ESI resolves in one request.
pub const MAX_IDS_PER_REQUEST: usize = 1000;

/// How many "some ID is invalid" answers one lookup may cause.
///
/// ESI rejects a whole request when any one ID is invalid, so finding the
/// bad ones means asking again with smaller groups, and every rejection
/// counts against ESI's error limit (100 a minute when this was written).
/// Staying well under it leaves room for whatever else the user is running.
pub const MAX_REJECTIONS: usize = 40;

const ESI_NAMES_URL: &str = "https://esi.evetech.net/universe/names";
/// The ESI behaviour this code was written against.
const ESI_COMPATIBILITY_DATE: &str = "2025-08-26";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Below this many errors left in ESI's window, stop asking.
const ERROR_LIMIT_FLOOR: u32 = 20;

/// What is known about a character ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterName {
    Known(String),
    /// ESI says there is no such character. Remembered so it isn't asked
    /// about again on every start.
    NotFound,
}

/// The names and labels the app has collected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameBook {
    characters: BTreeMap<u64, CharacterName>,
    /// Names of kinds of item (ships, drones, structures...), by type ID.
    item_types: BTreeMap<u64, CharacterName>,
    account_labels: BTreeMap<u64, String>,
    /// The character the user marked as their main, if any.
    main_character: Option<u64>,
    /// The account the user marked as their main, if any.
    main_account: Option<u64>,
}

impl NameBook {
    /// A character's name, when it is known.
    pub fn character(&self, id: u64) -> Option<&str> {
        match self.characters.get(&id) {
            Some(CharacterName::Known(name)) => Some(name),
            Some(CharacterName::NotFound) | None => None,
        }
    }

    /// Whether a character ID still needs looking up.
    pub fn needs_lookup(&self, id: u64) -> bool {
        !self.characters.contains_key(&id)
    }

    pub fn set_character(&mut self, id: u64, name: CharacterName) {
        let name = match name {
            CharacterName::Known(name) => match clean(&name) {
                // An empty name isn't a name. Recording it as a miss stops
                // the same ID being asked about over and over.
                name if name.is_empty() => CharacterName::NotFound,
                name => CharacterName::Known(name),
            },
            CharacterName::NotFound => CharacterName::NotFound,
        };
        self.characters.insert(id, name);
    }

    /// The name of a kind of item, when it is known.
    pub fn item_type(&self, id: u64) -> Option<&str> {
        match self.item_types.get(&id) {
            Some(CharacterName::Known(name)) => Some(name),
            Some(CharacterName::NotFound) | None => None,
        }
    }

    /// Whether a type ID still needs looking up.
    pub fn item_type_needs_lookup(&self, id: u64) -> bool {
        !self.item_types.contains_key(&id)
    }

    pub fn set_item_type(&mut self, id: u64, name: CharacterName) {
        let name = match name {
            CharacterName::Known(name) => match clean(&name) {
                name if name.is_empty() => CharacterName::NotFound,
                name => CharacterName::Known(name),
            },
            CharacterName::NotFound => CharacterName::NotFound,
        };
        self.item_types.insert(id, name);
    }

    /// The label the user gave an account, if any.
    pub fn account_label(&self, id: u64) -> Option<&str> {
        self.account_labels.get(&id).map(String::as_str)
    }

    /// The character marked as the main one, if any.
    pub fn main_character(&self) -> Option<u64> {
        self.main_character
    }

    /// The account marked as the main one, if any.
    pub fn main_account(&self) -> Option<u64> {
        self.main_account
    }

    /// Marks a character as the main one, replacing any other. `None`
    /// leaves no character marked.
    pub fn set_main_character(&mut self, id: Option<u64>) {
        self.main_character = id;
    }

    /// Marks an account as the main one, replacing any other. `None` leaves
    /// no account marked.
    pub fn set_main_account(&mut self, id: Option<u64>) {
        self.main_account = id;
    }

    /// Sets an account's label. An empty label removes it.
    pub fn set_account_label(&mut self, id: u64, label: &str) {
        let label = clean(label);
        if label.is_empty() {
            self.account_labels.remove(&id);
        } else {
            self.account_labels.insert(id, label);
        }
    }

    /// Reads a name book saved by [`NameBook::save`]. A missing file is an
    /// empty book. Lines that don't make sense are ignored: this is a cache,
    /// and losing an entry only means looking it up again.
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    fn parse(text: &str) -> Self {
        let mut book = Self::default();
        for line in text.lines() {
            let mut fields = line.splitn(3, '\t');
            let (Some(kind), Some(id), value) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(id) = id.parse::<u64>() else {
                continue;
            };
            match (kind, value) {
                ("character", Some(name)) => {
                    book.set_character(id, CharacterName::Known(name.to_owned()));
                }
                ("no-character", _) => book.set_character(id, CharacterName::NotFound),
                ("type", Some(name)) => {
                    book.set_item_type(id, CharacterName::Known(name.to_owned()));
                }
                ("no-type", _) => book.set_item_type(id, CharacterName::NotFound),
                ("account", Some(label)) => book.set_account_label(id, label),
                ("main-character", _) => book.set_main_character(Some(id)),
                ("main-account", _) => book.set_main_account(Some(id)),
                _ => {}
            }
        }
        book
    }

    fn to_text(&self) -> String {
        let mut text = String::new();
        for (id, name) in &self.characters {
            match name {
                CharacterName::Known(name) => text.push_str(&format!("character\t{id}\t{name}\n")),
                CharacterName::NotFound => text.push_str(&format!("no-character\t{id}\n")),
            }
        }
        for (id, name) in &self.item_types {
            match name {
                CharacterName::Known(name) => text.push_str(&format!("type\t{id}\t{name}\n")),
                CharacterName::NotFound => text.push_str(&format!("no-type\t{id}\n")),
            }
        }
        for (id, label) in &self.account_labels {
            text.push_str(&format!("account\t{id}\t{label}\n"));
        }
        if let Some(id) = self.main_character {
            text.push_str(&format!("main-character\t{id}\n"));
        }
        if let Some(id) = self.main_account {
            text.push_str(&format!("main-account\t{id}\n"));
        }
        text
    }

    /// Saves the book, creating its folder if needed. The new contents are
    /// flushed to disk and then swapped in, so neither a crash nor a power
    /// cut can leave half a file or an empty one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut temp = path.as_os_str().to_owned();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        let written = fs::File::create(&temp)
            .and_then(|mut file| {
                file.write_all(self.to_text().as_bytes())?;
                file.sync_all()
            })
            .and_then(|()| fs::rename(&temp, path));
        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written
    }
}

/// A name or label made safe for a one-line, tab-separated record.
fn clean(text: &str) -> String {
    let single_spaced: Vec<&str> = text
        .split(|c: char| c.is_control() || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    single_spaced.join(" ")
}

/// Why a lookup stopped early.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LookupError {
    #[error("couldn't reach EVE's name service ({0})")]
    Network(String),
    #[error("EVE's name service answered with an error ({0})")]
    Service(String),
    #[error("EVE's name service sent an answer that couldn't be read ({0})")]
    BadAnswer(String),
    #[error("too many lookups failed, so the rest were left for later")]
    TooManyRejections,
}

/// The answer to one request for names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The character names among the IDs asked about.
    Names(Vec<(u64, String)>),
    /// At least one ID isn't valid, so none were resolved.
    SomeInvalid,
}

/// Something that can turn IDs into names. ESI in the app; a stand-in in
/// tests.
pub trait NameSource {
    /// Resolves up to [`MAX_IDS_PER_REQUEST`] IDs in one request.
    fn resolve(&self, ids: &[u64]) -> Result<Answer, LookupError>;
}

/// What a lookup found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lookup {
    /// Every ID whose answer is now known.
    pub found: Vec<(u64, CharacterName)>,
    /// Why some IDs are still unknown, if any are.
    pub stopped: Option<LookupError>,
}

/// Looks up the names of `ids`.
///
/// ESI rejects a whole request when one ID in it is invalid, so a rejected
/// group is split in half and each half asked again, until the invalid IDs
/// are found one by one.
pub fn look_up(source: &dyn NameSource, ids: &[u64]) -> Lookup {
    let mut lookup = Lookup::default();
    let mut rejections = 0;
    // Groups still to ask about, most recently split first.
    let mut pending: Vec<&[u64]> = ids.chunks(MAX_IDS_PER_REQUEST).rev().collect();
    while let Some(group) = pending.pop() {
        match source.resolve(group) {
            Ok(Answer::Names(names)) => {
                for &id in group {
                    let name = names.iter().find(|(named, _)| *named == id);
                    let name = match name {
                        Some((_, name)) => CharacterName::Known(name.clone()),
                        // Valid, but not a character: some other kind of
                        // thing happens to have this ID.
                        None => CharacterName::NotFound,
                    };
                    lookup.found.push((id, name));
                }
            }
            Ok(Answer::SomeInvalid) => {
                rejections += 1;
                if let [only] = group {
                    lookup.found.push((*only, CharacterName::NotFound));
                } else {
                    let (first, second) = group.split_at(group.len() / 2);
                    pending.push(second);
                    pending.push(first);
                }
                if rejections >= MAX_REJECTIONS && !pending.is_empty() {
                    lookup.stopped = Some(LookupError::TooManyRejections);
                    break;
                }
            }
            Err(error) => {
                lookup.stopped = Some(error);
                break;
            }
        }
    }
    lookup
}

#[derive(Deserialize)]
struct EsiName {
    category: String,
    id: u64,
    name: String,
}

/// ESI's category for characters.
pub const CHARACTER: &str = "character";
/// ESI's category for kinds of item.
pub const ITEM_TYPE: &str = "inventory_type";
/// ESI's category for solar systems.
pub const SOLAR_SYSTEM: &str = "solar_system";

/// Reads the body of a successful ESI answer, keeping only the entries in
/// one category. Different kinds of thing can share an ID.
pub fn parse_esi_names(body: &str, categories: &[&str]) -> Result<Vec<(u64, String)>, LookupError> {
    let entries: Vec<EsiName> =
        serde_json::from_str(body).map_err(|error| LookupError::BadAnswer(error.to_string()))?;
    Ok(entries
        .into_iter()
        .filter(|entry| categories.contains(&entry.category.as_str()))
        .map(|entry| (entry.id, entry.name))
        .collect())
}

/// Turns an ESI response into an [`Answer`].
///
/// `errors_left` is ESI's count of how many more errors it will tolerate in
/// its current window, when it sent one.
pub fn interpret_esi_response(
    status: u16,
    body: &str,
    errors_left: Option<u32>,
    categories: &[&str],
) -> Result<Answer, LookupError> {
    match status {
        200 => parse_esi_names(body, categories).map(Answer::Names),
        // Keep well clear of the limit: going over it gets the app blocked.
        404 if errors_left.is_some_and(|left| left < ERROR_LIMIT_FLOOR) => {
            Err(LookupError::TooManyRejections)
        }
        404 => Ok(Answer::SomeInvalid),
        other => Err(LookupError::Service(format!("status {other}"))),
    }
}

/// ESI, EVE's public API.
pub struct Esi {
    agent: ureq::Agent,
    /// Which kinds of name this source resolves.
    categories: &'static [&'static str],
}

/// What ESI asks every app to send: its name and version, and a way to reach
/// its makers, here the link to the source code.
fn user_agent(app_version: &str) -> String {
    format!(
        "clone-bay/{app_version} (+{})",
        env!("CARGO_PKG_REPOSITORY")
    )
}

impl Esi {
    /// `app_version` goes in the User-Agent, which ESI asks every app to send.
    pub fn new(app_version: &str) -> Self {
        Self::for_categories(app_version, &[CHARACTER])
    }

    /// A source of the names of the things settings refer to by ID: kinds
    /// of item (ships, drones, structures...) and solar systems. Their IDs
    /// never overlap (item types are below ten million, solar systems are
    /// thirty million and up), so one lookup serves both.
    pub fn for_item_types(app_version: &str) -> Self {
        Self::for_categories(app_version, &[ITEM_TYPE, SOLAR_SYSTEM])
    }

    fn for_categories(app_version: &str, categories: &'static [&'static str]) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            // The operating system's own trusted certificates.
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .user_agent(user_agent(app_version))
            .timeout_global(Some(REQUEST_TIMEOUT))
            // A 404 is an answer here, not a failure.
            .http_status_as_error(false)
            .tls_config(tls)
            .build()
            .new_agent();
        Self { agent, categories }
    }
}

impl NameSource for Esi {
    fn resolve(&self, ids: &[u64]) -> Result<Answer, LookupError> {
        let request = serde_json::to_string(ids)
            .map_err(|error| LookupError::BadAnswer(error.to_string()))?;
        let mut response = self
            .agent
            .post(ESI_NAMES_URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("X-Compatibility-Date", ESI_COMPATIBILITY_DATE)
            .send(request)
            .map_err(|error| LookupError::Network(error.to_string()))?;
        let errors_left = response
            .headers()
            .get("X-Esi-Error-Limit-Remain")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| LookupError::Network(error.to_string()))?;
        interpret_esi_response(status, &body, errors_left, self.categories)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A stand-in for ESI that knows some characters and treats listed IDs
    /// as invalid.
    struct Fake {
        characters: Vec<(u64, &'static str)>,
        invalid: Vec<u64>,
        requests: RefCell<Vec<Vec<u64>>>,
        fail_after: Option<usize>,
    }

    impl Fake {
        fn new(characters: &[(u64, &'static str)], invalid: &[u64]) -> Self {
            Self {
                characters: characters.to_vec(),
                invalid: invalid.to_vec(),
                requests: RefCell::new(Vec::new()),
                fail_after: None,
            }
        }
    }

    impl NameSource for Fake {
        fn resolve(&self, ids: &[u64]) -> Result<Answer, LookupError> {
            assert!(!ids.is_empty() && ids.len() <= MAX_IDS_PER_REQUEST);
            if self.fail_after == Some(self.requests.borrow().len()) {
                return Err(LookupError::Network("offline".into()));
            }
            self.requests.borrow_mut().push(ids.to_vec());
            if ids.iter().any(|id| self.invalid.contains(id)) {
                return Ok(Answer::SomeInvalid);
            }
            Ok(Answer::Names(
                self.characters
                    .iter()
                    .filter(|(id, _)| ids.contains(id))
                    .map(|(id, name)| (*id, (*name).to_owned()))
                    .collect(),
            ))
        }
    }

    fn known(name: &str) -> CharacterName {
        CharacterName::Known(name.to_owned())
    }

    fn sorted(mut found: Vec<(u64, CharacterName)>) -> Vec<(u64, CharacterName)> {
        found.sort_by_key(|(id, _)| *id);
        found
    }

    #[test]
    fn valid_ids_are_resolved_in_one_request() {
        let esi = Fake::new(&[(1, "Ann"), (2, "Bob")], &[]);
        let lookup = look_up(&esi, &[1, 2]);
        assert_eq!(lookup.found, [(1, known("Ann")), (2, known("Bob"))]);
        assert_eq!(lookup.stopped, None);
        assert_eq!(*esi.requests.borrow(), [vec![1, 2]]);
    }

    #[test]
    fn nothing_to_look_up_makes_no_request() {
        let esi = Fake::new(&[], &[]);
        assert_eq!(look_up(&esi, &[]), Lookup::default());
        assert!(esi.requests.borrow().is_empty());
    }

    #[test]
    fn an_invalid_id_is_found_by_splitting_and_the_rest_still_resolve() {
        let esi = Fake::new(&[(1, "Ann"), (2, "Bob"), (3, "Cy"), (5, "Eve")], &[4]);
        let lookup = look_up(&esi, &[1, 2, 3, 4, 5]);
        assert_eq!(
            sorted(lookup.found),
            [
                (1, known("Ann")),
                (2, known("Bob")),
                (3, known("Cy")),
                (4, CharacterName::NotFound),
                (5, known("Eve")),
            ]
        );
        assert_eq!(lookup.stopped, None);
        assert_eq!(
            *esi.requests.borrow(),
            [
                vec![1, 2, 3, 4, 5],
                vec![1, 2],
                vec![3, 4, 5],
                vec![3],
                vec![4, 5],
                vec![4],
                vec![5]
            ]
        );
    }

    #[test]
    fn a_valid_id_that_is_not_a_character_is_remembered_as_not_found() {
        // ESI answers, but the ID belongs to something else (it was
        // filtered out as a non-character), so no name comes back for it.
        let esi = Fake::new(&[(1, "Ann")], &[]);
        let lookup = look_up(&esi, &[1, 30_000_142]);
        assert_eq!(
            lookup.found,
            [(1, known("Ann")), (30_000_142, CharacterName::NotFound)]
        );
    }

    #[test]
    fn large_lookups_are_split_into_requests_of_a_thousand() {
        let ids: Vec<u64> = (1..=2_500).collect();
        let esi = Fake::new(&[], &[]);
        let lookup = look_up(&esi, &ids);
        let sizes: Vec<usize> = esi.requests.borrow().iter().map(Vec::len).collect();
        assert_eq!(sizes, [1000, 1000, 500]);
        assert_eq!(esi.requests.borrow()[0][0], 1);
        assert_eq!(esi.requests.borrow()[2][499], 2_500);
        assert_eq!(lookup.found.len(), 2_500);
    }

    #[test]
    fn a_network_failure_keeps_what_was_found_and_stops() {
        let mut esi = Fake::new(&[(1, "Ann"), (2, "Bob")], &[3]);
        // [1,2,3] is rejected, [1] resolves, then the connection drops.
        esi.fail_after = Some(2);
        let lookup = look_up(&esi, &[1, 2, 3]);
        assert_eq!(lookup.found, [(1, known("Ann"))]);
        assert_eq!(lookup.stopped, Some(LookupError::Network("offline".into())));
        assert_eq!(esi.requests.borrow().len(), 2);
    }

    #[test]
    fn rejections_are_capped_so_the_error_limit_is_never_reached() {
        // Every ID is invalid, so every request is a rejection.
        let ids: Vec<u64> = (1..=500).collect();
        let esi = Fake::new(&[], &ids);
        let lookup = look_up(&esi, &ids);
        assert_eq!(esi.requests.borrow().len(), MAX_REJECTIONS);
        assert_eq!(lookup.stopped, Some(LookupError::TooManyRejections));
        assert!(lookup
            .found
            .iter()
            .all(|(_, name)| *name == CharacterName::NotFound));
        assert!(lookup.found.len() < 40);
    }

    #[test]
    fn reaching_the_cap_on_the_very_last_request_is_not_an_early_stop() {
        // One real character followed by 20 invalid IDs takes exactly
        // MAX_REJECTIONS rejections, the last of them on the final request.
        let invalid: Vec<u64> = (2..=21).collect();
        let ids: Vec<u64> = (1..=21).collect();
        let esi = Fake::new(&[(1, "Ann")], &invalid);
        let lookup = look_up(&esi, &ids);
        assert_eq!(esi.requests.borrow().len(), MAX_REJECTIONS + 1);
        assert_eq!(esi.requests.borrow().last().unwrap(), &[21]);
        assert_eq!(lookup.stopped, None);
        assert_eq!(lookup.found.len(), 21);
    }

    #[test]
    fn the_user_agent_names_the_app_version_and_source() {
        assert_eq!(
            user_agent("1.2.3"),
            "clone-bay/1.2.3 (+https://github.com/kriller641/clone-bay)"
        );
    }

    #[test]
    fn esi_answers_are_read_and_only_characters_kept() {
        // A real answer, plus a corporation that must be ignored.
        let body = r#"[{"category":"character","id":2112345678,"name":"Example Pilot"},
                       {"category":"corporation","id":98000001,"name":"Some Corp"},
                       {"category":"character","id":7,"name":"Zoë O'Neil-Smith","extra":1}]"#;
        assert_eq!(
            parse_esi_names(body, &[CHARACTER]),
            Ok(vec![
                (2_112_345_678, "Example Pilot".to_owned()),
                (7, "Zoë O'Neil-Smith".to_owned())
            ])
        );
        assert_eq!(parse_esi_names("[]", &[CHARACTER]), Ok(vec![]));
        for bad in [
            "",
            "{}",
            r#"[{"id":1}]"#,
            r#"[{"category":"character","id":"1","name":"x"}]"#,
        ] {
            assert!(
                matches!(
                    parse_esi_names(bad, &[CHARACTER]),
                    Err(LookupError::BadAnswer(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_source_can_take_names_from_more_than_one_category() {
        let body = r#"[{"category":"inventory_type","id":638,"name":"Raven"},
                       {"category":"solar_system","id":30000142,"name":"Jita"},
                       {"category":"character","id":90000001,"name":"Somebody"}]"#;
        assert_eq!(
            parse_esi_names(body, &[ITEM_TYPE, SOLAR_SYSTEM]),
            Ok(vec![
                (638, "Raven".to_owned()),
                (30_000_142, "Jita".to_owned())
            ])
        );
        assert_eq!(parse_esi_names(body, &[]), Ok(vec![]));
    }

    #[test]
    fn item_type_names_are_a_separate_category() {
        let body = r#"[{"category":"inventory_type","id":638,"name":"Raven"},
                       {"category":"character","id":638,"name":"Somebody"}]"#;
        assert_eq!(
            parse_esi_names(body, &[ITEM_TYPE]),
            Ok(vec![(638, "Raven".to_owned())])
        );
        assert_eq!(
            parse_esi_names(body, &[CHARACTER]),
            Ok(vec![(638, "Somebody".to_owned())])
        );
        assert_eq!(
            interpret_esi_response(200, body, None, &[ITEM_TYPE]),
            Ok(Answer::Names(vec![(638, "Raven".to_owned())]))
        );

        let mut book = NameBook::default();
        assert!(book.item_type_needs_lookup(638));
        book.set_item_type(638, known("Raven"));
        book.set_item_type(5, CharacterName::NotFound);
        book.set_item_type(6, known("  "));
        book.set_character(638, known("Somebody"));
        assert_eq!(book.item_type(638), Some("Raven"));
        assert_eq!(book.character(638), Some("Somebody"));
        assert_eq!(book.item_type(5), None);
        assert_eq!(book.item_type(6), None);
        assert!(!book.item_type_needs_lookup(5) && !book.item_type_needs_lookup(6));
        assert!(book.item_type_needs_lookup(7));
        // Saved and loaded with everything else.
        let text = book.to_text();
        assert!(
            text.contains("type\t638\tRaven\n") && text.contains("no-type\t5\n"),
            "{text}"
        );
        assert_eq!(NameBook::parse(&text), book);
    }

    #[test]
    fn esi_statuses_are_interpreted() {
        let names = r#"[{"category":"character","id":1,"name":"Ann"}]"#;
        assert_eq!(
            interpret_esi_response(200, names, Some(100), &[CHARACTER]),
            Ok(Answer::Names(vec![(1, "Ann".to_owned())]))
        );
        let rejected = r#"{"error":"Ensure all IDs are valid before resolving."}"#;
        assert_eq!(
            interpret_esi_response(404, rejected, Some(100), &[CHARACTER]),
            Ok(Answer::SomeInvalid)
        );
        assert_eq!(
            interpret_esi_response(404, rejected, None, &[CHARACTER]),
            Ok(Answer::SomeInvalid)
        );
        assert_eq!(
            interpret_esi_response(404, rejected, Some(20), &[CHARACTER]),
            Ok(Answer::SomeInvalid)
        );
        // Close to ESI's error limit: stop asking.
        assert_eq!(
            interpret_esi_response(404, rejected, Some(19), &[CHARACTER]),
            Err(LookupError::TooManyRejections)
        );
        for status in [400, 420, 500, 503, 204] {
            assert_eq!(
                interpret_esi_response(status, "", Some(100), &[CHARACTER]),
                Err(LookupError::Service(format!("status {status}")))
            );
        }
        assert!(matches!(
            interpret_esi_response(200, "not json", Some(100), &[CHARACTER]),
            Err(LookupError::BadAnswer(_))
        ));
    }

    /// Talks to the real ESI, so it is opt-in:
    /// `cargo test -p clone-bay-core --lib -- --ignored esi_live`
    #[test]
    #[ignore = "makes real requests to ESI"]
    fn esi_live_resolves_characters_and_reports_invalid_ids() {
        let esi = Esi::new("test");
        // 196379789 is Chribba, a well-known player since 2003; 1 is not a
        // valid ID.
        let lookup = look_up(&esi, &[196_379_789, 1]);
        assert_eq!(lookup.stopped, None);
        // 638 is the Raven; it has been 638 for as long as there has been an API.
        // 30000142 is Jita. The same source names systems and item types.
        let types = look_up(&Esi::for_item_types("test"), &[638, 30_000_142]);
        assert_eq!(
            sorted(types.found),
            [(638, known("Raven")), (30_000_142, known("Jita"))]
        );
        let found = sorted(lookup.found);
        assert_eq!(found[0], (1, CharacterName::NotFound));
        assert!(matches!(&found[1], (196_379_789, CharacterName::Known(name)) if !name.is_empty()));
    }

    // Written with the "mark as main" feature and not yet run when committed.
    #[test]
    fn one_main_character_and_one_main_account_are_remembered() {
        let mut book = NameBook::default();
        assert_eq!((book.main_character(), book.main_account()), (None, None));
        book.set_main_character(Some(5));
        book.set_main_account(Some(9));
        // Marking another replaces the first: there is one of each.
        book.set_main_character(Some(6));
        assert_eq!(
            (book.main_character(), book.main_account()),
            (Some(6), Some(9))
        );
        let text = book.to_text();
        assert!(
            text.contains("main-character\t6\n") && text.contains("main-account\t9\n"),
            "{text}"
        );
        assert_eq!(NameBook::parse(&text), book);
        book.set_main_account(None);
        assert_eq!(book.main_account(), None);
        assert!(!book.to_text().contains("main-account"));
    }

    #[test]
    fn a_book_remembers_names_misses_and_labels() {
        let mut book = NameBook::default();
        assert!(book.needs_lookup(1));
        book.set_character(1, known("Ann"));
        book.set_character(2, CharacterName::NotFound);
        book.set_account_label(9, "main account");
        assert_eq!(book.character(1), Some("Ann"));
        assert_eq!(book.character(2), None);
        assert_eq!(book.character(3), None);
        assert!(!book.needs_lookup(1));
        assert!(!book.needs_lookup(2), "a miss is remembered");
        assert!(book.needs_lookup(3));
        assert_eq!(book.account_label(9), Some("main account"));
        assert_eq!(
            book.account_label(1),
            None,
            "characters and accounts are separate"
        );

        book.set_account_label(9, "   ");
        assert_eq!(book.account_label(9), None, "an empty label removes it");
    }

    #[test]
    fn names_and_labels_are_kept_to_one_clean_line() {
        let mut book = NameBook::default();
        book.set_account_label(1, "  my\tmain\r\naccount  ");
        assert_eq!(book.account_label(1), Some("my main account"));
        book.set_character(2, known("Ann\tO'Neil\n"));
        assert_eq!(book.character(2), Some("Ann O'Neil"));
        // An empty name isn't a name; it is remembered as a miss.
        book.set_character(3, known(" \t "));
        assert_eq!(book.character(3), None);
        assert!(!book.needs_lookup(3));
    }

    #[test]
    fn a_book_survives_saving_and_loading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("names.tsv");
        let mut book = NameBook::default();
        book.set_character(2_112_345_678, known("Example Pilot"));
        book.set_character(5, known("Zoë O'Neil-Smith"));
        book.set_character(6, CharacterName::NotFound);
        book.set_account_label(12_345_678, "main: PvE & mining");
        book.save(&path).unwrap();

        assert_eq!(NameBook::load(&path).unwrap(), book);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "character\t5\tZoë O'Neil-Smith\nno-character\t6\n\
             character\t2112345678\tExample Pilot\naccount\t12345678\tmain: PvE & mining\n"
        );
        // Saving again replaces the file and leaves nothing else behind.
        book.set_account_label(12_345_678, "");
        book.save(&path).unwrap();
        assert_eq!(NameBook::load(&path).unwrap(), book);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn a_missing_book_is_empty_and_a_damaged_one_keeps_what_it_can() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            NameBook::load(&dir.path().join("none.tsv")).unwrap(),
            NameBook::default()
        );

        let text = "character\t1\tAnn\n\
                    garbage\n\
                    character\tnot-a-number\tBob\n\
                    character\t3\n\
                    wizard\t4\tMerlin\n\
                    \n\
                    account\t5\tmain\n\
                    account\t6\n\
                    no-character\t7\n";
        let book = NameBook::parse(text);
        let mut expected = NameBook::default();
        expected.set_character(1, known("Ann"));
        expected.set_character(7, CharacterName::NotFound);
        expected.set_account_label(5, "main");
        assert_eq!(book, expected);
    }

    #[test]
    fn a_book_that_cannot_be_read_or_saved_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be.
        assert!(NameBook::load(dir.path()).is_err());
        assert!(NameBook::default().save(dir.path()).is_err());
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            0,
            "no temp file left"
        );
    }
}
