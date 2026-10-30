//! Custom probe formations, stored in an account file.
//!
//! `ui -> probescanning.customFormations` is
//! `(timestamp, {id: (name, [((x, y, z), range), ...])})` with positions and
//! ranges in metres. `ui -> probescanning.selectedFormationID` names the one
//! in use. Entries with a negative ID are the client's own scratch state
//! (for example `-4`, "tempFormation") and are carried through untouched.
//!
//! The presets and the idea of balancing a formation with a counterweight
//! probe follow EVE Wrench (MIT), `formation_editor/model.rs`.

use std::path::{Path, PathBuf};

use clone_bay_marshal::{Long, Value};
use thiserror::Error;

use crate::edit::{open, save_with, stored_setting, EditError, Write};
use crate::platform::EveStatus;
use crate::safe_write::{BackupSession, Fingerprint};

/// One astronomical unit in metres.
pub const AU: f64 = 149_597_870_700.0;
/// One kilometre in metres.
pub const KM: f64 = 1000.0;
/// A probe launcher holds eight probes.
pub const MAX_PROBES: usize = 8;
/// The scan ranges a probe can be set to, in AU.
pub const RANGE_OPTIONS: [f64; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// The range new probes start with, in AU.
pub const DEFAULT_RANGE: f64 = 4.0;
/// The longest name a formation may be given.
pub const MAX_NAME_CHARS: usize = 100;
/// The nearest to the centre a probe can be moved by setting its distance,
/// in metres. Any nearer and the direction it lies in would be lost.
pub const MIN_DISTANCE: f64 = 1.0;
/// The furthest a probe may be placed from the centre, in metres (100 AU).
pub const MAX_OFFSET: f64 = 100.0 * AU;
/// A formation whose centre is within this many metres of zero launches as
/// drawn; a drift this small can't be seen at probe scales.
pub const BALANCE_TOLERANCE: f64 = 0.5 * KM;

const SECTION: &str = "ui";
const FORMATIONS_KEY: &str = "probescanning.customFormations";
const SELECTED_KEY: &str = "probescanning.selectedFormationID";

/// One probe: where it sits relative to the formation's centre, and its
/// scan range. Both in metres.
///
/// Directions, as EVE Wrench names them (not yet confirmed in the client):
/// `+x` is west, `+y` is up and `+z` is north.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub range: f64,
}

impl Probe {
    /// A probe at a position given in kilometres, with a range in AU.
    /// How far the probe is from the centre, in metres.
    pub fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    /// Moves the probe straight towards or away from the centre until it is
    /// `metres` from it, keeping its direction. Positions are left rounded
    /// to the nearest millimetre.
    ///
    /// Returns `false`, changing nothing, when that can't be done: the
    /// probe is at the centre and so has no direction, or the distance is
    /// less than [`MIN_DISTANCE`] (which would lose the direction) or more
    /// than a probe can be placed at.
    pub fn set_distance(&mut self, metres: f64) -> bool {
        let now = self.distance();
        let wanted = (MIN_DISTANCE..=MAX_OFFSET).contains(&metres);
        if !wanted || now == 0.0 || !now.is_finite() {
            return false;
        }
        let factor = metres / now;
        let moved = [self.x, self.y, self.z].map(|c| round_to_millimetre(c * factor));
        if moved.iter().any(|c| c.abs() > MAX_OFFSET) {
            return false;
        }
        [self.x, self.y, self.z] = moved;
        true
    }

    pub fn at_km(x: f64, y: f64, z: f64, range_au: f64) -> Self {
        Self {
            x: x * KM,
            y: y * KM,
            z: z * KM,
            range: range_au * AU,
        }
    }
}

/// A formation the user made.
#[derive(Debug, Clone, PartialEq)]
pub struct Formation {
    pub id: i64,
    pub name: String,
    pub probes: Vec<Probe>,
    /// Whether the name is stored as text (as opposed to a byte string), so
    /// an untouched formation is written back exactly as it was.
    name_is_text: bool,
}

impl Formation {
    pub fn new(id: i64, name: &str, probes: Vec<Probe>) -> Self {
        Self {
            id,
            name: name.to_owned(),
            probes,
            name_is_text: true,
        }
    }

    /// The average position of the probes. The game launches a formation
    /// centred on this point.
    pub fn centre(&self) -> (f64, f64, f64) {
        if self.probes.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let count = self.probes.len() as f64;
        let sum = self.probes.iter().fold((0.0, 0.0, 0.0), |sum, probe| {
            (sum.0 + probe.x, sum.1 + probe.y, sum.2 + probe.z)
        });
        (sum.0 / count, sum.1 / count, sum.2 / count)
    }

