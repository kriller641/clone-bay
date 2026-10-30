//! Editing an account's probe formations: what is shown, what has been
//! changed, and saving. No drawing happens here.

use clone_bay_core::discovery::SettingsFileEntry;
use clone_bay_core::edit::timestamp_now;
use clone_bay_core::formation_share;
use clone_bay_core::formations::{
    open_file, save, Formation, FormationError, Formations, Preset, Probe, View, DEFAULT_RANGE, KM,
    MAX_OFFSET, MAX_PROBES, MIN_DISTANCE,
};
use clone_bay_core::safe_write::Fingerprint;

use crate::saving::Saving;
use crate::services::Services;
use crate::state::Wake;
use crate::viewer::Camera;

/// How many steps back Undo can go.
const HISTORY_LIMIT: usize = 200;
/// Typed or dragged changes closer together than this, in seconds, are one
/// step to undo.
const TYPING_GAP: f64 = 1.0;

/// What the boxes for changing a whole formation hold between clicks. None
/// of it is part of the formations: typing here is not an edit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tools {
    /// How much "Scale" multiplies every position by.
    pub scale_factor: f64,
    /// How many degrees "Turn" turns the formation, clockwise.
    pub turn_degrees: f64,
    /// The side the turn is seen from.
    pub turn_view: View,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            scale_factor: 1.5,
            turn_degrees: 90.0,
            turn_view: View::Above,
        }
    }
}

/// "1 formation" or "3 formations".
fn counted(count: usize) -> String {
    if count == 1 {
        "1 formation".to_owned()
    } else {
        format!("{count} formations")
    }
}

pub struct FormationFlow {
    pub entry: SettingsFileEntry,
    /// The account's label, when it has one.
    pub name: Option<String>,
    /// The formations as edited so far.
    pub formations: Formations,
    /// The formations as the file holds them.
    saved: Formations,
    /// Earlier states of `formations`, oldest first, for Undo.
    undo_stack: Vec<Formations>,
    /// States undone, most recently undone last, for Redo.
    redo_stack: Vec<Formations>,
    /// When the last typed or dragged change was recorded, in seconds, so a
    /// run of them counts as one step to undo.
    last_typing_at: Option<f64>,
    /// The ID of the formation whose probes are being shown.
    pub current: Option<i64>,
    /// Where the picture of the formation is being looked at from.
    pub camera: Camera,
    /// The probe picked out in the picture and the table: the formation's
    /// ID and the probe's place in it.
    pub picked_probe: Option<(i64, usize)>,
    /// What the boxes for changing a whole formation hold.
    pub tools: Tools,
    /// Why the file couldn't be opened, or the last save was refused.
    pub problem: Option<String>,
    /// What the last copy to or paste from the clipboard did.
    pub share_note: Option<String>,
    /// Whether the file could be read. If not, nothing can be edited.
    readable: bool,
    /// What the file held when it was opened or last saved. A save is
    /// refused once it holds anything else.
    opened_as: Option<Fingerprint>,
    pub saving: Saving<FormationError>,
    services: Services,
}

impl FormationFlow {
    /// Opens the probe formations of the account file `entry`.
    pub fn new(
        entry: &SettingsFileEntry,
        name: Option<String>,
        services: Services,
        wake: Wake,
    ) -> Self {
        let mut flow = Self {
            entry: entry.clone(),
            name,
            formations: Formations::default(),
            saved: Formations::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_typing_at: None,
            current: None,
            camera: Camera::default(),
            picked_probe: None,
            tools: Tools::default(),
            problem: None,
            share_note: None,
            readable: false,
            opened_as: None,
            saving: Saving::new(services.clone(), wake),
            services,
        };
        flow.reload();
        flow
    }

