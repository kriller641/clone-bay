//! Copying one settings file onto others of the same kind, with each target
//! keeping its own protected data.
//!
//! The work is split in two so the user can review before anything changes:
//! [`plan_copy`] examines every file and changes nothing; [`execute_copy`]
//! backs up, re-checks and writes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use clone_bay_marshal::{encode, Decoded, Value};
use thiserror::Error;

use crate::discovery::{classify, file_name, FileKind};
use crate::groups::{
    build_selected, channels_added, chat_channels, same_settings, verify_selected, Channel,
    GroupError, Scope, Selection,
};
use crate::inspect::{decode_settings, read_settings_bytes, InspectError};
use crate::platform::EveStatus;
use crate::protect::{
    build_protected_copy, describe, protected_parts, verify_output, ProtectError, PROTECTED,
};
use crate::safe_write::{BackupSession, Fingerprint, WriteError};
use crate::tab_order::opening_on_first_tab;

/// Why nothing can be copied from the chosen source.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error("{0} isn't a settings file")]
    NotASettingsFile(String),
    #[error("can't use {file} as the source: {reason}")]
    SourceUnusable { file: String, reason: String },
    #[error("nothing was chosen to copy")]
    NothingChosen,
}

/// What will happen to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Neither file holds protected data, so the source is copied byte for
    /// byte.
    ExactCopy,
    /// The source's settings are written with the target's protected data
    /// kept. `kept` describes that data; it is empty when the target has
    /// none, in which case it ends up with none.
    KeepOwn { kept: Vec<String> },
    /// Only the chosen groups of settings are written; everything else in
    /// the target stays as it is. `channels_added` is how many chat channels
    /// the target gains.
    Chosen { channels_added: usize },
    /// The target is left alone.
    Skip { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPlan {
    pub path: PathBuf,
    pub action: Action,
    /// The target's contents when it was examined. `None` when skipped.
    fingerprint: Option<Fingerprint>,
}

/// The reviewed plan for a copy. Holds the source's contents, so a later
/// change to the source file can't alter what gets written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyPlan {
    pub source: PathBuf,
    pub kind: FileKind,
    pub targets: Vec<TargetPlan>,
    /// How much of the source is being copied.
    pub scope: Scope,
    source_bytes: Vec<u8>,
    /// What is copied: the source's settings, with the overview set to open
    /// on its first tab.
    source_root: Value,
    source_has_protected: bool,
    /// Whether `source_root` differs from the source file in that one way.
    source_on_another_tab: bool,
}

/// The group that holds which tab each tabbed window has selected.
const TAB_GROUPS_GROUP: &str = "tabgroups";

impl CopyPlan {
    /// Whether this copy makes its targets open the overview on the first
    /// tab where the source would have had them open on another.
    pub fn resets_overview_tab(&self) -> bool {
        self.source_on_another_tab
            && match &self.scope {
                Scope::Everything => true,
                Scope::Selected(selection) => selection.groups.contains(TAB_GROUPS_GROUP),
            }
    }

    /// How many targets will be written.
    pub fn writable(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| !matches!(target.action, Action::Skip { .. }))
            .count()
    }
}

fn kind_of(path: &Path) -> Option<FileKind> {
    classify(&file_name(path)).map(|(kind, _)| kind)
}

/// More threads than this don't help: disk reads start to dominate.
const MAX_PLAN_THREADS: usize = 8;

/// Examines the source and every target and works out what a copy would do.
/// Nothing is changed. `progress` is called with the number of targets
/// examined so far; targets are examined on several threads.
pub fn plan_copy(
    source: &Path,
    targets: &[PathBuf],
    progress: &(dyn Fn(usize) + Sync),
) -> Result<CopyPlan, PlanError> {
    plan_copy_scoped(source, targets, Scope::Everything, progress)
}