    /// How far, in metres, the formation will be shifted when launched.
    pub fn launch_shift(&self) -> f64 {
        let (x, y, z) = self.centre();
        (x * x + y * y + z * z).sqrt()
    }

    /// Whether the formation launches as drawn.
    pub fn is_balanced(&self) -> bool {
        self.launch_shift() < BALANCE_TOLERANCE
    }

    /// Makes the formation launch as drawn by placing one counterweight
    /// probe opposite the rest. The probe is added if there is room;
    /// otherwise the last probe becomes the counterweight, keeping its range.
    pub fn balance(&mut self) {
        if self.is_balanced() {
            return;
        }
        // The counterweight takes the last probe's range either way.
        let Some(last) = self.probes.last().copied() else {
            return;
        };
        if self.probes.len() >= MAX_PROBES {
            // No room: give up the last probe's position and balance the rest.
            self.probes.pop();
        }
        self.push_counterweight(last.range);
    }

    /// Adds the probe that brings the centre of the current probes to zero.
    fn push_counterweight(&mut self, range: f64) {
        let sum = self.probes.iter().fold((0.0, 0.0, 0.0), |sum, probe| {
            (sum.0 + probe.x, sum.1 + probe.y, sum.2 + probe.z)
        });
        self.probes.push(Probe {
            x: -sum.0,
            y: -sum.1,
            z: -sum.2,
            range,
        });
    }

    /// Makes the formation larger or smaller by multiplying every position.
    /// Ranges are not changed. A factor that isn't a positive, finite number
    /// is ignored.
    pub fn scale(&mut self, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        for probe in &mut self.probes {
            probe.x *= factor;
            probe.y *= factor;
            probe.z *= factor;
        }
    }

    /// Turns the whole formation around its middle by `degrees`, clockwise
    /// as seen from the side `view` names. A negative number turns it the
    /// other way. Turning by a number that isn't finite does nothing.
    ///
    /// Quarter turns are exact. Any other angle leaves positions rounded to
    /// the nearest millimetre, so they don't pick up a tail of meaningless
    /// digits.
    pub fn turn(&mut self, view: View, degrees: f64) {
        if !degrees.is_finite() {
            return;
        }
        let quarter_turns = degrees / 90.0;
        let exact = quarter_turns.fract() == 0.0;
        let (sin, cos) = if exact {
            match quarter_turns.rem_euclid(4.0) as u8 {
                0 => (0.0, 1.0),
                1 => (1.0, 0.0),
                2 => (0.0, -1.0),
                _ => (-1.0, 0.0),
            }
        } else {
            degrees.to_radians().sin_cos()
        };
        // In each view, `across` runs to the right and `up` to the top, and
        // a clockwise turn carries "up" towards "across".
        let turned = |across: f64, up: f64| {
            let pair = (across * cos + up * sin, up * cos - across * sin);
            if exact {
                pair
            } else {
                (round_to_millimetre(pair.0), round_to_millimetre(pair.1))
            }
        };
        for probe in &mut self.probes {
            // West is +x, so east is -x.
            match view {
                // East to the right, north at the top.
                View::Above => {
                    let (east, north) = turned(-probe.x, probe.z);
                    probe.x = -east;
                    probe.z = north;
                }
                // East to the right, up at the top.
                View::South => {
                    let (east, up) = turned(-probe.x, probe.y);
                    probe.x = -east;
                    probe.y = up;
                }
                // North to the right, up at the top.
                View::East => {
                    let (north, up) = turned(probe.z, probe.y);
                    probe.z = north;
                    probe.y = up;
                }
            }
            // A quarter turn can leave "-0" where there was "0".
            for coordinate in [&mut probe.x, &mut probe.y, &mut probe.z] {
                if *coordinate == 0.0 {
                    *coordinate = 0.0;
                }
            }
        }
    }

    /// Swaps the two ends of one direction: north with south, west with
    /// east, or up with down.
    pub fn mirror(&mut self, direction: Direction) {
        for probe in &mut self.probes {
            let coordinate = match direction {
                Direction::NorthSouth => &mut probe.z,
                Direction::WestEast => &mut probe.x,
                Direction::UpDown => &mut probe.y,
            };
            // Leave a zero as it is instead of making it "-0".
            if *coordinate != 0.0 {
                *coordinate = -*coordinate;
            }
        }
    }

