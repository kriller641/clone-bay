//! The parts of the outside world the app depends on, gathered in one place
//! so tests can stand in for them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clone_bay_core::names::{Esi, NameSource};
use clone_bay_core::platform::EveStatus;
use clone_bay_core::safe_write::BackupSession;
use clone_bay_core::update::{Keys, ReleaseSource, Version};

const NAMES_FILE: &str = "names.tsv";
const COPY_CHOICES_FILE: &str = "copy_choices.tsv";
const UPDATE_FILE: &str = "update.tsv";
const ABOUT_SEEN_FILE: &str = "about_seen.txt";
/// Long enough for antivirus to scan a new exe before it starts.
const SELF_TEST_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

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
    /// Opens a web page in the browser. Errors are in plain words for
    /// showing to the user.
    pub open_link: Arc<OpenLink>,
    /// Where it is recorded that the About page has been shown, so it opens
    /// by itself only the first time. `None` means it never opens by
    /// itself.
    pub about_seen_file: Option<PathBuf>,
    /// Looking for and installing updates. `None` means the app never
    /// looks.
    pub updater: Option<Updater>,
}

/// Everything looking for and installing an update needs from outside.
#[derive(Clone)]
pub struct Updater {
    /// Where releases are found.
    pub source: Arc<dyn ReleaseSource + Send + Sync>,
    /// The keys a release must be signed with.
    pub keys: Arc<Keys>,
    /// The version that is running.
    pub running: Version,
    /// The running exe's file.
    pub exe: PathBuf,
    /// Where the tick box and the time of the last check are kept. `None`
    /// means they last only as long as the window is open.
    pub prefs_file: Option<PathBuf>,
    /// The time, in seconds since 1970.
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// Runs a downloaded exe to prove it starts and is the version given.
    pub self_test: Arc<ExeAction>,
    /// Starts the exe that has just been put in place, given the version
    /// it replaced.
    pub start: Arc<ExeAction>,
    /// Opens the page releases are downloaded from.
    pub open_page: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
}

/// Something done with an exe and a version number. Errors are in plain
/// words for showing to the user.
pub type ExeAction = dyn Fn(&Path, &str) -> Result<(), String> + Send + Sync;

/// Opens a folder in the system's file manager.
pub type ShowFolder = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

/// Opens a web page in the browser.
pub type OpenLink = dyn Fn(&str) -> Result<(), String> + Send + Sync;

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
            open_link: Arc::new(|url| {
                clone_bay_core::platform::open_url(url).map_err(|error| error.to_string())
            }),
            about_seen_file: clone_bay_core::platform::app_data_dir()
                .map(|folder| folder.join(ABOUT_SEEN_FILE)),
            updater: real_updater(),
        }
    }
}

fn real_updater() -> Option<Updater> {
    use clone_bay_core::{install, platform, update_sources};

    let exe = std::env::current_exe().ok()?;
    let running = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let (source, keys) = release_source();
    Some(Updater {
        source,
        keys: Arc::new(keys),
        running,
        exe,
        prefs_file: platform::app_data_dir().map(|folder| folder.join(UPDATE_FILE)),
        now: Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs())
        }),
        self_test: Arc::new(|staged, version| {
            install::self_test(staged, version, SELF_TEST_LIMIT).map_err(|error| error.to_string())
        }),
        start: Arc::new(|exe, old_version| {
            // The folder the app was opened with goes to the new version.
            let folder = match crate::launch::parse(std::env::args_os().skip(1)) {
                crate::launch::Launch::Open { folder, .. } => folder,
                crate::launch::Launch::SelfTest(_) => None,
            };
            install::start_updated(exe, old_version, folder.as_deref().map(Path::as_os_str))
                .map_err(|error| error.to_string())
        }),
        open_page: Arc::new(|| {
            platform::open_url(&update_sources::download_page()).map_err(|error| error.to_string())
        }),
    })
}