    /// Reads the file again, dropping any unsaved changes.
    fn reload(&mut self) {
        match open_file(&self.entry.path) {
            Ok((formations, opened_as)) => {
                self.saved = formations.clone();
                self.formations = formations;
                self.readable = true;
                self.opened_as = Some(opened_as);
            }
            Err(error) => {
                self.saved = Formations::default();
                self.formations = Formations::default();
                self.readable = false;
                self.opened_as = None;
                self.problem = Some(error.to_string());
            }
        }
        // Keep showing the same formation if it still exists.
        let still_there = self
            .current
            .is_some_and(|id| self.formations.get(id).is_some());
        if !still_there {
            self.current = self
                .formations
                .selected
                .filter(|id| self.formations.get(*id).is_some())
                .or_else(|| self.formations.list().first().map(|formation| formation.id));
        }
    }

    /// Whether the file's formations could be read, so they can be edited.
    pub fn readable(&self) -> bool {
        self.readable
    }

    pub fn is_dirty(&self) -> bool {
        self.formations != self.saved
    }

    /// Why the formations can't be saved as they are, if they can't.
    pub fn invalid(&self) -> Option<String> {
        self.formations
            .validate()
            .err()
            .map(|error| error.to_string())
    }

    pub fn busy(&self) -> bool {
        self.saving.busy()
    }

    /// Whether "Save changes" may be pressed.
    pub fn can_save(&self) -> bool {
        self.readable && self.is_dirty() && self.invalid().is_none() && self.saving.ready()
    }

    pub fn changed_file(&self) -> bool {
        self.saving.changed_file()
    }

