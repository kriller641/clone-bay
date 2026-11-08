//! The parts of the outside world the app depends on, gathered in one place
//! so tests can stand in for them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clone_bay_core::names::{Esi, NameSource};
use clone_bay_core::platform::EveStatus;
use clone_bay_core::safe_write::BackupSession;

const NAMES_FILE: &str = "names.tsv";
const COPY_CHOICES_FILE: &str = "copy_choices.tsv";

#[derive(Clone)]
pub struct Services {
    /// Whether the EVE client is running right now.
    pub eve_status: Arc<dyn Fn() -> EveStatus + Send + Sync>,
    /// The backups made while the app is open. Every write goes through it.
    pub backups: Arc<BackupSession>,
    /// Where character names are looked up.
    pub name_source: Arc<dyn NameSource + Send + Sync>,
    /// Where the names of kinds of item are looked up.
    pub type_name_source: Arc<dyn NameSource + Send + Sync>,
    /// Where names and account labels are remembered. `None` means they
    /// last only as long as the window is open.
    pub names_file: Option<PathBuf>,
    /// Where what was last chosen to copy is remembered. `None` means it
    /// lasts only as long as the copy screen is open.
    pub copy_choices_file: Option<PathBuf>,
    /// The clipboard, for copying probe formations out and pasting them in.
    pub clipboard: Arc<dyn Clipboard + Send + Sync>,
    /// Opens a folder in the system's file manager. Errors are in plain
    /// words for showing to the user.
    pub show_folder: Arc<ShowFolder>,
}

/// Opens a folder in the system's file manager.
pub type ShowFolder = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

/// The text clipboard. Errors are in plain words for showing to the user.
pub trait Clipboard {
    fn copy(&self, text: &str) -> Result<(), String>;
    fn paste(&self) -> Result<String, String>;
}

/// The operating system's clipboard.
struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy(&self, text: &str) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .map_err(|error| format!("the clipboard couldn't be written to ({error})"))
    }

    fn paste(&self) -> Result<String, String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|error| match error {
                arboard::Error::ContentNotAvailable => "the clipboard has no text on it".to_owned(),
                other => format!("the clipboard couldn't be read ({other})"),
            })
    }
}

impl Services {
    pub fn real() -> Self {
        Self {
            eve_status: Arc::new(clone_bay_core::platform::eve_status),
            backups: Arc::new(BackupSession::starting_now()),
            name_source: Arc::new(Esi::new(env!("CARGO_PKG_VERSION"))),
            type_name_source: Arc::new(Esi::for_item_types(env!("CARGO_PKG_VERSION"))),
            names_file: clone_bay_core::platform::app_data_dir()
                .map(|folder| folder.join(NAMES_FILE)),
            copy_choices_file: clone_bay_core::platform::app_data_dir()
                .map(|folder| folder.join(COPY_CHOICES_FILE)),
            clipboard: Arc::new(SystemClipboard),
            show_folder: Arc::new(|folder| {
                clone_bay_core::platform::show_folder(folder).map_err(|error| error.to_string())
            }),
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use std::sync::{Condvar, Mutex};

    use clone_bay_core::names::{Answer, LookupError};

    use super::*;

    /// Holds every check of whether EVE is running until `open` is called,
    /// so a save or copy stays under way for as long as a test needs.
    #[derive(Clone, Default)]
    pub struct EveGate(Arc<(Mutex<bool>, Condvar)>);

    impl EveGate {
        /// The `eve_status` service: waits for the gate, then says EVE is
        /// not running.
        pub fn eve_status(&self) -> Arc<dyn Fn() -> EveStatus + Send + Sync> {
            let gate = Arc::clone(&self.0);
            Arc::new(move || {
                let (open, opened) = &*gate;
                let mut is_open = open.lock().unwrap();
                while !*is_open {
                    is_open = opened.wait(is_open).unwrap();
                }
                EveStatus::NotRunning
            })
        }

        pub fn open(&self) {
            let (open, opened) = &*self.0;
            *open.lock().unwrap() = true;
            opened.notify_all();
        }
    }

    /// A name service that knows the given characters and counts requests.
    /// Tests must never talk to the real one.
    pub struct FakeNames {
        pub characters: Vec<(u64, &'static str)>,
        pub requests: Mutex<Vec<Vec<u64>>>,
        pub offline: bool,
    }

    impl FakeNames {
        pub fn knowing(characters: &[(u64, &'static str)]) -> Arc<Self> {
            Arc::new(Self {
                characters: characters.to_vec(),
                requests: Mutex::new(Vec::new()),
                offline: false,
            })
        }

        pub fn offline() -> Arc<Self> {
            Arc::new(Self {
                characters: Vec::new(),
                requests: Mutex::new(Vec::new()),
                offline: true,
            })
        }

        pub fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    impl NameSource for FakeNames {
        fn resolve(&self, ids: &[u64]) -> Result<Answer, LookupError> {
            self.requests.lock().unwrap().push(ids.to_vec());
            if self.offline {
                return Err(LookupError::Network("offline".to_owned()));
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

    /// Stands in for the clipboard: it holds one piece of text, or nothing.
    #[derive(Default)]
    pub struct FakeClipboard {
        pub text: Mutex<Option<String>>,
        /// Makes copying fail, as when another program holds the clipboard.
        pub broken: Mutex<bool>,
    }

    impl Clipboard for FakeClipboard {
        fn copy(&self, text: &str) -> Result<(), String> {
            if *self.broken.lock().unwrap() {
                return Err("the clipboard couldn't be written to (busy)".to_owned());
            }
            *self.text.lock().unwrap() = Some(text.to_owned());
            Ok(())
        }

        fn paste(&self) -> Result<String, String> {
            let text = self.text.lock().unwrap().clone();
            text.ok_or_else(|| "the clipboard has no text on it".to_owned())
        }
    }

    impl Services {
        /// Nothing real: EVE is not running, the name service knows nobody,
        /// nothing is saved, the clipboard is a stand-in that starts empty,
        /// and no folder is ever shown.
        pub fn for_tests() -> Self {
            Self {
                eve_status: Arc::new(|| EveStatus::NotRunning),
                backups: Arc::new(BackupSession::new("T")),
                name_source: FakeNames::knowing(&[]),
                type_name_source: FakeNames::knowing(&[]),
                names_file: None,
                copy_choices_file: None,
                clipboard: Arc::new(FakeClipboard::default()),
                show_folder: Arc::new(|_| Ok(())),
            }
        }
    }
}