    /// Moves every probe by the same amount so the formation's centre is
    /// the middle, which makes it launch as drawn without adding a probe.
    /// The shape is unchanged.
    pub fn recentre(&mut self) {
        let (x, y, z) = self.centre();
        for probe in &mut self.probes {
            probe.x -= x;
            probe.y -= y;
            probe.z -= z;
        }
    }

    /// Sets every probe to the same scan range, given in AU.
    pub fn set_range(&mut self, range_au: f64) {
        for probe in &mut self.probes {
            probe.range = range_au * AU;
        }
    }
}

/// The side a formation is looked at from when it is turned. The names
/// match the picture's straight-on views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Looking down: north at the top, east to the right.
    Above,
    /// Looking north from the south: up at the top, east to the right.
    South,
    /// Looking west from the east: up at the top, north to the right.
    East,
}

/// One of the three directions a formation can be mirrored along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    NorthSouth,
    WestEast,
    UpDown,
}

fn round_to_millimetre(metres: f64) -> f64 {
    (metres * 1000.0).round() / 1000.0
}

/// A ready-made formation to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Eight probes all at the centre, to be placed by hand.
    Blank,
    /// Eight probes spread 250 km around the centre.
    Spread,
    /// A 500 km sphere with an extra pair 1000 km above and below.
    Pinpoint,
    /// Seven probes in a line along one axis, 200 km apart, with a
    /// counterweight on the far side so it launches as drawn.
    Stack(Axis),
}

/// A direction along one of the three axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    XPlus,
    XMinus,
    YPlus,
    YMinus,
    ZPlus,
    ZMinus,
}

impl Preset {
    pub const ALL: [Preset; 9] = [
        Preset::Blank,
        Preset::Spread,
        Preset::Pinpoint,
        Preset::Stack(Axis::XPlus),
        Preset::Stack(Axis::XMinus),
        Preset::Stack(Axis::YPlus),
        Preset::Stack(Axis::YMinus),
        Preset::Stack(Axis::ZPlus),
        Preset::Stack(Axis::ZMinus),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Blank => "Blank",
            Preset::Spread => "Spread (250 km)",
            Preset::Pinpoint => "Pinpoint (500 km)",
            Preset::Stack(Axis::XPlus) => "Line to the west",
            Preset::Stack(Axis::XMinus) => "Line to the east",
            Preset::Stack(Axis::YPlus) => "Line upwards",
            Preset::Stack(Axis::YMinus) => "Line downwards",
            Preset::Stack(Axis::ZPlus) => "Line to the north",
            Preset::Stack(Axis::ZMinus) => "Line to the south",
        }
    }

    /// The probes of this preset. Every preset has eight and is balanced.
    pub fn probes(self) -> Vec<Probe> {
        let at = |x: f64, y: f64, z: f64| Probe::at_km(x, y, z, DEFAULT_RANGE);
        let ring = |d: f64| {
            vec![
                at(d, 0.0, 0.0),
                at(-d, 0.0, 0.0),
                at(0.0, 0.0, d),
                at(0.0, 0.0, -d),
                at(0.0, d, 0.0),
                at(0.0, -d, 0.0),
                at(0.0, 2.0 * d, 0.0),
                at(0.0, -2.0 * d, 0.0),
            ]
        };
        match self {
            Preset::Blank => vec![at(0.0, 0.0, 0.0); MAX_PROBES],
            Preset::Spread => ring(250.0),
            Preset::Pinpoint => ring(500.0),
            Preset::Stack(axis) => {
                let along = |distance: f64| match axis {
                    Axis::XPlus => at(distance, 0.0, 0.0),
                    Axis::XMinus => at(-distance, 0.0, 0.0),
                    Axis::YPlus => at(0.0, distance, 0.0),
                    Axis::YMinus => at(0.0, -distance, 0.0),
                    Axis::ZPlus => at(0.0, 0.0, distance),
                    Axis::ZMinus => at(0.0, 0.0, -distance),
                };
                let mut probes: Vec<Probe> = (1..=7).map(|n| along(200.0 * f64::from(n))).collect();
                // 200 + 400 + ... + 1400 = 5600.
                probes.push(along(-5600.0));
                probes
            }
        }
    }
}

/// One entry of the stored dict, in stored order.
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    User(Formation),
    /// The client's own scratch state: kept exactly as it was.
    Scratch(Value, Value),
}

/// Every formation in an account file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Formations {
    entries: Vec<Entry>,
    /// The formation in use, as stored. `None` when the file doesn't say.
    pub selected: Option<i64>,
}

