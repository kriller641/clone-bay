//! The steps of copying one file's settings onto others: pick targets,
//! review what will happen, copy, see the result. No drawing happens here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use clone_bay_core::copy::{
    execute_copy, plan_copy_scoped, source_chat_channels, CopyPlan, CopyReport, ExecuteError,
};
use clone_bay_core::copy_choices::{CopyChoices, ScopeChoice};
use clone_bay_core::discovery::SettingsFileEntry;
use clone_bay_core::groups::{groups_for, has_chat_channels, Channel, Group, Scope, Selection};
use clone_bay_core::names::NameBook;

use crate::list_model::{compare_entries, display_name, matches_search, Row, Sort, Status};
use crate::services::Services;
use crate::state::Wake;

/// A file the settings could be copied onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub entry: SettingsFileEntry,
    /// The character's name or the account's label, when there is one.
    pub name: Option<String>,
    /// Why it can't be chosen, when it is already known to be unusable.
    pub unusable: Option<String>,
    pub ticked: bool,
}

/// The chat channels of the source character that could be added to the
/// targets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatPicker {
    /// Whether chat channels are being copied at all.
    pub ticked: bool,
    /// Each channel that can be copied, and whether it is chosen.
    pub channels: Vec<(Channel, bool)>,
    /// Why the source's channels couldn't be read, if they couldn't.
    pub problem: Option<String>,
}

/// What the user wants copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePicker {
    /// Copy everything, as opposed to only what is ticked below.
    pub everything: bool,
    pub groups: Vec<(Group, bool)>,
    /// Settings that belong to no group.
    pub everything_else: bool,
    /// `None` for account files, which have no chat channels.
    pub chat: Option<ChatPicker>,
}

impl ScopePicker {
    /// Starts from `remembered`, what the user last chose for this kind of
    /// file. The first time, it starts at "everything". Should the user
    /// switch to "only what I tick", every group, "everything else" and (when
    /// the source has channels to offer) chat channels start ticked, so they
    /// untick what they don't want instead of building the choice up from
    /// nothing. Chat channels themselves always start ticked.
    fn new(source: &SettingsFileEntry, remembered: Option<&ScopeChoice>) -> Self {
        let chat =
            has_chat_channels(source.kind).then(|| match source_chat_channels(&source.path) {
                Ok(channels) => {
                    let channels: Vec<(Channel, bool)> = channels
                        .into_iter()
                        .filter(Channel::can_be_copied)
                        .map(|channel| (channel, true))
                        .collect();
                    ChatPicker {
                        ticked: !channels.is_empty() && remembered.is_none_or(|choice| choice.chat),
                        channels,
                        problem: None,
                    }
                }
                Err(problem) => ChatPicker {
                    problem: Some(problem),
                    ..ChatPicker::default()
                },
            });
        Self {
            everything: remembered.is_none_or(|choice| choice.everything),
            groups: groups_for(source.kind)
                .iter()
                .map(|group| {
                    let ticked = remembered.is_none_or(|choice| choice.groups.contains(group.id));
                    (*group, ticked)
                })
                .collect(),
            everything_else: remembered.is_none_or(|choice| choice.everything_else),
            chat,
        }
    }

    /// The choice to remember. A character with no channels to offer says
    /// nothing about chat, so what was remembered from `before` stays.
    fn choice(&self, before: Option<&ScopeChoice>) -> ScopeChoice {
        let offered = self
            .chat
            .as_ref()
            .filter(|chat| !chat.channels.is_empty())
            .map(|chat| chat.ticked);
        ScopeChoice {
            everything: self.everything,
            groups: self
                .groups
                .iter()
                .filter(|(_, ticked)| *ticked)
                .map(|(group, _)| group.id.to_owned())
                .collect(),
            everything_else: self.everything_else,
            chat: offered.or(before.map(|choice| choice.chat)).unwrap_or(true),
        }
    }

    /// The choice as the core understands it.
    pub fn scope(&self) -> Scope {
        if self.everything {
            return Scope::Everything;
        }
        Scope::Selected(Selection {
            groups: self
                .groups
                .iter()
                .filter(|(_, ticked)| *ticked)
                .map(|(group, _)| group.id.to_owned())
                .collect(),
            everything_else: self.everything_else,
            chat_channels: self.chat.as_ref().filter(|chat| chat.ticked).map(|chat| {
                chat.channels
                    .iter()
                    .filter(|(_, ticked)| *ticked)
                    .map(|(channel, _)| channel.id.clone())
                    .collect()
            }),
        })
    }

