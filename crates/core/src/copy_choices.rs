//! Remembering what the user last chose to copy, one choice for each kind of
//! file. Kept in the app's own folder, never in a settings file.
//!
//! One record a line, tab-separated: the kind of file, then what is
//! remembered about it. A line that can't be read is passed over, and a kind
//! with no record of its `everything` choice counts as never chosen.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use crate::app_file;
use crate::discovery::FileKind;

/// What was chosen on the copy screen. Which chat channels were chosen isn't
/// part of it, because they differ from one character to the next.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScopeChoice {
    /// Copy everything, as opposed to only what is ticked.
    pub everything: bool,
    /// IDs of the ticked groups.
    pub groups: BTreeSet<String>,
    pub everything_else: bool,
    pub chat: bool,
}

/// The last choice for each kind of file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CopyChoices {
    account: Option<ScopeChoice>,
    character: Option<ScopeChoice>,
}

impl CopyChoices {
    /// Reads the saved choices. A file that doesn't exist yet holds none.
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        app_file::replace(path, &self.to_text())
    }

    /// The last choice for this kind of file, if there has been one.
    pub fn get(&self, kind: FileKind) -> Option<&ScopeChoice> {
        match kind {
            FileKind::Account => self.account.as_ref(),
            FileKind::Character => self.character.as_ref(),
        }
    }

    pub fn set(&mut self, kind: FileKind, choice: ScopeChoice) {
        match kind {
            FileKind::Account => self.account = Some(choice),
            FileKind::Character => self.character = Some(choice),
        }
    }

    fn parse(text: &str) -> Self {
        Self {
            account: parse_kind(text, kind_name(FileKind::Account)),
            character: parse_kind(text, kind_name(FileKind::Character)),
        }
    }

    fn to_text(&self) -> String {
        let mut text = String::new();
        for kind in [FileKind::Account, FileKind::Character] {
            if let Some(choice) = self.get(kind) {
                write_kind(&mut text, kind_name(kind), choice);
            }
        }
        text
    }
}

fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Account => "account",
        FileKind::Character => "character",
    }
}

fn parse_kind(text: &str, kind: &str) -> Option<ScopeChoice> {
    let mut choice = ScopeChoice::default();
    let mut everything = None;
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(line_kind), Some(what), Some(value), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if line_kind != kind {
            continue;
        }
        match (what, value) {
            ("group", id) => {
                choice.groups.insert(id.to_owned());
            }
            ("everything", value) => everything = parse_switch(value),
            ("everything_else", value) => {
                choice.everything_else = parse_switch(value).unwrap_or(false);
            }
            ("chat", value) => choice.chat = parse_switch(value).unwrap_or(false),
            _ => {}
        }
    }
    choice.everything = everything?;
    Some(choice)
}

fn parse_switch(value: &str) -> Option<bool> {
    match value {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

fn write_kind(text: &mut String, kind: &str, choice: &ScopeChoice) {
    let switch = |on: bool| if on { "1" } else { "0" };
    let mut line = |what: &str, value: &str| {
        text.push_str(&format!("{kind}\t{what}\t{value}\n"));
    };
    line("everything", switch(choice.everything));
    line("everything_else", switch(choice.everything_else));
    line("chat", switch(choice.chat));
    for group in &choice.groups {
        line("group", group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(everything: bool, groups: &[&str], everything_else: bool, chat: bool) -> ScopeChoice {
        ScopeChoice {
            everything,
            groups: groups.iter().map(|group| (*group).to_owned()).collect(),
            everything_else,
            chat,
        }
    }

    #[test]
    fn each_kind_of_file_is_remembered_separately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("folder").join("choices.tsv");
        let account = choice(false, &["keys", "sound"], true, false);
        let character = choice(true, &[], false, true);

        let mut choices = CopyChoices::default();
        choices.set(FileKind::Account, account.clone());
        choices.save(&path).unwrap();
        let loaded = CopyChoices::load(&path).unwrap();
        assert_eq!(loaded.get(FileKind::Account), Some(&account));
        assert_eq!(loaded.get(FileKind::Character), None);

        choices.set(FileKind::Character, character.clone());
        choices.save(&path).unwrap();
        let loaded = CopyChoices::load(&path).unwrap();
        assert_eq!(loaded.get(FileKind::Account), Some(&account));
        assert_eq!(loaded.get(FileKind::Character), Some(&character));
    }

    #[test]
    fn a_file_that_does_not_exist_holds_no_choices() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = CopyChoices::load(&dir.path().join("none.tsv")).unwrap();
        assert_eq!(loaded, CopyChoices::default());
    }

    #[test]
    fn a_file_that_cannot_be_read_or_saved_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(CopyChoices::load(dir.path()).is_err());
        let file = dir.path().join("a file");
        fs::write(&file, b"x").unwrap();
        assert!(CopyChoices::default()
            .save(&file.join("choices.tsv"))
            .is_err());
    }

    #[test]
    fn lines_that_cannot_be_read_are_passed_over() {
        let text = "nonsense\n\
                    account\teverything\t0\n\
                    account\tgroup\tkeys\n\
                    account\tgroup\n\
                    account\tgroup\tsound\textra\n\
                    account\tsomething\t1\n\
                    account\teverything_else\tyes\n\
                    account\tchat\t1\n\
                    planet\teverything\t1\n";
        let choices = CopyChoices::parse(text);
        assert_eq!(
            choices.get(FileKind::Account),
            Some(&choice(false, &["keys"], false, true))
        );
    }

    #[test]
    fn a_kind_with_no_readable_everything_choice_counts_as_never_chosen() {
        for text in [
            "account\tgroup\tkeys\n",
            "account\teverything\tmaybe\naccount\tgroup\tkeys\n",
        ] {
            assert_eq!(CopyChoices::parse(text).get(FileKind::Account), None);
        }
    }
}