/// Why formations can't be read or saved.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormationError {
    #[error("probe formations can only be edited in an account file")]
    NotAnAccountFile,
    #[error("the probe formations are stored in a form this app doesn't understand")]
    Unreadable,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Edit(#[from] EditError),
}

impl Formations {
    /// The user's formations, in stored order.
    pub fn list(&self) -> Vec<&Formation> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::User(formation) => Some(formation),
                Entry::Scratch(..) => None,
            })
            .collect()
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut Formation> {
        self.entries.iter_mut().find_map(|entry| match entry {
            Entry::User(formation) if formation.id == id => Some(formation),
            _ => None,
        })
    }

    pub fn get(&self, id: i64) -> Option<&Formation> {
        self.list().into_iter().find(|formation| formation.id == id)
    }

    /// An ID no formation is using.
    pub fn next_id(&self) -> i64 {
        self.list()
            .iter()
            .map(|formation| formation.id)
            .max()
            .map_or(1, |max| max + 1)
    }

    /// Adds a formation with a fresh ID and returns that ID.
    pub fn add(&mut self, name: &str, probes: Vec<Probe>) -> i64 {
        let id = self.next_id();
        self.entries
            .push(Entry::User(Formation::new(id, name, probes)));
        id
    }

    /// Removes a formation. Removing the one in use is put right when the
    /// file is saved.
    pub fn remove(&mut self, id: i64) {
        self.entries
            .retain(|entry| !matches!(entry, Entry::User(formation) if formation.id == id));
    }

    /// The formation that will be in use after a save: the stored choice if
    /// it still exists, otherwise the first formation, otherwise 0. `None`
    /// when the file doesn't store a choice at all.
    pub fn selected_after_save(&self) -> Option<i64> {
        let selected = self.selected?;
        let list = self.list();
        if list.iter().any(|formation| formation.id == selected) {
            Some(selected)
        } else {
            Some(list.first().map_or(0, |formation| formation.id))
        }
    }

    /// Checks every formation against what the game allows.
    pub fn validate(&self) -> Result<(), FormationError> {
        let list = self.list();
        for (index, formation) in list.iter().enumerate() {
            let name = formation.name.trim();
            let invalid = |problem: &str| {
                let shown = if name.is_empty() { "(unnamed)" } else { name };
                Err(FormationError::Invalid(format!(
                    "Formation '{shown}': {problem}"
                )))
            };
            if formation.id < 0 || list[..index].iter().any(|other| other.id == formation.id) {
                return invalid("its ID is already in use");
            }
            if name.is_empty() {
                return invalid("it needs a name");
            }
            if formation.name.chars().count() > MAX_NAME_CHARS
                || formation.name.chars().any(char::is_control)
            {
                return invalid("its name is too long or has characters that can't be used");
            }
            if formation.probes.is_empty() || formation.probes.len() > MAX_PROBES {
                return invalid("it must have between 1 and 8 probes");
            }
            for probe in &formation.probes {
                let placed = [probe.x, probe.y, probe.z]
                    .iter()
                    .all(|c| c.is_finite() && c.abs() <= MAX_OFFSET);
                if !placed {
                    return invalid("a probe is placed too far away");
                }
                if !RANGE_OPTIONS
                    .iter()
                    .any(|option| option * AU == probe.range)
                {
                    return invalid("a probe has a scan range the game doesn't offer");
                }
            }
        }
        Ok(())
    }

    fn formations_value(&self) -> Value {
        let pairs = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Scratch(key, value) => (key.clone(), value.clone()),
                Entry::User(formation) => (Value::Int(formation.id), formation_value(formation)),
            })
            .collect();
        Value::Dict(pairs)
    }
}

fn formation_value(formation: &Formation) -> Value {
    let name = if formation.name_is_text {
        Value::Str(formation.name.clone())
    } else {
        Value::Bytes(formation.name.as_bytes().to_vec())
    };
    let probes = formation
        .probes
        .iter()
        .map(|probe| {
            let position = Value::Tuple(vec![
                Value::Float(probe.x),
                Value::Float(probe.y),
                Value::Float(probe.z),
            ]);
            Value::Tuple(vec![position, Value::Float(probe.range)])
        })
        .collect();
    Value::Tuple(vec![name, Value::List(probes)])
}