/// [`plan_copy`] for a chosen part of the source's settings.
pub fn plan_copy_scoped(
    source: &Path,
    targets: &[PathBuf],
    scope: Scope,
    progress: &(dyn Fn(usize) + Sync),
) -> Result<CopyPlan, PlanError> {
    if matches!(&scope, Scope::Selected(selection) if selection.is_empty()) {
        return Err(PlanError::NothingChosen);
    }
    let kind = kind_of(source).ok_or_else(|| PlanError::NotASettingsFile(file_name(source)))?;
    let unusable = |reason: String| PlanError::SourceUnusable {
        file: file_name(source),
        reason,
    };
    let source_bytes = read_settings_bytes(source).map_err(|e| unusable(e.to_string()))?;
    let source_root = decode_settings(&source_bytes)
        .map_err(|e| unusable(e.to_string()))?
        .value;
    let source_has_protected = !protected_parts(&source_root, PROTECTED)
        .map_err(|e| unusable(e.to_string()))?
        .is_empty();
    // All overview windows share one "selected tab" setting. Copied as it
    // is, a source last closed on its second tab makes every target open on
    // its second tab, so a copy always carries "the first tab" instead.
    let first_tab = match kind {
        FileKind::Account => opening_on_first_tab(&source_root),
        FileKind::Character => None,
    };
    let source_on_another_tab = first_tab.is_some();
    let source_root = first_tab.unwrap_or(source_root);

    let mut plan = CopyPlan {
        source: source.to_path_buf(),
        kind,
        targets: Vec::new(),
        scope,
        source_bytes,
        source_root,
        source_has_protected,
        source_on_another_tab,
    };
    plan.targets = plan.plan_targets(targets, progress);
    Ok(plan)
}

impl CopyPlan {
    /// Plans every target, in the order given, sharing the work between
    /// threads.
    fn plan_targets(
        &self,
        targets: &[PathBuf],
        progress: &(dyn Fn(usize) + Sync),
    ) -> Vec<TargetPlan> {
        let threads = thread::available_parallelism()
            .map_or(1, usize::from)
            .min(MAX_PLAN_THREADS)
            .min(targets.len());
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let mut planned: Vec<(usize, TargetPlan)> = thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut mine = Vec::new();
                        loop {
                            let index = next.fetch_add(1, Ordering::SeqCst);
                            let Some(path) = targets.get(index) else {
                                break;
                            };
                            mine.push((index, self.plan_target(path)));
                            progress(done.fetch_add(1, Ordering::SeqCst) + 1);
                        }
                        mine
                    })
                })
                .collect();
            workers
                .into_iter()
                // A worker can only fail by panicking, which the lints forbid
                // in this crate. Its targets would be reported as skipped below.
                .flat_map(|worker| worker.join().unwrap_or_default())
                .collect()
        });
        planned.sort_by_key(|(index, _)| *index);
        let mut planned = planned.into_iter().peekable();
        targets
            .iter()
            .enumerate()
            .map(
                |(index, path)| match planned.next_if(|(found, _)| *found == index) {
                    Some((_, target)) => target,
                    None => TargetPlan {
                        path: path.clone(),
                        action: Action::Skip {
                            reason: "it couldn't be examined".to_owned(),
                        },
                        fingerprint: None,
                    },
                },
            )
            .collect()
    }
}

impl CopyPlan {
    fn plan_target(&self, path: &Path) -> TargetPlan {
        let skip = |reason: String| TargetPlan {
            path: path.to_path_buf(),
            action: Action::Skip { reason },
            fingerprint: None,
        };
        if same_file(path, &self.source) {
            return skip("it is the file being copied from".to_owned());
        }
        if kind_of(path) != Some(self.kind) {
            return skip("it isn't the same kind of settings file as the source".to_owned());
        }
        match fs::metadata(path) {
            Ok(metadata) if metadata.permissions().readonly() => {
                return skip("the file is read-only".to_owned());
            }
            Ok(_) => {}
            Err(error) => return skip(InspectError::Read(error.to_string()).to_string()),
        }
        let bytes = match read_settings_bytes(path) {
            Ok(bytes) => bytes,
            Err(error) => return skip(error.to_string()),
        };
        match self.output_for(&bytes) {
            Ok((action, _)) => TargetPlan {
                path: path.to_path_buf(),
                action,
                fingerprint: Some(Fingerprint::of(&bytes)),
            },
            Err(reason) => skip(reason),
        }
    }

