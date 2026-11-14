//! Looking for a newer version and installing it. No drawing happens here.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use clone_bay_core::install::{staged_path, swap_in, undo_swap};
use clone_bay_core::update::{check, download, Offer};
use clone_bay_core::update_prefs::UpdatePrefs;

use crate::services::Updater;
use crate::state::Wake;

/// The one sentence about updates the window shows, if there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// This version has just replaced an older one.
    Updated(String),
    /// A newer version can be installed.
    Available(String),
    /// It is being downloaded and checked.
    Downloading(String),
    /// It is downloaded and checked; the app has to restart to use it.
    Ready(String),
    /// It couldn't be installed. Nothing was changed.
    Failed(String),
}

enum Stage {
    Quiet,
    Offered(Offer),
    Downloading { offer: Offer, received: u64 },
    Ready(Offer),
    Failed(String),
}

enum Event {
    Checked(Option<Offer>),
    Progress(u64),
    Staged(Result<(), String>),
}

pub struct UpdateFlow {
    updater: Updater,
    wake: Wake,
    prefs: UpdatePrefs,
    stage: Stage,
    /// The version this one has just replaced, until the user has read so.
    updated_from: Option<String>,
    events: Option<Receiver<Event>>,
}

impl UpdateFlow {
    /// Looks for a newer version in the background if a check is due.
    pub fn new(updater: Updater, wake: Wake, updated_from: Option<String>) -> Self {
        // A prefs file that can't be read counts as never having been
        // written: checking is on.
        let prefs = updater
            .prefs_file
            .as_deref()
            .and_then(|file| UpdatePrefs::load(file).ok())
            .unwrap_or_default();
        let mut flow = Self {
            updater,
            wake,
            prefs,
            stage: Stage::Quiet,
            updated_from,
            events: None,
        };
        if flow.prefs.due((flow.updater.now)()) {
            flow.start_check();
        }
        flow
    }

    fn start_check(&mut self) {
        // Remembered whether or not the check works out, so a PC that is
        // offline asks once a day, not at every start.
        self.prefs.last_check = Some((self.updater.now)());
        self.save_prefs();
        let source = Arc::clone(&self.updater.source);
        let keys = Arc::clone(&self.updater.keys);
        let (running, wake) = (self.updater.running, Arc::clone(&self.wake));
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        thread::spawn(move || {
            // Offline, a bad signature, anything else: no offer, silently.
            let offer = check(source.as_ref(), &keys, running).ok().flatten();
            // If the window has gone there is nobody to tell.
            let _ = sender.send(Event::Checked(offer));
            wake();
        });
    }

    /// If it can't be saved the choice lasts as long as the window does.
    fn save_prefs(&self) {
        if let Some(file) = &self.updater.prefs_file {
            let _ = self.prefs.save(file);
        }
    }

    /// Whether the app looks for updates when it opens.
    pub fn checking(&self) -> bool {
        self.prefs.check
    }

    pub fn set_checking(&mut self, on: bool) {
        self.prefs.check = on;
        self.save_prefs();
    }

    /// The version that is running, as shown in the window.
    pub fn running(&self) -> String {
        self.updater.running.to_string()
    }

    /// Whether something is under way in the background.
    #[cfg(test)]
    pub fn busy(&self) -> bool {
        self.events.is_some()
    }