fn parse_formation(id: i64, value: &Value) -> Option<Formation> {
    let Value::Tuple(pair) = value else {
        return None;
    };
    let [name, Value::List(probes)] = pair.as_slice() else {
        return None;
    };
    let (name, name_is_text) = match name {
        Value::Str(name) => (name.clone(), true),
        Value::Bytes(bytes) => (String::from_utf8(bytes.clone()).ok()?, false),
        _ => return None,
    };
    let probes = probes
        .iter()
        .map(|probe| {
            let Value::Tuple(parts) = probe else {
                return None;
            };
            let [Value::Tuple(position), Value::Float(range)] = parts.as_slice() else {
                return None;
            };
            let [Value::Float(x), Value::Float(y), Value::Float(z)] = position.as_slice() else {
                return None;
            };
            Some(Probe {
                x: *x,
                y: *y,
                z: *z,
                range: *range,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Formation {
        id,
        name,
        probes,
        name_is_text,
    })
}

/// The value stored under `key` in the `ui` section, after the timestamp.
/// `Ok(None)` when it isn't stored.
fn stored<'a>(root: &'a Value, key: &str) -> Result<Option<&'a Value>, FormationError> {
    stored_setting(root, SECTION, key).map_err(|()| FormationError::Unreadable)
}

/// Reads the formations out of a decoded account file.
pub fn read(root: &Value) -> Result<Formations, FormationError> {
    let mut entries = Vec::new();
    match stored(root, FORMATIONS_KEY)? {
        None => {}
        Some(Value::Dict(pairs)) => {
            for (key, value) in pairs {
                match key {
                    Value::Int(id) if *id >= 0 => {
                        let formation =
                            parse_formation(*id, value).ok_or(FormationError::Unreadable)?;
                        entries.push(Entry::User(formation));
                    }
                    Value::Int(_) => entries.push(Entry::Scratch(key.clone(), value.clone())),
                    _ => return Err(FormationError::Unreadable),
                }
            }
        }
        Some(_) => return Err(FormationError::Unreadable),
    }
    let selected = match stored(root, SELECTED_KEY)? {
        None => None,
        Some(Value::Int(id)) => Some(*id),
        Some(_) => return Err(FormationError::Unreadable),
    };
    Ok(Formations { entries, selected })
}

/// Reads the formations of the account file at `path`.
pub fn read_file(path: &Path) -> Result<Formations, FormationError> {
    open_file(path).map(|(formations, _)| formations)
}

/// Reads the formations of the account file at `path` for editing, with
/// the fingerprint [`save`] needs.
pub fn open_file(path: &Path) -> Result<(Formations, Fingerprint), FormationError> {
    let opened = open(path)?;
    if opened.kind != crate::discovery::FileKind::Account {
        return Err(FormationError::NotAnAccountFile);
    }
    Ok((read(&opened.root)?, opened.fingerprint))
}

/// What has to be written to turn the file's formations into `wanted`.
/// Only what actually differs is written, so an untouched selection keeps
/// its timestamp.
fn writes(root: &Value, wanted: &Formations) -> Result<Vec<Write>, FormationError> {
    wanted.validate()?;
    // The file must be readable the same way, or we don't know what we
    // would be replacing.
    read(root)?;
    let mut writes = Vec::new();
    let formations = wanted.formations_value();
    if stored(root, FORMATIONS_KEY)? != Some(&formations) {
        writes.push((SECTION, FORMATIONS_KEY, formations));
    }
    if let Some(selected) = wanted.selected_after_save() {
        let selected = Value::Int(selected);
        if stored(root, SELECTED_KEY)? != Some(&selected) {
            writes.push((SECTION, SELECTED_KEY, selected));
        }
    }
    Ok(writes)
}

/// Whether saving `wanted` would change the file's formations at all.
pub fn differs(root: &Value, wanted: &Formations) -> Result<bool, FormationError> {
    Ok(!writes(root, wanted)?.is_empty())
}

/// Saves formations to the account file at `path`, with the same checks,
/// backup and self-check as any other edit. `opened_as` is the fingerprint
/// [`open_file`] gave; the save is refused if the file has changed since.
/// Returns the folder holding the backup.
pub fn save(
    path: &Path,
    opened_as: Fingerprint,
    wanted: &Formations,
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
    now: &Long,
) -> Result<PathBuf, FormationError> {
    let mut refused = None;
    let result = save_with(
        path,
        opened_as,
        eve,
        user_confirmed_eve_closed,
        backups,
        now,
        |kind, root| {
            if kind != crate::discovery::FileKind::Account {
                refused = Some(FormationError::NotAnAccountFile);
                return Err(EditError::NothingToSave);
            }
            writes(root, wanted).map_err(|error| {
                refused = Some(error);
                EditError::NothingToSave
            })
        },
    );
    match (result, refused) {
        (_, Some(error)) => Err(error),
        (result, None) => Ok(result?),
    }
}