/// GitHub, signed with the built-in keys. A debug build can be pointed at
/// a folder instead, to try a whole update before a release depends on
/// it; a release build ignores the override.
fn release_source() -> (Arc<dyn ReleaseSource + Send + Sync>, Keys) {
    use clone_bay_core::update_sources::GitHubReleases;

    #[cfg(debug_assertions)]
    if let Some(folder) = std::env::var_os("CLONE_BAY_UPDATE_DIR") {
        use clone_bay_core::update_sources::FolderSource;

        let mut keys = Keys::built_in();
        if let Ok(key) = std::env::var("CLONE_BAY_UPDATE_KEY") {
            keys.add(&key);
        }
        return (Arc::new(FolderSource::new(folder.into())), keys);
    }
    (
        Arc::new(GitHubReleases::new(env!("CARGO_PKG_VERSION"))),
        Keys::built_in(),
    )
}

#[cfg(test)]
pub mod test_support {
    use std::sync::{Condvar, Mutex};

    use clone_bay_core::names::{Answer, LookupError};
    use clone_bay_core::update::{Offer, UpdateError};

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

    /// A signed release held in memory.
    pub struct FakeRelease {
        pub listing: String,
        pub signature: String,
        pub body: Vec<u8>,
        pub requests: Mutex<usize>,
    }

    impl ReleaseSource for FakeRelease {
        fn latest_text(&self, name: &str) -> Result<String, UpdateError> {
            *self.requests.lock().unwrap() += 1;
            match name {
                clone_bay_core::update::LISTING_NAME => Ok(self.listing.clone()),
                _ => Ok(self.signature.clone()),
            }
        }

        fn open(&self, _offer: &Offer) -> Result<Box<dyn std::io::Read>, UpdateError> {
            Ok(Box::new(std::io::Cursor::new(self.body.clone())))
        }
    }

    /// What a fake updater was asked to do.
    #[derive(Default)]
    pub struct UpdaterLog {
        /// The old version each start of a new exe was given.
        pub started: Mutex<Vec<String>>,
        pub pages_opened: Mutex<usize>,
        /// Makes the self-test fail.
        pub self_test_fails: Mutex<bool>,
        /// Makes starting the new exe fail.
        pub start_fails: Mutex<bool>,
    }

    /// The time every fake updater reports.
    pub const FAKE_NOW: u64 = 1_000_000;

    /// An updater running 0.1.4 from `dir/clone-bay.exe` (holding `old`),
    /// with a signed release of `offered` whose exe holds `new exe`.
    pub fn fake_updater(dir: &Path, offered: &str) -> (Updater, Arc<FakeRelease>, Arc<UpdaterLog>) {
        use sha2::{Digest, Sha256};

        let body = b"new exe".to_vec();
        let hash: String = Sha256::digest(&body)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let listing = format!("{hash}  clone-bay-v{offered}-windows-x64.exe\n");
        let minisign::KeyPair { pk, sk } =
            minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(None, &sk, std::io::Cursor::new(&listing), None, None)
            .unwrap()
            .into_string();
        let mut keys = Keys::default();
        assert!(keys.add(&pk.to_base64()));
        let release = Arc::new(FakeRelease {
            listing,
            signature,
            body,
            requests: Mutex::new(0),
        });
        let exe = dir.join("clone-bay.exe");
        std::fs::write(&exe, b"old").unwrap();
        let log = Arc::new(UpdaterLog::default());
        let (testing, starting, opening) = (Arc::clone(&log), Arc::clone(&log), Arc::clone(&log));
        let updater = Updater {
            source: Arc::clone(&release) as Arc<dyn ReleaseSource + Send + Sync>,
            keys: Arc::new(keys),
            running: Version::parse("0.1.4").unwrap(),
            exe,
            prefs_file: Some(dir.join("update.tsv")),
            now: Arc::new(|| FAKE_NOW),
            self_test: Arc::new(move |_, _| {
                if *testing.self_test_fails.lock().unwrap() {
                    Err("the new version didn't run properly when it was checked".to_owned())
                } else {
                    Ok(())
                }
            }),
            start: Arc::new(move |_, old_version| {
                if *starting.start_fails.lock().unwrap() {
                    return Err("the new version couldn't be started (blocked)".to_owned());
                }
                starting
                    .started
                    .lock()
                    .unwrap()
                    .push(old_version.to_owned());
                Ok(())
            }),
            open_page: Arc::new(move || {
                *opening.pages_opened.lock().unwrap() += 1;
                Ok(())
            }),
        };
        (updater, release, log)
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
                open_link: Arc::new(|_| Ok(())),
                about_seen_file: None,
                updater: None,
            }
        }
    }
}