    /// Whether anything at all is chosen.
    pub fn chooses_something(&self) -> bool {
        match self.scope() {
            Scope::Everything => true,
            Scope::Selected(selection) => !selection.is_empty(),
        }
    }

    /// What is being copied, in a few words, for the review screen.
    pub fn summary(&self) -> String {
        if self.everything {
            return "everything".to_owned();
        }
        let mut names: Vec<String> = self
            .groups
            .iter()
            .filter(|(_, ticked)| *ticked)
            .map(|(group, _)| group.label.to_lowercase())
            .collect();
        if let Some(chat) = self.chat.as_ref().filter(|chat| chat.ticked) {
            let chosen = chat.channels.iter().filter(|(_, ticked)| *ticked).count();
            if chosen > 0 {
                names.push(match chosen {
                    1 => "1 chat channel".to_owned(),
                    n => format!("{n} chat channels"),
                });
            }
        }
        if self.everything_else {
            names.push("everything else".to_owned());
        }
        names.join(", ")
    }
}

pub enum Stage {
    /// Choosing which files to copy onto.
    Picking,
    /// Examining the chosen files. Nothing is changed.
    Planning,
    /// Showing what would happen, waiting for the go-ahead.
    Reviewing(Box<CopyPlan>),
    /// Writing.
    Copying,
    /// Finished: the report, or why nothing was copied.
    Done(Result<CopyReport, String>),
}

impl Stage {
    /// Which of the three steps this is: 1 choosing, 2 reviewing, 3 done.
    pub fn step(&self) -> usize {
        match self {
            Self::Picking | Self::Planning => 1,
            Self::Reviewing(_) | Self::Copying => 2,
            Self::Done(_) => 3,
        }
    }
}

enum Finished {
    Planned(Result<CopyPlan, String>),
    Copied(Box<CopyPlan>, Result<CopyReport, ExecuteError>),
}

pub struct CopyFlow {
    pub source: SettingsFileEntry,
    pub source_name: Option<String>,
    pub candidates: Vec<Candidate>,
    /// What to copy.
    pub scope: ScopePicker,
    /// What was last chosen to copy, as saved.
    choices: CopyChoices,
    /// What the screen started with, so an untouched choice isn't saved.
    opening_choice: ScopeChoice,
    /// Why the choice couldn't be remembered, if it couldn't.
    pub choice_problem: Option<String>,
    pub search: String,
    /// The order the files to copy onto are listed in.
    pub sort: Sort,
    /// The row last clicked without Shift: where a Shift-click's range
    /// starts from.
    anchor: Option<usize>,
    pub stage: Stage,
    /// Why the last attempt to copy was refused, shown on the review screen.
    pub refusal: Option<String>,
    /// Whether the review lists every file, or only the ones that will be
    /// skipped.
    pub show_every_file: bool,
    /// Set when it couldn't be checked whether EVE is running, so the user
    /// has to vouch for it.
    pub needs_eve_confirmation: bool,
    pub eve_closed_confirmed: bool,
    progress: Arc<AtomicUsize>,
    progress_total: usize,
    pending: Option<Receiver<Finished>>,
    services: Services,
    wake: Wake,
}

impl CopyFlow {
    /// Starts a copy from `source`. Every other file of the same kind in
    /// `rows` can be chosen as a target.
    pub fn new(
        source: &SettingsFileEntry,
        rows: &[Row],
        names: &NameBook,
        services: Services,
        wake: Wake,
    ) -> Self {
        let name_of = |entry: &SettingsFileEntry| {
            display_name(names, entry.kind, entry.id).map(str::to_owned)
        };
        let candidates = rows
            .iter()
            .filter(|row| row.entry.kind == source.kind && row.entry.path != source.path)
            .map(|row| Candidate {
                entry: row.entry.clone(),
                name: name_of(&row.entry),
                unusable: match &row.status {
                    Status::Unreadable(error) => Some(error.to_string()),
                    Status::Pending | Status::Readable(_) => None,
                },
                ticked: false,
            })
            .collect();
        let (choices, choice_problem) = match &services.copy_choices_file {
            Some(file) => match CopyChoices::load(file) {
                Ok(choices) => (choices, None),
                Err(error) => (
                    CopyChoices::default(),
                    Some(format!(
                        "couldn't read your last choice of what to copy ({error})"
                    )),
                ),
            },
            None => (CopyChoices::default(), None),
        };
        let remembered = choices.get(source.kind);
        let scope = ScopePicker::new(source, remembered);
        let opening_choice = scope.choice(remembered);
        Self {
            source: source.clone(),
            source_name: name_of(source),
            candidates,
            scope,
            choices,
            opening_choice,
            choice_problem,
            search: String::new(),
            sort: Sort::default(),
            anchor: None,
            stage: Stage::Picking,
            refusal: None,
            show_every_file: false,
            needs_eve_confirmation: false,
            eve_closed_confirmed: false,
            progress: Arc::new(AtomicUsize::new(0)),
            progress_total: 0,
            pending: None,
            services,
            wake,
        }
    }