    /// What to write over a target that currently holds `target_bytes`.
    /// The result has already passed the self-check.
    fn output_for(&self, target_bytes: &[u8]) -> Result<(Action, Vec<u8>), String> {
        let target = decode_settings(target_bytes).map_err(|e| e.to_string())?;
        if let Scope::Selected(selection) = &self.scope {
            return self.chosen_output_for(&target, selection);
        }
        let explain = |error: ProtectError| error.to_string();
        let parts = protected_parts(&target.value, PROTECTED).map_err(explain)?;
        // The source's own bytes will only do when nothing in them has to
        // differ in the target.
        if !self.source_has_protected && parts.is_empty() && !self.source_on_another_tab {
            return Ok((Action::ExactCopy, self.source_bytes.clone()));
        }
        let built =
            build_protected_copy(&self.source_root, &target.value, PROTECTED).map_err(explain)?;
        let data = encode(&built, target.had_checksum).map_err(|e| e.to_string())?;
        verify_output(&data, &self.source_root, &target.value, PROTECTED).map_err(explain)?;
        let kept = describe(&parts);
        Ok((Action::KeepOwn { kept }, data))
    }
}

impl CopyPlan {
    /// What to write over a target when only some groups were chosen.
    fn chosen_output_for(
        &self,
        target: &Decoded,
        selection: &Selection,
    ) -> Result<(Action, Vec<u8>), String> {
        let explain = |error: GroupError| error.to_string();
        let (source, kind) = (&self.source_root, self.kind);
        let built = build_selected(source, &target.value, kind, selection).map_err(explain)?;
        if same_settings(&built, &target.value, kind).map_err(explain)? {
            return Err("it already has these settings".to_owned());
        }
        let data = encode(&built, target.had_checksum).map_err(|e| e.to_string())?;
        verify_selected(&data, source, &target.value, kind, selection).map_err(explain)?;
        let channels_added = channels_added(&target.value, &built);
        Ok((Action::Chosen { channels_added }, data))
    }
}

/// The chat channels stored in a character file, for choosing which to
/// copy. Nothing is changed.
pub fn source_chat_channels(source: &Path) -> Result<Vec<Channel>, String> {
    let bytes = read_settings_bytes(source).map_err(|e| e.to_string())?;
    let root = decode_settings(&bytes).map_err(|e| e.to_string())?.value;
    chat_channels(&root).map_err(|e| e.to_string())
}

/// Whether two paths name the same file, as far as can be told.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Why a copy didn't start. In every case nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExecuteError {
    #[error(
        "EVE is running. Close every EVE client first: the client saves its settings when it \
         closes and would overwrite the result. Nothing was changed"
    )]
    EveRunning,
    #[error(
        "Couldn't check whether EVE is running. Confirm that every EVE client is closed, then \
         try again. Nothing was changed"
    )]
    EveUnknown,
    #[error("None of the chosen files can be written. Nothing was changed")]
    NothingToWrite,
    #[error("Backup failed: {0}. Nothing was changed")]
    Backup(WriteError),
}

/// What happened to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Written,
    /// Left alone, as planned.
    Skipped(String),
    /// Should have been written but wasn't, or was written wrongly.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyReport {
    /// The folder holding the originals of every written target.
    pub backup_dir: PathBuf,
    /// One entry per planned target, in plan order.
    pub outcomes: Vec<(PathBuf, Outcome)>,
}

impl CopyReport {
    pub fn written(&self) -> usize {
        self.count(|outcome| matches!(outcome, Outcome::Written))
    }

    pub fn failed(&self) -> usize {
        self.count(|outcome| matches!(outcome, Outcome::Failed(_)))
    }

    fn count(&self, want: fn(&Outcome) -> bool) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| want(outcome))
            .count()
    }
}