    /// The user chose to update: downloads the offered version and checks
    /// it, in the background. The running exe is not touched.
    pub fn start_update(&mut self) {
        let Stage::Offered(offer) = &self.stage else {
            return;
        };
        let offer = offer.clone();
        self.stage = Stage::Downloading {
            offer: offer.clone(),
            received: 0,
        };
        let updater = self.updater.clone();
        let wake = Arc::clone(&self.wake);
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        thread::spawn(move || {
            let staged = staged_path(&updater.exe);
            let progress = |received| {
                let _ = sender.send(Event::Progress(received));
                wake();
            };
            let result = download(updater.source.as_ref(), &offer, &staged, &progress)
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    let tested = (updater.self_test)(&staged, &offer.version.to_string());
                    if tested.is_err() {
                        let _ = std::fs::remove_file(&staged);
                    }
                    tested
                });
            let _ = sender.send(Event::Staged(result));
            wake();
        });
    }

    /// Takes in what the background work has reported. Returns true when a
    /// download has just become ready to install.
    pub fn receive(&mut self) -> bool {
        let mut became_ready = false;
        loop {
            let Some(event) = self
                .events
                .as_ref()
                .and_then(|events| events.try_recv().ok())
            else {
                return became_ready;
            };
            match event {
                Event::Checked(offer) => {
                    self.events = None;
                    if let Some(offer) = offer {
                        self.stage = Stage::Offered(offer);
                    }
                }
                Event::Progress(now) => {
                    if let Stage::Downloading { received, .. } = &mut self.stage {
                        *received = now;
                    }
                }
                Event::Staged(result) => {
                    self.events = None;
                    let Stage::Downloading { offer, .. } = &self.stage else {
                        continue;
                    };
                    match result {
                        Ok(()) => {
                            self.stage = Stage::Ready(offer.clone());
                            became_ready = true;
                        }
                        Err(why) => self.stage = Stage::Failed(why),
                    }
                }
            }
        }
    }

    /// Puts the downloaded version in place of the running one and starts
    /// it. Returns whether it started, in which case this window must
    /// close. On any failure the running exe is back as it was.
    pub fn install(&mut self) -> bool {
        let Stage::Ready(_) = &self.stage else {
            return false;
        };
        let exe = &self.updater.exe;
        let installed = swap_in(exe, &staged_path(exe))
            .map_err(|error| error.to_string())
            .and_then(|()| {
                (self.updater.start)(exe, &self.updater.running.to_string()).inspect_err(|_| {
                    let _ = undo_swap(exe);
                })
            });
        match installed {
            Ok(()) => true,
            Err(why) => {
                self.stage = Stage::Failed(why);
                false
            }
        }
    }

    /// Opens the page the newest version is downloaded from.
    pub fn open_page(&mut self) {
        if let Err(why) = (self.updater.open_page)() {
            self.stage = Stage::Failed(why);
        }
    }

    /// The user has read a failure or the "updated" message.
    pub fn dismiss(&mut self) {
        if matches!(self.stage, Stage::Failed(_)) {
            self.stage = Stage::Quiet;
        } else {
            self.updated_from = None;
        }
    }

    pub fn notice(&self) -> Option<Notice> {
        match &self.stage {
            Stage::Offered(offer) => Some(Notice::Available(format!(
                "Clone Bay {} is available.",
                offer.version
            ))),
            Stage::Downloading { offer, received } => Some(Notice::Downloading(format!(
                "Downloading Clone Bay {}… {}",
                offer.version,
                megabytes(*received)
            ))),
            Stage::Ready(offer) => Some(Notice::Ready(format!(
                "Clone Bay {} is ready.",
                offer.version
            ))),
            Stage::Failed(why) => Some(Notice::Failed(format!(
                "Couldn't update: {why}. Clone Bay is unchanged."
            ))),
            Stage::Quiet => self
                .updated_from
                .as_ref()
                .map(|_| Notice::Updated(format!("Updated to Clone Bay {}.", self.running()))),
        }
    }
}