    /// A name no formation is using, starting from `wanted`.
    fn free_name(&self, wanted: &str) -> String {
        let taken = |name: &str| self.formations.list().iter().any(|f| f.name == name);
        if !taken(wanted) {
            return wanted.to_owned();
        }
        (2..)
            .map(|n| format!("{wanted} {n}"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| wanted.to_owned())
    }

    /// Adds a formation made from a preset and shows it.
    pub fn add_preset(&mut self, preset: Preset) {
        if !self.readable {
            return;
        }
        let name = self.free_name(preset.label());
        self.current = Some(self.formations.add(&name, preset.probes()));
    }

    /// Adds a copy of the formation being shown, and shows the copy.
    pub fn duplicate_current(&mut self) {
        let Some(original) = self.current.and_then(|id| self.formations.get(id)) else {
            return;
        };
        let (name, probes) = (format!("{} copy", original.name), original.probes.clone());
        let name = self.free_name(&name);
        self.current = Some(self.formations.add(&name, probes));
    }

    /// Copies the formation being shown to the clipboard, as text.
    pub fn copy_current(&mut self) {
        let Some(formation) = self.current.and_then(|id| self.formations.get(id)) else {
            return;
        };
        let formation = formation.clone();
        self.copy(&[&formation]);
    }

    /// Copies every formation to the clipboard, as text.
    pub fn copy_all(&mut self) {
        let formations = self.formations.clone();
        self.copy(&formations.list());
    }

    /// Copies the formations as they are in the editor now, saved or not.
    fn copy(&mut self, formations: &[&Formation]) {
        self.share_note = None;
        self.problem = None;
        if formations.is_empty() {
            return;
        }
        let text = formation_share::to_text(formations);
        match self.services.clipboard.copy(&text) {
            Ok(()) => {
                self.share_note = Some(format!(
                    "Copied {} to the clipboard",
                    counted(formations.len())
                ));
            }
            Err(error) => self.problem = Some(format!("nothing was copied: {error}")),
        }
    }

    /// Adds the formations in the text on the clipboard. They are new
    /// formations in the editor: nothing is written to the settings file
    /// until the user saves, and nothing already here is replaced. Text
    /// with anything wrong in it adds nothing.
    pub fn paste(&mut self) {
        if !self.readable {
            return;
        }
        self.share_note = None;
        self.problem = None;
        let shared =
            self.services.clipboard.paste().and_then(|text| {
                formation_share::from_text(&text).map_err(|error| error.to_string())
            });
        let shared = match shared {
            Ok(shared) => shared,
            Err(error) => {
                self.problem = Some(format!("nothing was pasted: {error}"));
                return;
            }
        };
        let mut first = None;
        for formation in &shared {
            let name = self.free_name(&formation.name);
            let id = self.formations.add(&name, formation.probes.clone());
            first.get_or_insert(id);
        }
        self.current = first.or(self.current);
        self.share_note = Some(format!(
            "Added {} from the clipboard. Press \"Save changes\" to keep them",
            counted(shared.len())
        ));
    }

    /// Removes the formation being shown, and shows the first one left.
    pub fn delete_current(&mut self) {
        let Some(id) = self.current else {
            return;
        };
        self.formations.remove(id);
        self.current = self.formations.list().first().map(|formation| formation.id);
    }

    /// Adds a probe at the centre of the formation being shown, if there
    /// is room for one.
    pub fn add_probe(&mut self) {
        let Some(formation) = self.current.and_then(|id| self.formations.get_mut(id)) else {
            return;
        };
        if formation.probes.len() < MAX_PROBES {
            // Match the others' range so the new one isn't the odd one out.
            let range_au = formation.probes.last().map_or(DEFAULT_RANGE, |probe| {
                probe.range / clone_bay_core::formations::AU
            });
            formation.probes.push(Probe::at_km(0.0, 0.0, 0.0, range_au));
        }
    }

    /// Puts everything back to what the file holds. This can be undone.
    pub fn discard(&mut self) {
        if !self.busy() {
            let before = self.formations.clone();
            self.problem = None;
            self.reload();
            if self.formations != before {
                self.record_change(before, false, 0.0);
            }
        }
    }

    /// Remembers what the formations were before a change, so it can be
    /// undone. `typing` marks a change made by typing or dragging a number:
    /// a run of those within [`TYPING_GAP`] of each other is one step.
    /// `now` is the time in seconds.
    pub fn record_change(&mut self, before: Formations, typing: bool, now: f64) {
        let continues_typing = typing
            && !self.undo_stack.is_empty()
            && self
                .last_typing_at
                .is_some_and(|last| (now - last).abs() < TYPING_GAP);
        if !continues_typing {
            self.undo_stack.push(before);
            if self.undo_stack.len() > HISTORY_LIMIT {
                self.undo_stack.remove(0);
            }
        }
        self.last_typing_at = typing.then_some(now);
        // A new change means the undone states no longer follow from here.
        self.redo_stack.clear();
    }

    /// The probe picked out in formation `id`, by its place in the table.
    /// A probe picked in another formation, or one that has since been
    /// removed, isn't picked.
    pub fn picked_in(&self, id: i64) -> Option<usize> {
        let (formation, index) = self.picked_probe?;
        let count = self.formations.get(id)?.probes.len();
        (formation == id && index < count).then_some(index)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty() && !self.busy()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty() && !self.busy()
    }

    /// Takes back the last change.
    pub fn undo(&mut self) {
        if !self.can_undo() {
            return;
        }
        if let Some(earlier) = self.undo_stack.pop() {
            let undone = std::mem::replace(&mut self.formations, earlier);
            self.redo_stack.push(undone);
            self.after_history_move();
        }
    }

    /// Puts back the last change that was undone.
    pub fn redo(&mut self) {
        if !self.can_redo() {
            return;
        }
        if let Some(later) = self.redo_stack.pop() {
            let current = std::mem::replace(&mut self.formations, later);
            self.undo_stack.push(current);
            self.after_history_move();
        }
    }

    fn after_history_move(&mut self) {
        self.last_typing_at = None;
        // The formation being shown may not exist in this state.
        let still_there = self
            .current
            .is_some_and(|id| self.formations.get(id).is_some());
        if !still_there {
            self.current = self.formations.list().first().map(|formation| formation.id);
        }
    }

    /// Saves the formations in the background.
    pub fn save(&mut self) {
        if !self.can_save() {
            return;
        }
        let Some(opened_as) = self.opened_as else {
            return;
        };
        let formations = self.formations.clone();
        let path = self.entry.path.clone();
        self.problem = None;
        self.saving.start(move |context| {
            save(
                &path,
                opened_as,
                &formations,
                context.eve,
                context.user_confirmed_eve_closed,
                context.backups,
                &timestamp_now(),
            )
        });
    }

    /// Applies the result of a save, if it has finished.
    pub fn poll(&mut self) {
        match self.saving.poll() {
            Some(Ok(())) => self.reload(),
            Some(Err(error)) => self.problem = Some(error.to_string()),
            None => {}
        }
    }
}

/// How far one click of an arrow beside a coordinate moves the probe.
pub const NUDGE: f64 = 50.0 * KM;

/// `coordinate` moved by `step` metres, or `None` if that is further out
/// than a probe can be placed.
pub fn nudged(coordinate: f64, step: f64) -> Option<f64> {
    let moved = coordinate + step;
    (moved.abs() <= MAX_OFFSET).then_some(moved)
}

/// A plain number as shown in a box: up to three decimal places, without
/// trailing zeros.
pub fn format_number(number: f64) -> String {
    format_km(number)
}

/// Reads a factor to scale a formation by: a number from 0.01 to 100, with
/// an optional "x" after it. Anything else is `None`.
pub fn parse_factor(text: &str) -> Option<f64> {
    let text = text.trim();
    let text = text
        .strip_suffix(['x', 'X', '\u{D7}'])
        .unwrap_or(text)
        .trim_end();
    let factor = plain_number(text)?;
    (0.01..=100.0).contains(&factor).then_some(factor)
}

/// Reads a number of degrees to turn a formation by: from -360 to 360, with
/// an optional degree sign after it. Anything else is `None`.
pub fn parse_degrees(text: &str) -> Option<f64> {
    let text = text.trim();
    let text = text.strip_suffix('\u{B0}').unwrap_or(text).trim_end();
    let degrees = plain_number(text)?;
    (-360.0..=360.0).contains(&degrees).then_some(degrees)
}

/// A number written with digits, at most one leading minus and at most one
/// decimal point. No exponents, "inf" or "nan".
fn plain_number(text: &str) -> Option<f64> {
    let sensible = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.'));
    if !sensible {
        return None;
    }
    text.parse().ok().filter(|number: &f64| number.is_finite())
}

/// A position in kilometres as shown in its box: up to three decimal places,
/// without trailing zeros.
pub fn format_km(km: f64) -> String {
    let text = format!("{km:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        other => other.to_owned(),
    }
}

/// Reads a position typed by the user: a number of kilometres, which may be
/// negative or have a decimal part, no further out than a probe can be
/// placed. Spaces and commas between digits are allowed, as is a trailing
/// "km". Anything else is `None`.
pub fn parse_km(text: &str) -> Option<f64> {
    let text = text.trim();
    let text = text
        .strip_suffix("km")
        .or_else(|| text.strip_suffix("KM"))
        .unwrap_or(text);
    let digits: String = text.chars().filter(|c| !matches!(c, ' ' | ',')).collect();
    let sensible = !digits.is_empty()
        && digits
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.'));
    if !sensible {
        return None;
    }
    let km: f64 = digits.parse().ok()?;
    (km.is_finite() && (km * KM).abs() <= MAX_OFFSET).then_some(km)
}

/// Reads a distance from the centre typed by the user: kilometres, written
/// like a position, from a metre up to as far as a probe can be placed.
/// Nothing nearer is taken, because a probe moved onto the centre would lose
/// the direction it lies in (and "0" is on the way to typing "0.5").
pub fn parse_distance_km(text: &str) -> Option<f64> {
    parse_km(text).filter(|km| km * KM >= MIN_DISTANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_distance_is_at_least_a_metre() {
        assert_eq!(parse_distance_km("250"), Some(250.0));
        assert_eq!(parse_distance_km(" 1,400.5 km "), Some(1400.5));
        assert_eq!(parse_distance_km("0.001"), Some(0.001));
        assert_eq!(parse_distance_km("14959787070"), Some(14_959_787_070.0));
        for text in ["0", "0.0", "0.0009", "-5", "", "abc", "14959787071"] {
            assert_eq!(parse_distance_km(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_scale_factor_is_a_plain_number_from_a_hundredth_to_a_hundred() {
        assert_eq!(parse_factor("2"), Some(2.0));
        assert_eq!(parse_factor(" 1.5 x "), Some(1.5));
        assert_eq!(parse_factor("0.5X"), Some(0.5));
        assert_eq!(parse_factor("3\u{D7}"), Some(3.0));
        assert_eq!(parse_factor("0.01"), Some(0.01));
        assert_eq!(parse_factor("100"), Some(100.0));
        for text in [
            "", "x", "0", "0.009", "100.1", "-2", "1e2", "inf", "nan", "two", "1.2.3", "1 5",
        ] {
            assert_eq!(parse_factor(text), None, "{text:?}");
        }
    }

    #[test]
    fn degrees_are_a_plain_number_within_a_full_turn_either_way() {
        assert_eq!(parse_degrees("90"), Some(90.0));
        assert_eq!(parse_degrees(" -45 "), Some(-45.0));
        assert_eq!(parse_degrees("22.5\u{B0}"), Some(22.5));
        assert_eq!(parse_degrees("0"), Some(0.0));
        assert_eq!(parse_degrees("360"), Some(360.0));
        assert_eq!(parse_degrees("-360"), Some(-360.0));
        for text in [
            "", "-", "360.5", "-361", "1e2", "inf", "nan", "ninety", "--5",
        ] {
            assert_eq!(parse_degrees(text), None, "{text:?}");
        }
    }

    #[test]
    fn an_arrow_moves_a_coordinate_unless_that_is_too_far_out() {
        assert_eq!(NUDGE, 50_000.0);
        assert_eq!(nudged(0.0, NUDGE), Some(50_000.0));
        assert_eq!(nudged(20_000.0, -NUDGE), Some(-30_000.0));
        assert_eq!(nudged(MAX_OFFSET - NUDGE, NUDGE), Some(MAX_OFFSET));
        assert_eq!(nudged(MAX_OFFSET, NUDGE), None);
        assert_eq!(nudged(-MAX_OFFSET, -NUDGE), None);
        assert_eq!(nudged(MAX_OFFSET, -NUDGE), Some(MAX_OFFSET - NUDGE));
    }

    #[test]
    fn positions_are_shown_and_read_in_kilometres() {
        assert_eq!(format_km(0.0), "0");
        assert_eq!(format_km(-0.0), "0");
        assert_eq!(format_km(200.0), "200");
        assert_eq!(format_km(-5600.0), "-5600");
        assert_eq!(format_km(0.5), "0.5");
        assert_eq!(format_km(12.3456), "12.346");
        assert_eq!(format_km(-0.0001), "0");

        assert_eq!(parse_km("200"), Some(200.0));
        assert_eq!(parse_km(" -5 600 km "), Some(-5600.0));
        assert_eq!(parse_km("1,400.5"), Some(1400.5));
        assert_eq!(parse_km("0"), Some(0.0));
        for text in ["", "-", "abc", "1e3", "12m", "1.2.3", "--5", "inf", "nan"] {
            assert_eq!(parse_km(text), None, "{text:?}");
        }
        // No further than a probe can be placed (100 AU).
        assert_eq!(parse_km("14959787070"), Some(14_959_787_070.0));
        assert_eq!(parse_km("14959787071"), None);
        assert_eq!(parse_km("-14959787071"), None);
    }

    #[test]
    fn numbers_are_shown_plainly() {
        assert_eq!(format_number(1.5), "1.5");
        assert_eq!(format_number(90.0), "90");
        assert_eq!(format_number(-22.5), "-22.5");
    }
}