/// Carries out a reviewed plan.
///
/// Refuses if EVE is running. If that couldn't be checked, it only proceeds
/// when `user_confirmed_eve_closed` is set. `backups` makes sure every
/// target has a verified backup before the first write. `progress` is called
/// with the number of files written or attempted so far.
pub fn execute_copy(
    plan: &CopyPlan,
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
    progress: &dyn Fn(usize),
) -> Result<CopyReport, ExecuteError> {
    match eve {
        EveStatus::Running => return Err(ExecuteError::EveRunning),
        EveStatus::Unknown if !user_confirmed_eve_closed => return Err(ExecuteError::EveUnknown),
        EveStatus::Unknown | EveStatus::NotRunning => {}
    }
    let todo: Vec<(&TargetPlan, Fingerprint)> = plan
        .targets
        .iter()
        .filter_map(|target| Some((target, target.fingerprint?)))
        .collect();
    if todo.is_empty() {
        return Err(ExecuteError::NothingToWrite);
    }

    let mut folders = Vec::with_capacity(todo.len());
    for (target, fingerprint) in &todo {
        let folder = backups
            .back_up(&target.path, *fingerprint)
            .map_err(ExecuteError::Backup)?;
        folders.push(folder);
    }

    let mut attempted = 0;
    let outcomes = plan
        .targets
        .iter()
        .map(|target| {
            let outcome = match (&target.action, target.fingerprint) {
                (Action::Skip { reason }, _) => Outcome::Skipped(reason.clone()),
                (_, None) => Outcome::Failed("it was never examined".to_owned()),
                (_, Some(fingerprint)) => {
                    attempted += 1;
                    let outcome = plan.write_target(&target.path, fingerprint, backups);
                    progress(attempted);
                    outcome
                }
            };
            (target.path.clone(), outcome)
        })
        .collect();
    Ok(CopyReport {
        backup_dir: holding_all(&folders),
        outcomes,
    })
}

/// The one folder to point the user at: the folder the backups share, or
/// the one above when they are spread over several.
fn holding_all(folders: &[PathBuf]) -> PathBuf {
    match folders {
        [first, rest @ ..] if rest.iter().all(|folder| folder == first) => first.clone(),
        [first, ..] => first.parent().unwrap_or(first).to_path_buf(),
        [] => PathBuf::new(),
    }
}

impl CopyPlan {
    fn write_target(
        &self,
        path: &Path,
        fingerprint: Fingerprint,
        backups: &BackupSession,
    ) -> Outcome {
        let failed = |error: WriteError| Outcome::Failed(error.to_string());
        // Work from the file as it is now. If it differs from what was
        // reviewed, stop before computing anything from it.
        let current = match read_settings_bytes(path) {
            Ok(bytes) => bytes,
            Err(error) => return failed(WriteError::CannotRead(error.to_string())),
        };
        if Fingerprint::of(&current) != fingerprint {
            return failed(WriteError::ChangedSinceCheck);
        }
        // These are the bytes that were reviewed, so this gives the same
        // result the plan showed.
        let data = match self.output_for(&current) {
            Ok((_, data)) => data,
            Err(reason) => return Outcome::Failed(reason),
        };
        match backups.replace_file(path, &data, fingerprint) {
            Ok(()) => Outcome::Written,
            Err(error) => failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_file_is_recognised_however_its_path_is_spelled() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("core_user_1.dat");
        let other = dir.path().join("core_user_2.dat");
        fs::write(&file, b"x").unwrap();
        fs::write(&other, b"x").unwrap();

        assert!(same_file(&file, &file));
        // A spelling that only matches once the path is resolved.
        fs::create_dir(dir.path().join("sub")).unwrap();
        let roundabout = dir.path().join("sub").join("..").join("core_user_1.dat");
        assert_ne!(roundabout, file);
        assert!(same_file(&file, &roundabout));
        assert!(same_file(
            &file,
            &dir.path().join(".").join("core_user_1.dat")
        ));
        assert!(!same_file(&file, &other));
    }

    #[test]
    fn paths_that_do_not_exist_are_compared_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("core_user_8.dat");
        assert!(same_file(&missing, &missing));
        assert!(!same_file(&missing, &dir.path().join("core_user_9.dat")));
        // One exists and one doesn't: they can't be the same file.
        let file = dir.path().join("core_user_1.dat");
        fs::write(&file, b"x").unwrap();
        assert!(!same_file(&file, &missing));
    }
}