/// A size as megabytes with one decimal: `3.2 MB`.
fn megabytes(bytes: u64) -> String {
    let tenths = bytes / (1024 * 1024 / 10);
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use clone_bay_core::install::{old_path, staged_path};
    use clone_bay_core::update_prefs::UpdatePrefs;

    use super::*;
    use crate::services::test_support::{fake_updater, FAKE_NOW};
    use crate::state::test_support::quiet;

    /// Takes in background results until `ready`.
    fn wait(flow: &mut UpdateFlow, ready: impl Fn(&UpdateFlow) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            flow.receive();
            if ready(flow) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the update"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn offered(flow: &UpdateFlow) -> bool {
        matches!(flow.notice(), Some(Notice::Available(_)))
    }

    fn settled(flow: &UpdateFlow) -> bool {
        !flow.busy()
    }

    #[test]
    fn a_newer_version_is_offered_and_the_check_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, _) = fake_updater(dir.path(), "0.1.5");
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, offered);
        assert_eq!(
            flow.notice(),
            Some(Notice::Available(
                "Clone Bay 0.1.5 is available.".to_owned()
            ))
        );
        let saved = UpdatePrefs::load(&dir.path().join("update.tsv")).unwrap();
        assert_eq!(saved.last_check, Some(FAKE_NOW));
    }

    #[test]
    fn the_running_version_is_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, release, _) = fake_updater(dir.path(), "0.1.4");
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, settled);
        assert_eq!(flow.notice(), None);
        assert_eq!(*release.requests.lock().unwrap(), 2);
    }

    #[test]
    fn nothing_is_asked_when_checking_is_off_or_was_done_today() {
        for prefs in [
            UpdatePrefs {
                check: false,
                last_check: None,
            },
            UpdatePrefs {
                check: true,
                last_check: Some(FAKE_NOW - 60),
            },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (updater, release, _) = fake_updater(dir.path(), "0.1.5");
            prefs.save(&dir.path().join("update.tsv")).unwrap();
            let mut flow = UpdateFlow::new(updater, quiet(), None);
            wait(&mut flow, settled);
            assert_eq!(flow.notice(), None);
            assert_eq!(*release.requests.lock().unwrap(), 0);
        }
    }

    #[test]
    fn the_tick_box_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, _) = fake_updater(dir.path(), "0.1.4");
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        assert!(flow.checking());
        flow.set_checking(false);
        assert!(!flow.checking());
        let saved = UpdatePrefs::load(&dir.path().join("update.tsv")).unwrap();
        assert!(!saved.check);
    }

    #[test]
    fn updating_downloads_checks_swaps_and_starts_the_new_version() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, log) = fake_updater(dir.path(), "0.1.5");
        let exe = updater.exe.clone();
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, offered);
        flow.start_update();
        assert!(matches!(flow.notice(), Some(Notice::Downloading(_))));
        wait(&mut flow, |flow| {
            matches!(flow.notice(), Some(Notice::Ready(_)))
        });
        assert_eq!(
            flow.notice(),
            Some(Notice::Ready("Clone Bay 0.1.5 is ready.".to_owned()))
        );
        // Nothing is swapped until the window is ready to close.
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert_eq!(fs::read(staged_path(&exe)).unwrap(), b"new exe");

        assert!(flow.install());
        assert_eq!(fs::read(&exe).unwrap(), b"new exe");
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"old");
        assert_eq!(*log.started.lock().unwrap(), ["0.1.4"]);
    }

    #[test]
    fn a_download_that_fails_its_check_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, log) = fake_updater(dir.path(), "0.1.5");
        *log.self_test_fails.lock().unwrap() = true;
        let exe = updater.exe.clone();
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, offered);
        flow.start_update();
        wait(&mut flow, |flow| {
            matches!(flow.notice(), Some(Notice::Failed(_)))
        });
        assert_eq!(
            flow.notice(),
            Some(Notice::Failed(
                "Couldn't update: the new version didn't run properly when it was checked. \
                 Clone Bay is unchanged."
                    .to_owned()
            ))
        );
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert!(!staged_path(&exe).exists());
        assert!(!flow.install());
    }

    #[test]
    fn a_new_version_that_will_not_start_is_swapped_back() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, log) = fake_updater(dir.path(), "0.1.5");
        *log.start_fails.lock().unwrap() = true;
        let exe = updater.exe.clone();
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, offered);
        flow.start_update();
        wait(&mut flow, |flow| {
            matches!(flow.notice(), Some(Notice::Ready(_)))
        });
        assert!(!flow.install());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert!(!old_path(&exe).exists());
        assert!(matches!(flow.notice(), Some(Notice::Failed(_))));
    }

    #[test]
    fn a_folder_that_cannot_be_written_to_fails_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut updater, _, _) = fake_updater(dir.path(), "0.1.5");
        // An exe in a folder that doesn't exist: the download can't be saved.
        updater.exe = dir.path().join("gone").join("clone-bay.exe");
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, offered);
        flow.start_update();
        wait(&mut flow, |flow| {
            matches!(flow.notice(), Some(Notice::Failed(_)))
        });
    }

    #[test]
    fn after_an_update_it_says_so_until_dismissed() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, _) = fake_updater(dir.path(), "0.1.4");
        let mut flow = UpdateFlow::new(updater, quiet(), Some("0.1.3".to_owned()));
        wait(&mut flow, settled);
        assert_eq!(
            flow.notice(),
            Some(Notice::Updated("Updated to Clone Bay 0.1.4.".to_owned()))
        );
        flow.dismiss();
        assert_eq!(flow.notice(), None);
    }

    #[test]
    fn the_download_page_can_be_opened_and_a_failure_can_be_dismissed() {
        let dir = tempfile::tempdir().unwrap();
        let (updater, _, log) = fake_updater(dir.path(), "0.1.5");
        *log.self_test_fails.lock().unwrap() = true;
        let mut flow = UpdateFlow::new(updater, quiet(), None);
        wait(&mut flow, offered);
        flow.open_page();
        assert_eq!(*log.pages_opened.lock().unwrap(), 1);
        flow.start_update();
        wait(&mut flow, |flow| {
            matches!(flow.notice(), Some(Notice::Failed(_)))
        });
        flow.dismiss();
        assert_eq!(flow.notice(), None);
    }

    #[test]
    fn sizes_show_as_megabytes_with_one_decimal() {
        assert_eq!(megabytes(0), "0.0 MB");
        assert_eq!(megabytes(1024 * 1024), "1.0 MB");
        assert_eq!(megabytes(7_655_424), "7.3 MB");
    }
}
