//! Saving one file in the background, shared by the edit, formation and tab
//! order screens: whether a save is running, where the last one put the
//! backup, and the "I have closed every EVE client" step. No drawing happens
//! here.

use std::fmt::Display;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use clone_bay_core::edit::EditError;
use clone_bay_core::formations::FormationError;
use clone_bay_core::platform::EveStatus;
use clone_bay_core::restore::RestoreError;
use clone_bay_core::safe_write::BackupSession;
use clone_bay_core::tab_order::TabOrderError;

use crate::services::Services;
use crate::state::Wake;

/// Why a save was refused, as each screen's core module reports it.
pub trait SaveError: Display + Send + 'static {
    /// Whether it was refused because nobody could tell if EVE is running,
    /// so the user has to vouch that it is closed.
    fn eve_unknown(&self) -> bool;
}

impl SaveError for EditError {
    fn eve_unknown(&self) -> bool {
        *self == EditError::EveUnknown
    }
}

impl SaveError for FormationError {
    fn eve_unknown(&self) -> bool {
        *self == FormationError::Edit(EditError::EveUnknown)
    }
}

impl SaveError for RestoreError {
    fn eve_unknown(&self) -> bool {
        *self == RestoreError::EveUnknown
    }
}

impl SaveError for TabOrderError {
    fn eve_unknown(&self) -> bool {
        *self == TabOrderError::Edit(EditError::EveUnknown)
    }
}

/// What a save is given that the screen doesn't choose: whether EVE is
/// running, whether the user vouched that it is closed, and the run's
/// backups.
pub struct SaveContext<'a> {
    pub eve: EveStatus,
    pub user_confirmed_eve_closed: bool,
    pub backups: &'a BackupSession,
}

pub struct Saving<E> {
    /// Where the original went after the last successful save.
    pub saved_to: Option<PathBuf>,
    /// Set when it couldn't be checked whether EVE is running, so the user
    /// has to vouch for it.
    pub needs_eve_confirmation: bool,
    pub eve_closed_confirmed: bool,
    /// Whether any save succeeded, so the file list is out of date.
    changed_file: bool,
    pending: Option<Receiver<Result<PathBuf, E>>>,
    services: Services,
    wake: Wake,
}

impl<E: SaveError> Saving<E> {
    pub fn new(services: Services, wake: Wake) -> Self {
        Self {
            saved_to: None,
            needs_eve_confirmation: false,
            eve_closed_confirmed: false,
            changed_file: false,
            pending: None,
            services,
            wake,
        }
    }

    /// Whether a save is running, during which the screen can't be left.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a save may start, as far as saving goes: none is running,
    /// and the user has vouched that EVE is closed if that was asked.
    pub fn ready(&self) -> bool {
        !self.busy() && (!self.needs_eve_confirmation || self.eve_closed_confirmed)
    }

    /// Whether a save has changed the file since the screen was opened.
    pub fn changed_file(&self) -> bool {
        self.changed_file
    }

    /// Runs `save` in the background. Does nothing unless [`ready`](Self::ready).
    pub fn start(
        &mut self,
        save: impl FnOnce(SaveContext<'_>) -> Result<PathBuf, E> + Send + 'static,
    ) {
        if !self.ready() {
            return;
        }
        let services = self.services.clone();
        let confirmed = self.eve_closed_confirmed;
        let wake = Arc::clone(&self.wake);
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.saved_to = None;
        thread::spawn(move || {
            let result = save(SaveContext {
                eve: (services.eve_status)(),
                user_confirmed_eve_closed: confirmed,
                backups: &services.backups,
            });
            // If the window has gone there is nobody to tell.
            let _ = sender.send(result);
            wake();
        });
    }

    /// The result of the save, once it has finished. A success is recorded
    /// here; the screen should then read the file again. A refusal is handed
    /// back for the screen to show.
    pub fn poll(&mut self) -> Option<Result<(), E>> {
        let result = self.pending.as_ref()?.try_recv().ok()?;
        self.pending = None;
        Some(match result {
            Ok(backup) => {
                self.saved_to = Some(backup);
                self.changed_file = true;
                self.needs_eve_confirmation = false;
                self.eve_closed_confirmed = false;
                Ok(())
            }
            Err(error) => {
                if error.eve_unknown() {
                    self.needs_eve_confirmation = true;
                }
                Err(error)
            }
        })
    }
}