    /// Indices of the candidates matching the search, in the order chosen
    /// by `sort`.
    pub fn visible(&self) -> Vec<usize> {
        let mut visible: Vec<usize> = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                matches_search(&self.search, candidate.entry.id, candidate.name.as_deref())
            })
            .map(|(index, _)| index)
            .collect();
        visible.sort_by(|&a, &b| {
            let (a, b) = (&self.candidates[a], &self.candidates[b]);
            compare_entries(
                (&a.entry, a.name.as_deref()),
                (&b.entry, b.name.as_deref()),
                self.sort,
            )
        });
        visible
    }

    /// Ticks the candidate at `index` if it isn't ticked, and unticks it if
    /// it is. A file that can't be used stays unticked.
    pub fn toggle(&mut self, index: usize) {
        if let Some(candidate) = self.candidates.get_mut(index) {
            if candidate.unusable.is_none() {
                candidate.ticked = !candidate.ticked;
            }
        }
    }

    /// A click on a row of the list.
    ///
    /// A plain click ticks or unticks that file and remembers it. A click
    /// with Shift held gives every usable file between the remembered one
    /// and this one, as the list is currently shown, the remembered file's
    /// tick. With nothing remembered, or the remembered file searched out
    /// of view, a Shift-click is a plain click.
    pub fn click_row(&mut self, index: usize, shift: bool) {
        if !self
            .candidates
            .get(index)
            .is_some_and(|candidate| candidate.unusable.is_none())
        {
            return;
        }
        let visible = self.visible();
        let position = |wanted: usize| visible.iter().position(|&shown| shown == wanted);
        let range = match (shift, self.anchor.and_then(position), position(index)) {
            (true, Some(from), Some(to)) => Some((from.min(to), from.max(to))),
            _ => None,
        };
        match (range, self.anchor) {
            (Some((first, last)), Some(anchor)) => {
                let ticked = self.candidates[anchor].ticked;
                for &shown in &visible[first..=last] {
                    let candidate = &mut self.candidates[shown];
                    if candidate.unusable.is_none() {
                        candidate.ticked = ticked;
                    }
                }
            }
            _ => self.toggle(index),
        }
        self.anchor = Some(index);
    }

    /// The name of the target at `path`, when it has one.
    pub fn name_of(&self, path: &Path) -> Option<&str> {
        self.candidates
            .iter()
            .find(|candidate| candidate.entry.path == path)
            .and_then(|candidate| candidate.name.as_deref())
    }

    /// Ticks or unticks every usable candidate matching the search.
    pub fn tick_visible(&mut self, ticked: bool) {
        for index in self.visible() {
            let candidate = &mut self.candidates[index];
            if candidate.unusable.is_none() {
                candidate.ticked = ticked;
            }
        }
    }

    pub fn ticked(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.ticked)
            .count()
    }

    /// Whether "Check files" may be pressed: some file is ticked and
    /// something is chosen to copy.
    pub fn can_check(&self) -> bool {
        self.ticked() > 0 && self.scope.chooses_something()
    }

    /// Saves what is chosen now, for the next copy of this kind of file.
    /// A choice the user didn't change isn't saved.
    pub fn remember_choice(&mut self) {
        let kind = self.source.kind;
        let choice = self.scope.choice(self.choices.get(kind));
        if choice == self.opening_choice {
            return;
        }
        self.opening_choice = choice.clone();
        self.choices.set(kind, choice);
        if let Some(file) = &self.services.copy_choices_file {
            self.choice_problem = self.choices.save(file).err().map(|error| {
                format!(
                    "couldn't remember your choice of what to copy ({error}). No settings file \
                     was affected"
                )
            });
        }
    }

    /// How far the current background step has got: `(done, total)`.
    pub fn progress(&self) -> (usize, usize) {
        (self.progress.load(Ordering::SeqCst), self.progress_total)
    }

    fn begin(&mut self, total: usize) -> (Arc<AtomicUsize>, Wake) {
        self.progress = Arc::new(AtomicUsize::new(0));
        self.progress_total = total;
        (Arc::clone(&self.progress), Arc::clone(&self.wake))
    }

    /// Examines the ticked files in the background. Nothing is changed.
    pub fn start_plan(&mut self) {
        let targets: Vec<PathBuf> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.ticked)
            .map(|candidate| candidate.entry.path.clone())
            .collect();
        if targets.is_empty() || !matches!(self.stage, Stage::Picking) {
            return;
        }
        if !self.scope.chooses_something() {
            return;
        }
        self.remember_choice();
        let scope = self.scope.scope();
        let (progress, wake) = self.begin(targets.len());
        let source = self.source.path.clone();
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.stage = Stage::Planning;
        self.refusal = None;
        thread::spawn(move || {
            let report = |done: usize| {
                progress.store(done, Ordering::SeqCst);
                wake();
            };
            let plan = plan_copy_scoped(&source, &targets, scope, &report)
                .map_err(|error| error.to_string());
            // If the window has gone there is nobody to tell.
            let _ = sender.send(Finished::Planned(plan));
            wake();
        });
    }

    /// Carries out the reviewed plan in the background.
    pub fn start_copy(&mut self) {
        if !self.can_copy() {
            return;
        }
        let Stage::Reviewing(plan) = std::mem::replace(&mut self.stage, Stage::Copying) else {
            return;
        };
        let (progress, wake) = self.begin(plan.writable());
        let services = self.services.clone();
        let confirmed = self.eve_closed_confirmed;
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.refusal = None;
        thread::spawn(move || {
            let report = |done: usize| {
                progress.store(done, Ordering::SeqCst);
                wake();
            };
            let eve = (services.eve_status)();
            let result = execute_copy(&plan, eve, confirmed, &services.backups, &report);
            let _ = sender.send(Finished::Copied(plan, result));
            wake();
        });
    }

    /// Whether "Copy now" may be pressed.
    pub fn can_copy(&self) -> bool {
        match &self.stage {
            Stage::Reviewing(plan) => {
                plan.writable() > 0 && (!self.needs_eve_confirmation || self.eve_closed_confirmed)
            }
            _ => false,
        }
    }

    /// From the review screen back to choosing files.
    pub fn back_to_picking(&mut self) {
        if matches!(self.stage, Stage::Reviewing(_)) {
            self.stage = Stage::Picking;
            self.refusal = None;
            self.needs_eve_confirmation = false;
            self.eve_closed_confirmed = false;
        }
    }

    /// Whether files may have been changed, so the file list is out of date.
    pub fn changed_files(&self) -> bool {
        matches!(self.stage, Stage::Done(_))
    }

    /// Whether a background step is running, during which the flow can't be
    /// closed.
    pub fn busy(&self) -> bool {
        matches!(self.stage, Stage::Planning | Stage::Copying)
    }

    /// Applies the result of a background step, if it has finished.
    pub fn poll(&mut self) {
        let Some(finished) = self
            .pending
            .as_ref()
            .and_then(|pending| pending.try_recv().ok())
        else {
            return;
        };
        self.pending = None;
        match finished {
            Finished::Planned(Ok(plan)) => self.stage = Stage::Reviewing(Box::new(plan)),
            Finished::Planned(Err(reason)) => self.stage = Stage::Done(Err(reason)),
            Finished::Copied(_, Ok(report)) => self.stage = Stage::Done(Ok(report)),
            // EVE is running, or might be: nothing was touched and the plan
            // still stands, so the user can deal with it and try again.
            Finished::Copied(plan, Err(error @ ExecuteError::EveRunning)) => {
                self.refusal = Some(error.to_string());
                self.stage = Stage::Reviewing(plan);
            }
            Finished::Copied(plan, Err(error @ ExecuteError::EveUnknown)) => {
                self.refusal = Some(error.to_string());
                self.needs_eve_confirmation = true;
                self.stage = Stage::Reviewing(plan);
            }
            Finished::Copied(_, Err(error)) => self.stage = Stage::Done(Err(error.to_string())),
        }
    }
}
