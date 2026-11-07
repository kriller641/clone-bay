//! Changing individual settings inside one settings file.
//!
//! Every setting is stored as `(timestamp, value)`. An edit replaces that
//! pair with the new value and the current time, and touches nothing else:
//! the result is proven to equal the original with only the edited keys
//! different before it is written.

use std::path::{Path, PathBuf};

use clone_bay_marshal::{decode, encode, Long, Value};
use thiserror::Error;

use crate::discovery::{classify, file_name, FileKind};
use crate::formations::{AU, KM};
use crate::inspect::{decode_settings, read_settings_bytes};
use crate::platform::EveStatus;
use crate::protect::key_name;
use crate::safe_write::{BackupSession, Fingerprint, WriteError};

/// How a setting is stored and what values it may take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// On or off, stored as a true/false value.
    Bool,
    /// On or off, stored as the number 1 or 0.
    Flag,
    /// A whole number within a range.
    Whole { min: i64, max: i64 },
    /// One of a fixed list of whole numbers, each with a label.
    Choice(&'static [(i64, &'static str)]),
    /// A number within a range, which may have a fractional part.
    Number { min: f64, max: f64 },
    /// A distance in metres for each kind of object, keyed by type ID.
    /// Type 0 is the default for everything not listed. A distance the
    /// user sets must be between `min` and `max`.
    Distances { min: i64, max: i64 },
    /// One of a fixed list of words, each with a label. Only words seen in
    /// a real file are offered.
    Word(&'static [(&'static str, &'static str)]),
    /// A list of solar systems, by ID. Systems can be taken off the list
    /// but not put on it.
    Systems,
    /// A number the user sees in one unit and the game stores in another:
    /// what is stored is what is shown times `per_unit`. `min` and `max`
    /// are in the unit shown, as is `decimals`, the number of decimal places
    /// shown. `whole` means the game stores a whole number.
    Scaled {
        min: f64,
        max: f64,
        per_unit: f64,
        decimals: u8,
        whole: bool,
        unit: &'static str,
    },
    /// One of the overview presets the same file holds, by name.
    Preset,
    /// The probe scanner's result filters. Filters can be renamed, removed
    /// and added, and what each one shows can be changed.
    ScanFilters,
}

/// One setting the app knows how to edit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setting {
    pub id: &'static str,
    /// What the user sees.
    pub label: &'static str,
    /// The heading it is listed under.
    pub heading: &'static str,
    /// The top-level section it is stored in.
    pub section: &'static str,
    /// The key it is stored under. Some settings are stored under two keys
    /// that must agree; all of them are written together.
    pub keys: &'static [&'static str],
    pub kind: Kind,
}

const fn ui(
    id: &'static str,
    heading: &'static str,
    label: &'static str,
    key: &'static [&'static str],
    kind: Kind,
) -> Setting {
    Setting {
        id,
        label,
        heading,
        section: "ui",
        keys: key,
        kind,
    }
}

const fn sound(id: &'static str, label: &'static str, keys: &'static [&'static str]) -> Setting {
    Setting {
        id,
        label,
        heading: "Sound levels",
        section: "audio",
        keys,
        kind: Kind::Number { min: 0.0, max: 1.0 },
    }
}

const WARP_DISTANCES: &[(i64, &str)] = &[
    (0, "0 km"),
    (10_000, "10 km"),
    (20_000, "20 km"),
    (30_000, "30 km"),
    (50_000, "50 km"),
    (70_000, "70 km"),
    (100_000, "100 km"),
];

const GAMEPLAY: &str = "Gameplay";
const CAMERA: &str = "Camera";
const GRAPHICS: &str = "Effects in space";
const ASTEROIDS: &str = "Asteroid field effects";
const SCANNING: &str = "Scanning";

/// Kilometres in one astronomical unit, as the game counts them: its own
/// longest directional scan, 14.3 AU, is stored as 2,139,249,551 km.
const KM_PER_AU: f64 = AU / KM;
const DRONES: &str = "Drones";
const DISTANCES: &str = "Default distances";
const AUTOPILOT: &str = "Autopilot";

/// The route types seen in real files. The game has at least one more
/// (preferring less secure space); its stored word hasn't been seen, so it
/// isn't offered, and a file holding it shows the setting as not editable.
const ROUTE_TYPES: &[(&str, &str)] = &[("shortest", "Shortest"), ("safe", "Prefer safer")];

const ACCOUNT_SETTINGS: &[Setting] = &[
    ui(
        "default_warp_to",
        GAMEPLAY,
        "Default warp-to distance",
        &["defaultwarpto"],
        Kind::Choice(WARP_DISTANCES),
    ),
    ui(
        "auto_target_back",
        GAMEPLAY,
        "Auto target back (number of targets)",
        &["autoTargetBack"],
        Kind::Whole { min: 0, max: 5 },
    ),
    ui(
        "timestamp_chat",
        GAMEPLAY,
        "Show timestamps in chat",
        &["timestampchat"],
        Kind::Bool,
    ),
    ui(
        "tooltip_delay",
        GAMEPLAY,
        "Tooltip delay (milliseconds)",
        &["tooltipDelayGeneric"],
        Kind::Number {
            min: 0.0,
            max: 2000.0,
        },
    ),
    ui(
        "window_transparency",
        GAMEPLAY,
        "Window transparency",
        &["windowTransparency"],
        Kind::Number { min: 0.0, max: 1.0 },
    ),
    ui(
        "show_empty_slots",
        GAMEPLAY,
        "Show empty module slots",
        &["showEmptySlots"],
        Kind::Bool,
    ),
    ui(
        "camera_shake",
        CAMERA,
        "Camera shake",
        &["cameraShakeEnabled"],
        Kind::Flag,
    ),
    ui(
        "camera_bobbing",
        CAMERA,
        "Camera bobbing",
        &["cameraBobbingEnabled"],
        Kind::Flag,
    ),
    ui(
        "camera_invert_y",
        CAMERA,
        "Invert vertical camera movement",
        &["cameraInvertY"],
        Kind::Flag,
    ),
    ui(
        "camera_invert_zoom",
        CAMERA,
        "Invert camera zoom",
        &["invertCameraZoom"],
        Kind::Flag,
    ),
    ui(
        "camera_dynamic",
        CAMERA,
        "Dynamic camera movement",
        &["cameraDynamicMovement"],
        Kind::Flag,
    ),
    ui(
        "camera_advanced",
        CAMERA,
        "Advanced camera menu",
        &["advancedCamera"],
        Kind::Flag,
    ),
    ui(
        "camera_offset_ui",
        CAMERA,
        "Offset the UI with the camera",
        &["offsetUIwithCamera"],
        Kind::Flag,
    ),
    ui(
        "turrets",
        GRAPHICS,
        "Turret effects",
        &["turretsEnabled"],
        Kind::Flag,
    ),
    ui(
        "missiles",
        GRAPHICS,
        "Missile effects",
        &["missilesEnabled"],
        Kind::Flag,
    ),
    ui("trails", GRAPHICS, "Trails", &["trailsEnabled"], Kind::Flag),
    ui(
        "effects",
        GRAPHICS,
        "Effects",
        &["effectsEnabled"],
        Kind::Flag,
    ),
    ui(
        "explosions",
        GRAPHICS,
        "Explosion effects",
        &["explosionEffectsEnabled"],
        Kind::Flag,
    ),
    ui(
        "gpu_particles",
        GRAPHICS,
        "GPU particles",
        &["gpuParticlesEnabled"],
        Kind::Flag,
    ),
    ui(
        "drone_models",
        GRAPHICS,
        "Drone models",
        &["droneModelsEnabled"],
        Kind::Flag,
    ),
    ui(
        "ship_skins",
        GRAPHICS,
        "Ship skins in space",
        &["modelSkinsInSpaceEnabled"],
        Kind::Flag,
    ),
    ui(
        "dscan_range",
        SCANNING,
        "Directional scan range",
        &["dir_scanrange"],
        Kind::Scaled {
            min: 0.0,
            max: 14.3,
            per_unit: KM_PER_AU,
            decimals: 3,
            whole: true,
            unit: "AU",
        },
    ),
    ui(
        "dscan_angle",
        SCANNING,
        "Directional scan angle",
        &["scan_angleSlider"],
        Kind::Scaled {
            min: 5.0,
            max: 360.0,
            per_unit: std::f64::consts::PI / 180.0,
            decimals: 1,
            whole: false,
            unit: "degrees",
        },
    ),
    ui(
        "dscan_preset",
        SCANNING,
        "Directional scan overview preset",
        &["scanner_presetInUse"],
        Kind::Preset,
    ),
    ui(
        "probe_scan_filters",
        SCANNING,
        "Probe scan result filters",
        &[FILTERS_KEY],
        Kind::ScanFilters,
    ),
    ui(
        "asteroid_atmospherics",
        ASTEROIDS,
        "Atmospherics",
        &["UI_ASTEROID_ATMOSPHERICS"],
        Kind::Flag,
    ),
    ui(
        "asteroid_cloud_field",
        ASTEROIDS,
        "Cloud field",
        &["UI_ASTEROID_CLOUDFIELD"],
        Kind::Flag,
    ),
    ui(
        "asteroid_fog",
        ASTEROIDS,
        "Fog",
        &["UI_ASTEROID_FOG"],
        Kind::Flag,
    ),
    ui(
        "asteroid_god_rays",
        ASTEROIDS,
        "God rays",
        &["UI_ASTEROID_GODRAYS"],
        Kind::Flag,
    ),
    ui(
        "asteroid_particles",
        ASTEROIDS,
        "Particles",
        &["UI_ASTEROID_PARTICLES"],
        Kind::Flag,
    ),
    sound(
        "sound_turrets",
        "Turrets",
        &["custom_turrets", "soundLevel_custom_turrets"],
    ),
    sound(
        "sound_secondary_interfaces",
        "Secondary interfaces",
        &[
            "custom_secondaryinterfaces",
            "soundLevel_custom_secondaryinterfaces",
        ],
    ),
    sound(
        "sound_atmosphere",
        "Atmosphere",
        &["custom_atmosphere", "soundLevel_custom_atmosphere"],
    ),
    sound(
        "sound_ship_sounds",
        "Ship sounds",
        &["custom_shipsounds", "soundLevel_custom_shipsounds"],
    ),
    sound(
        "sound_ship_effects",
        "Ship effects",
        &["custom_shipeffects", "soundLevel_custom_shipeffects"],
    ),
    sound(
        "sound_ui_click",
        "UI clicks",
        &["custom_uiclick", "soundLevel_custom_uiclick"],
    ),
    sound(
        "sound_jump_activation",
        "Jump activation",
        &["custom_jumpactivation", "soundLevel_custom_jumpactivation"],
    ),
    sound(
        "sound_warnings",
        "Warning sounds",
        &["custom_warningsfx", "soundLevel_custom_warningsfx"],
    ),
];

const CHARACTER_SETTINGS: &[Setting] = &[
    ui(
        "drone_aggression",
        DRONES,
        "Drones attack on their own",
        &["droneAggression"],
        Kind::Bool,
    ),
    ui(
        "drone_focus_fire",
        DRONES,
        "Drones focus fire",
        &["droneFocusFire"],
        Kind::Bool,
    ),
    ui(
        "route_type",
        AUTOPILOT,
        "Route",
        &["pfRouteType"],
        Kind::Word(ROUTE_TYPES),
    ),
    ui(
        "route_jump_gates",
        AUTOPILOT,
        "Use Ansiblex in routes",
        &["pathFinder_includeJumpGates"],
        Kind::Bool,
    ),
    ui(
        "avoided_systems",
        AUTOPILOT,
        "Systems to avoid",
        &["autopilot_avoidance2"],
        Kind::Systems,
    ),
    ui(
        "orbit_distances",
        DISTANCES,
        "Orbit",
        &["defaultTypeOrbitDist"],
        Kind::Distances {
            min: 500,
            max: 1_000_000,
        },
    ),
    ui(
        "keep_at_range_distances",
        DISTANCES,
        "Keep at range",
        &["defaultTypeKeepAtRangeDist"],
        Kind::Distances {
            min: 500,
            max: 1_000_000,
        },
    ),
    ui(
        "warp_to_distances",
        DISTANCES,
        "Warp to",
        &["defaultTypeWarpToDist"],
        Kind::Distances {
            min: 0,
            max: 100_000,
        },
    ),
];

/// The settings that can be edited in a kind of file.
pub fn settings_for(kind: FileKind) -> &'static [Setting] {
    match kind {
        FileKind::Account => ACCOUNT_SETTINGS,
        FileKind::Character => CHARACTER_SETTINGS,
    }
}

/// A setting's value, as the editor handles it.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Switch(bool),
    Whole(i64),
    Number(f64),
    /// `(type ID, metres)` pairs in stored order.
    Distances(Vec<(i64, i64)>),
    /// One of the words a [`Kind::Word`] setting offers.
    Word(String),
    /// Solar systems in stored order.
    Systems(Vec<ListedId>),
    /// Probe scan result filters in stored order.
    Filters(Vec<ScanFilter>),
}

/// One of the probe scanner's result filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFilter {
    /// The number the game knows it by. It can't be changed.
    pub id: i64,
    pub name: String,
    /// The kinds of result it holds: item group IDs, in stored order.
    pub groups: Vec<i64>,
}

/// The most filters the app will save. A guard against nonsense: the game's
/// own limit isn't known.
pub const MAX_FILTERS: usize = 200;

/// The longest name a filter may be given. A guard against nonsense: the
/// game's own limit isn't known.
pub const MAX_FILTER_NAME_CHARS: usize = 100;

/// A stored filter: `(name, [group IDs])`.
fn filter_parts(value: &Value) -> Option<(&Value, &[Value])> {
    let Value::Tuple(parts) = value else {
        return None;
    };
    match parts.as_slice() {
        [name, Value::List(groups)] if word(name).is_some() => Some((name, groups)),
        _ => None,
    }
}

/// The filters a stored value holds, if it is `{id: (name, [group IDs])}`
/// with no ID twice.
fn filters(value: &Value) -> Option<Vec<ScanFilter>> {
    let Value::Dict(pairs) = value else {
        return None;
    };
    let mut found: Vec<ScanFilter> = Vec::new();
    for (key, filter) in pairs {
        let Value::Int(id) = key else {
            return None;
        };
        let (name, groups) = filter_parts(filter)?;
        if found.iter().any(|other| other.id == *id) {
            return None;
        }
        let groups = groups
            .iter()
            .map(|group| match group {
                Value::Int(group) => Some(*group),
                _ => None,
            })
            .collect::<Option<Vec<i64>>>()?;
        found.push(ScanFilter {
            id: *id,
            name: word(name)?.to_owned(),
            groups,
        });
    }
    Some(found)
}

/// A stored number shown in the unit of a [`Kind::Scaled`] setting, rounded
/// to the decimal places it is shown with.
fn shown(stored: f64, per_unit: f64, decimals: u8) -> f64 {
    let places = 10f64.powi(i32::from(decimals));
    (stored / per_unit * places).round() / places
}

/// An ID in a stored list. The game writes some as one kind of whole number
/// and some as another; which is remembered so that an entry left on the
/// list is written back exactly as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListedId {
    pub id: i64,
    long: bool,
}

impl ListedId {
    fn read(value: &Value) -> Option<Self> {
        match value {
            Value::Int(id) => Some(Self {
                id: *id,
                long: false,
            }),
            Value::Long(id) => Some(Self {
                id: id.to_i64()?,
                long: true,
            }),
            _ => None,
        }
    }

    fn value(self) -> Value {
        if self.long {
            Value::Long(Long::from_i64(self.id))
        } else {
            Value::Int(self.id)
        }
    }
}

/// The systems a stored list holds, if it is a list of IDs.
fn listed(value: &Value) -> Option<Vec<ListedId>> {
    match value {
        Value::List(items) => items.iter().map(ListedId::read).collect(),
        _ => None,
    }
}

/// The text of a stored word, whichever kind of string it is.
fn word(value: &Value) -> Option<&str> {
    match value {
        Value::Bytes(bytes) => std::str::from_utf8(bytes).ok(),
        Value::Str(text) => Some(text),
        _ => None,
    }
}

/// What a file currently holds for a setting.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Stored(Content),
    /// The file doesn't store it, so the game is using its own default.
    NotStored,
    /// It is stored in a form the editor doesn't understand, so it can be
    /// neither shown nor changed.
    Unreadable,
}

fn section<'a>(root: &'a Value, name: &str) -> Option<&'a [(Value, Value)]> {
    let Value::Dict(sections) = root else {
        return None;
    };
    let mut found = sections
        .iter()
        .filter(|(key, _)| key_name(key).as_deref() == Some(name));
    match (found.next(), found.next()) {
        (Some((_, Value::Dict(inner))), None) => Some(inner),
        _ => None,
    }
}

/// The value part of the `(timestamp, value)` pair stored under `key`.
/// `Err` means the key is there but isn't such a pair, or is there twice.
fn stored<'a>(entries: &'a [(Value, Value)], key: &str) -> Result<Option<&'a Value>, ()> {
    let mut found = entries
        .iter()
        .filter(|(entry_key, _)| key_name(entry_key).as_deref() == Some(key));
    let (first, second) = (found.next(), found.next());
    match (first, second) {
        (None, _) => Ok(None),
        (Some((_, Value::Tuple(pair))), None) => match pair.as_slice() {
            [Value::Long(_), value] => Ok(Some(value)),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// The value part of the `(timestamp, value)` pair stored under `key` in
/// the top-level section `section_name`. `Ok(None)` when either isn't there.
/// `Err` when the file isn't a dict of sections, the section is there twice
/// or isn't a dict, or the key is there twice or isn't such a pair.
pub(crate) fn stored_setting<'a>(
    root: &'a Value,
    section_name: &str,
    key: &str,
) -> Result<Option<&'a Value>, ()> {
    let Value::Dict(sections) = root else {
        return Err(());
    };
    let mut found = sections
        .iter()
        .filter(|(name, _)| key_name(name).as_deref() == Some(section_name));
    match (found.next(), found.next()) {
        (None, _) => Ok(None),
        (Some((_, Value::Dict(entries))), None) => stored(entries, key),
        _ => Err(()),
    }
}

fn content(kind: Kind, value: &Value) -> Option<Content> {
    match (kind, value) {
        (Kind::Bool, Value::Bool(on)) => Some(Content::Switch(*on)),
        (Kind::Flag, Value::Int(n @ (0 | 1))) => Some(Content::Switch(*n == 1)),
        (Kind::Whole { .. } | Kind::Choice(_), Value::Int(n)) => Some(Content::Whole(*n)),
        (Kind::Number { .. }, Value::Float(x)) if x.is_finite() => Some(Content::Number(*x)),
        // The game stores some of these as whole numbers when they are whole.
        (Kind::Number { .. }, Value::Int(n)) => Some(Content::Number(*n as f64)),
        (Kind::Distances { .. }, Value::Dict(pairs)) => pairs
            .iter()
            .map(|pair| match pair {
                (Value::Int(type_id), Value::Int(metres)) => Some((*type_id, *metres)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Content::Distances),
        (Kind::Word(options), value) => {
            let stored = word(value)?;
            options
                .iter()
                .any(|(option, _)| *option == stored)
                .then(|| Content::Word(stored.to_owned()))
        }
        (Kind::Systems, value) => listed(value).map(Content::Systems),
        (
            Kind::Scaled {
                per_unit, decimals, ..
            },
            Value::Float(x),
        ) if x.is_finite() => Some(Content::Number(shown(*x, per_unit, decimals))),
        (
            Kind::Scaled {
                per_unit, decimals, ..
            },
            Value::Int(n),
        ) => Some(Content::Number(shown(*n as f64, per_unit, decimals))),
        (Kind::Preset, value) => word(value).map(|name| Content::Word(name.to_owned())),
        (Kind::ScanFilters, value) => filters(value).map(Content::Filters),
        _ => None,
    }
}

/// Reads a setting out of a decoded file.
pub fn read(root: &Value, setting: &Setting) -> Reading {
    let Some(entries) = section(root, setting.section) else {
        return match root {
            // No such section at all: nothing is stored.
            Value::Dict(sections)
                if !sections
                    .iter()
                    .any(|(key, _)| key_name(key).as_deref() == Some(setting.section)) =>
            {
                Reading::NotStored
            }
            _ => Reading::Unreadable,
        };
    };
    let mut readings = Vec::new();
    for key in setting.keys {
        match stored(entries, key) {
            Ok(Some(value)) => match content(setting.kind, value) {
                Some(content) => readings.push(Some(content)),
                None => return Reading::Unreadable,
            },
            Ok(None) => readings.push(None),
            Err(()) => return Reading::Unreadable,
        }
    }
    // Stored under several keys: they must all agree.
    let first = readings.first().cloned().flatten();
    if readings.iter().any(|reading| *reading != first) {
        return Reading::Unreadable;
    }
    first.map_or(Reading::NotStored, Reading::Stored)
}

/// A new value for one setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub setting: &'static str,
    pub content: Content,
}

/// Why an edit can't be made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    #[error("{0} isn't a settings file")]
    NotASettingsFile(String),
    #[error("can't read {file}: {reason}")]
    Unreadable { file: String, reason: String },
    #[error("'{0}' isn't a setting that can be edited in this kind of file")]
    UnknownSetting(String),
    #[error("'{0}' is stored in a form that can't be changed safely")]
    CannotChange(&'static str),
    #[error("the value given for '{0}' isn't allowed")]
    NotAllowed(&'static str),
    #[error("self-check failed: {0}")]
    SelfCheck(&'static str),
    #[error(
        "EVE is running. Close every EVE client first: the client saves its settings when it \
         closes and would overwrite the change. Nothing was changed"
    )]
    EveRunning,
    #[error(
        "Couldn't check whether EVE is running. Confirm that every EVE client is closed, then \
         try again. Nothing was changed"
    )]
    EveUnknown,
    #[error("{0}. Nothing was changed")]
    InvalidFilter(String),
    #[error(
        "the file changed after it was opened (EVE may have saved it), so nothing was saved. \
         Press \"Discard changes\" to load it again"
    )]
    ChangedSinceOpened,
    #[error("there is nothing to save")]
    NothingToSave,
    #[error("{0}")]
    Write(#[from] WriteError),
}

/// The value to store for a change, after checking it against the setting's
/// rules. `stored` is what the file currently holds for it, so a number
/// stays in the form the game wrote and an untouched distance isn't refused.
fn new_value(
    setting: &Setting,
    content: &Content,
    stored: Option<&Value>,
) -> Result<Value, EditError> {
    let refuse = || EditError::NotAllowed(setting.label);
    let stored_as_whole = matches!(stored, Some(Value::Int(_)));
    match (setting.kind, content) {
        (Kind::Bool, Content::Switch(on)) => Ok(Value::Bool(*on)),
        (Kind::Flag, Content::Switch(on)) => Ok(Value::Int(i64::from(*on))),
        (Kind::Whole { min, max }, Content::Whole(n)) if (min..=max).contains(n) => {
            Ok(Value::Int(*n))
        }
        (Kind::Choice(options), Content::Whole(n))
            if options.iter().any(|(value, _)| value == n) =>
        {
            Ok(Value::Int(*n))
        }
        (Kind::Number { min, max }, Content::Number(x)) if (min..=max).contains(x) => {
            if stored_as_whole && x.fract() == 0.0 {
                Ok(Value::Int(*x as i64))
            } else {
                Ok(Value::Float(*x))
            }
        }
        (Kind::Distances { min, max }, Content::Distances(entries)) => {
            // An entry the file already holds may stay as it is even if it
            // is outside the limits; only what the user sets is held to them.
            let unchanged = |type_id: i64, metres: i64| match stored {
                Some(Value::Dict(pairs)) => {
                    pairs.contains(&(Value::Int(type_id), Value::Int(metres)))
                }
                _ => false,
            };
            let mut seen = Vec::new();
            for (type_id, metres) in entries {
                let allowed = (min..=max).contains(metres) || unchanged(*type_id, *metres);
                let valid = *type_id >= 0 && allowed;
                if !valid || seen.contains(type_id) {
                    return Err(refuse());
                }
                seen.push(*type_id);
            }
            Ok(Value::Dict(
                entries
                    .iter()
                    .map(|(type_id, metres)| (Value::Int(*type_id), Value::Int(*metres)))
                    .collect(),
            ))
        }
        (Kind::Word(options), Content::Word(chosen))
            if options.iter().any(|(option, _)| option == chosen) =>
        {
            // Kept as the kind of string the game wrote.
            Ok(match stored {
                Some(Value::Str(_)) => Value::Str(chosen.clone()),
                _ => Value::Bytes(chosen.as_bytes().to_vec()),
            })
        }
        (
            Kind::Scaled {
                min,
                max,
                per_unit,
                whole,
                ..
            },
            Content::Number(x),
        ) if (min..=max).contains(x) => {
            let value = x * per_unit;
            let value = if whole { value.round() } else { value };
            // Kept as the kind of number the game wrote.
            if stored_as_whole && value.fract() == 0.0 {
                Ok(Value::Int(value as i64))
            } else {
                Ok(Value::Float(value))
            }
        }
        (Kind::Systems, Content::Systems(wanted)) => {
            // Only taking systems off is allowed: what is left must be
            // entries the file holds, in the order it holds them.
            let held = stored.and_then(listed).ok_or_else(refuse)?;
            let mut rest = held.iter();
            for entry in wanted {
                if !rest.any(|other| other == entry) {
                    return Err(refuse());
                }
            }
            Ok(Value::List(
                wanted.iter().map(|entry| entry.value()).collect(),
            ))
        }
        _ => Err(refuse()),
    }
}

const OVERVIEW: &str = "overview";
const PRESETS_KEY: &str = "overviewProfilePresets";
const FILTERS_KEY: &str = "probescanning.resultFilter.filters";
/// The filters switched on in the probe scanner: a set of filter IDs.
const ACTIVE_FILTERS_KEY: &str = "probescanning.resultFilter.activeFilterSet";
const SET: &str = "__builtin__.set";

/// The overview presets a file holds, as stored.
fn presets(root: &Value) -> &[(Value, Value)] {
    let entries = section(root, OVERVIEW).unwrap_or_default();
    match stored(entries, PRESETS_KEY) {
        Ok(Some(Value::Dict(presets))) => presets,
        _ => &[],
    }
}

/// The names of the overview presets a file holds, in stored order.
pub fn preset_names(root: &Value) -> Vec<String> {
    presets(root)
        .iter()
        .filter_map(|(name, _)| word(name).map(str::to_owned))
        .collect()
}

/// The names a [`Kind::Preset`] setting may be set to in this file. Empty
/// for every other kind of setting.
pub fn options(root: &Value, setting: &Setting) -> Vec<String> {
    match setting.kind {
        Kind::Preset => preset_names(root),
        _ => Vec::new(),
    }
}

/// What to store for a preset chosen by name: the preset's own name, as
/// the file holds it. A name the file has no preset for is refused.
fn preset_value(
    root: &Value,
    setting: &Setting,
    chosen: &str,
) -> Result<(&'static str, Value), EditError> {
    let [key] = setting.keys else {
        return Err(EditError::CannotChange(setting.label));
    };
    presets(root)
        .iter()
        .find(|(name, _)| word(name) == Some(chosen))
        .map(|(name, _)| (*key, name.clone()))
        .ok_or(EditError::NotAllowed(setting.label))
}

/// The IDs in a stored set of whole numbers, as the game writes one.
fn set_members(value: &Value) -> Option<&[Value]> {
    let Value::Reduce(reduce) = value else {
        return None;
    };
    let plain =
        reduce.state.is_none() && reduce.list_items.is_empty() && reduce.dict_items.is_empty();
    let Some(Value::Global(callable)) = &reduce.callable else {
        return None;
    };
    let Value::Tuple(args) = &reduce.args else {
        return None;
    };
    match args.as_slice() {
        [Value::List(members)]
            if plain
                && callable == SET
                && members.iter().all(|member| matches!(member, Value::Int(_))) =>
        {
            Some(members)
        }
        _ => None,
    }
}

/// What to store for an edited list of scan filters.
///
/// The filters the file holds keep their IDs and their order; any of them
/// may be left out. New filters come after them, with IDs above every ID
/// the file holds. A filter must have a name and show at least one kind of
/// result, and a kind that is added must be one the app knows. A filter
/// left as it was is written back exactly as stored. A removed filter is
/// also taken out of the set of filters switched on.
fn filter_values(
    setting: &Setting,
    entries: &[(Value, Value)],
    wanted: &[ScanFilter],
) -> Result<Vec<(&'static str, Value)>, EditError> {
    let refuse = || EditError::NotAllowed(setting.label);
    let held: &[(Value, Value)] = match stored(entries, FILTERS_KEY) {
        Ok(Some(Value::Dict(held))) => held,
        // A file that stores no filters yet can be given its first.
        Ok(None) => &[],
        _ => return Err(EditError::CannotChange(setting.label)),
    };
    let Some(held_filters) = filters(&Value::Dict(held.to_vec())) else {
        return Err(EditError::CannotChange(setting.label));
    };
    if wanted.len() > MAX_FILTERS {
        return Err(refuse());
    }
    let highest_held = held_filters
        .iter()
        .map(|filter| filter.id)
        .max()
        .unwrap_or(0);
    let mut rest = held.iter().zip(&held_filters);
    let mut last_new = highest_held.max(0);
    let mut kept = Vec::new();
    for filter in wanted {
        let name = filter.name.trim();
        let shown = if name.is_empty() { "(unnamed)" } else { name };
        let invalid = |problem: &str| {
            Err(EditError::InvalidFilter(format!(
                "Filter '{shown}' {problem}"
            )))
        };
        if name.is_empty() {
            return invalid("needs a name");
        }
        if name.chars().count() > MAX_FILTER_NAME_CHARS || name.chars().any(char::is_control) {
            return invalid("has a name that is too long or has characters that can't be used");
        }
        if filter.groups.is_empty() {
            return invalid("needs at least one kind of result ticked");
        }
        let repeated = filter
            .groups
            .iter()
            .enumerate()
            .any(|(index, group)| filter.groups[..index].contains(group));
        if repeated {
            return Err(refuse());
        }
        let groups = || {
            Value::List(
                filter
                    .groups
                    .iter()
                    .map(|group| Value::Int(*group))
                    .collect(),
            )
        };
        let is_held = held_filters.iter().any(|other| other.id == filter.id);
        if is_held {
            // New filters come last, so none may have been seen yet.
            if last_new != highest_held.max(0) {
                return Err(refuse());
            }
            let ((key, value), was) = rest
                .find(|(_, other)| other.id == filter.id)
                .ok_or_else(refuse)?;
            let (old_name, _) = filter_parts(value).ok_or_else(refuse)?;
            let known = |group: &i64| was.groups.contains(group) || offered(*group);
            if !filter.groups.iter().all(known) {
                return Err(refuse());
            }
            if was == filter {
                kept.push((key.clone(), value.clone()));
                continue;
            }
            let new_name = if word(old_name) == Some(filter.name.as_str()) {
                old_name.clone()
            } else {
                match old_name {
                    // Kept as a byte string where the game wrote one and
                    // the new name still fits in one.
                    Value::Bytes(_) if name.is_ascii() => Value::Bytes(name.as_bytes().to_vec()),
                    _ => Value::Str(name.to_owned()),
                }
            };
            kept.push((key.clone(), Value::Tuple(vec![new_name, groups()])));
        } else {
            if filter.id <= last_new || !filter.groups.iter().all(|group| offered(*group)) {
                return Err(refuse());
            }
            last_new = filter.id;
            kept.push((
                Value::Int(filter.id),
                Value::Tuple(vec![Value::Str(name.to_owned()), groups()]),
            ));
        }
    }
    let removed: Vec<&Value> = held
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !kept.iter().any(|(other, _)| other == *key))
        .collect();
    let mut values = vec![(FILTERS_KEY, Value::Dict(kept.clone()))];
    if !removed.is_empty() {
        match stored(entries, ACTIVE_FILTERS_KEY) {
            Ok(None) => {}
            Ok(Some(active)) => {
                let members = set_members(active).ok_or(EditError::CannotChange(setting.label))?;
                if members.iter().any(|member| removed.contains(&member)) {
                    let left: Vec<Value> = members
                        .iter()
                        .filter(|member| !removed.contains(member))
                        .cloned()
                        .collect();
                    let Value::Reduce(reduce) = active else {
                        return Err(EditError::CannotChange(setting.label));
                    };
                    let mut reduce = reduce.clone();
                    reduce.args = Value::Tuple(vec![Value::List(left)]);
                    values.push((ACTIVE_FILTERS_KEY, Value::Reduce(reduce)));
                }
            }
            Err(()) => return Err(EditError::CannotChange(setting.label)),
        }
    }
    Ok(values)
}

/// Whether a kind of result is one the app offers for adding to a filter.
fn offered(group: i64) -> bool {
    crate::bundled::scan_group(group).is_some_and(|group| group.published)
}

fn find_setting(kind: FileKind, id: &str) -> Result<&'static Setting, EditError> {
    settings_for(kind)
        .iter()
        .find(|setting| setting.id == id)
        .ok_or_else(|| EditError::UnknownSetting(id.to_owned()))
}

/// The `(key, value)` pairs to store for a set of changes, in order.
fn resolve(root: &Value, kind: FileKind, changes: &[Change]) -> Result<Vec<Write>, EditError> {
    let mut writes = Vec::new();
    for change in changes {
        let setting = find_setting(kind, change.setting)?;
        if read(root, setting) == Reading::Unreadable {
            return Err(EditError::CannotChange(setting.label));
        }
        let entries = section(root, setting.section).unwrap_or_default();
        let special = match (setting.kind, &change.content) {
            (Kind::Preset, Content::Word(chosen)) => {
                Some(vec![preset_value(root, setting, chosen)?])
            }
            (Kind::ScanFilters, Content::Filters(wanted)) => {
                Some(filter_values(setting, entries, wanted)?)
            }
            (Kind::Preset | Kind::ScanFilters, _) => {
                return Err(EditError::NotAllowed(setting.label));
            }
            _ => None,
        };
        if let Some(values) = special {
            for (key, value) in values {
                if writes
                    .iter()
                    .any(|(section, other, _)| (*section, *other) == (setting.section, key))
                {
                    return Err(EditError::NotAllowed(setting.label));
                }
                writes.push((setting.section, key, value));
            }
            continue;
        }
        for key in setting.keys {
            let held = stored(entries, key).ok().flatten();
            let value = new_value(setting, &change.content, held)?;
            if writes
                .iter()
                .any(|(section, other, _)| (*section, *other) == (setting.section, *key))
            {
                return Err(EditError::NotAllowed(setting.label));
            }
            writes.push((setting.section, *key, value));
        }
    }
    Ok(writes)
}

/// A copy of `root` with the changes made, each stamped with `now`.
/// Everything else, including the order of entries, is left as it was.
pub fn apply(
    root: &Value,
    kind: FileKind,
    changes: &[Change],
    now: &Long,
) -> Result<Value, EditError> {
    write_pairs(root, &resolve(root, kind, changes)?, now)
}

/// One value to store: the section, the key, and the value that goes after
/// the timestamp.
pub(crate) type Write = (&'static str, &'static str, Value);

/// A copy of `root` with each write stored as `(now, value)` under its key.
/// Everything else, including the order of entries, is left as it was.
pub(crate) fn write_pairs(root: &Value, writes: &[Write], now: &Long) -> Result<Value, EditError> {
    let Value::Dict(sections) = root else {
        return Err(EditError::SelfCheck("the file isn't a settings file"));
    };
    let mut sections = sections.clone();
    for (section_name, key, value) in writes.iter().cloned() {
        let pair = Value::Tuple(vec![Value::Long(now.clone()), value]);
        let position = sections
            .iter()
            .position(|(name, _)| key_name(name).as_deref() == Some(section_name));
        let entries = match position {
            Some(index) => match &mut sections[index].1 {
                Value::Dict(entries) => entries,
                _ => return Err(EditError::SelfCheck("a section isn't a list of settings")),
            },
            None => {
                sections.push((
                    Value::Bytes(section_name.as_bytes().to_vec()),
                    Value::Dict(vec![]),
                ));
                match sections.last_mut() {
                    Some((_, Value::Dict(entries))) => entries,
                    _ => return Err(EditError::SelfCheck("a section couldn't be added")),
                }
            }
        };
        match entries
            .iter_mut()
            .find(|(name, _)| key_name(name).as_deref() == Some(key))
        {
            Some((_, slot)) => *slot = pair,
            None => entries.push((Value::Bytes(key.as_bytes().to_vec()), pair)),
        }
    }
    Ok(Value::Dict(sections))
}

/// `root` with the given keys taken out, and sections left empty by that
/// dropped. What remains is everything an edit must not touch.
fn without_keys(root: &Value, keys: &[(&str, &str)]) -> Option<Value> {
    let Value::Dict(sections) = root else {
        return None;
    };
    let mut kept = Vec::new();
    for (name, value) in sections {
        let section_name = key_name(name);
        let edited: Vec<&str> = keys
            .iter()
            .filter(|(section, _)| section_name.as_deref() == Some(section))
            .map(|(_, key)| *key)
            .collect();
        match value {
            Value::Dict(entries) if !edited.is_empty() => {
                let entries: Vec<(Value, Value)> = entries
                    .iter()
                    .filter(|(key, _)| {
                        !key_name(key).is_some_and(|key| edited.contains(&key.as_ref()))
                    })
                    .cloned()
                    .collect();
                if !entries.is_empty() {
                    kept.push((name.clone(), Value::Dict(entries)));
                }
            }
            _ => kept.push((name.clone(), value.clone())),
        }
    }
    Some(Value::Dict(kept))
}

/// Decodes what is about to be written and proves it is the original with
/// exactly the requested changes: every edited key holds `(now, new value)`,
/// and everything else is identical and in the same order.
pub fn verify(
    data: &[u8],
    original: &Value,
    kind: FileKind,
    changes: &[Change],
    now: &Long,
) -> Result<(), EditError> {
    verify_pairs(data, original, &resolve(original, kind, changes)?, now)
}

/// Proves `data` decodes to `original` with exactly `writes` made: every
/// written key holds `(now, value)`, and everything else is identical and in
/// the same order.
pub(crate) fn verify_pairs(
    data: &[u8],
    original: &Value,
    writes: &[Write],
    now: &Long,
) -> Result<(), EditError> {
    let output = decode(data).map_err(|_| EditError::SelfCheck("the output can't be read back"))?;
    if output.trailing_bytes != 0 || output.unused_shared_slots != 0 {
        return Err(EditError::SelfCheck("the output can't be read back"));
    }
    let keys: Vec<(&str, &str)> = writes
        .iter()
        .map(|(section, key, _)| (*section, *key))
        .collect();
    if without_keys(&output.value, &keys) != without_keys(original, &keys) {
        return Err(EditError::SelfCheck(
            "a setting that wasn't edited has changed",
        ));
    }
    for (section_name, key, value) in writes {
        let entries = section(&output.value, section_name)
            .ok_or(EditError::SelfCheck("an edited section is missing"))?;
        let expected = Value::Tuple(vec![Value::Long(now.clone()), value.clone()]);
        let found: Vec<&Value> = entries
            .iter()
            .filter(|(name, _)| key_name(name).as_deref() == Some(key))
            .map(|(_, value)| value)
            .collect();
        if found != [&expected] {
            return Err(EditError::SelfCheck(
                "an edited setting doesn't hold the new value",
            ));
        }
    }
    Ok(())
}

/// The current time as the game stores it: 100-nanosecond ticks since
/// 1601-01-01 (a Windows FILETIME).
pub fn timestamp_now() -> Long {
    timestamp_from_unix(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

/// The game's timestamp for a time given as an offset from 1970-01-01.
pub fn timestamp_from_unix(since_unix_epoch: std::time::Duration) -> Long {
    /// Seconds between 1601-01-01 and 1970-01-01.
    const EPOCH_GAP_SECONDS: u128 = 11_644_473_600;
    const TICKS_PER_SECOND: u128 = 10_000_000;
    let ticks = EPOCH_GAP_SECONDS * TICKS_PER_SECOND + since_unix_epoch.as_nanos() / 100;
    Long::from_i64(i64::try_from(ticks).unwrap_or(i64::MAX))
}

/// A settings file read for editing.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub kind: FileKind,
    pub root: Value,
    /// What the file held when it was read. A save is refused once the file
    /// holds anything else, so changes the game made since aren't undone.
    pub fingerprint: Fingerprint,
}

/// Reads a settings file for editing.
pub fn open(path: &Path) -> Result<Opened, EditError> {
    let name = file_name(path);
    let (kind, _) = classify(&name).ok_or_else(|| EditError::NotASettingsFile(name.clone()))?;
    let unreadable = |reason: String| EditError::Unreadable {
        file: name.clone(),
        reason,
    };
    let bytes = read_settings_bytes(path).map_err(|e| unreadable(e.to_string()))?;
    let root = decode_settings(&bytes)
        .map_err(|e| unreadable(e.to_string()))?
        .value;
    Ok(Opened {
        kind,
        root,
        fingerprint: Fingerprint::of(&bytes),
    })
}

/// Reads every setting that can be edited in a file, in display order.
pub fn read_file(path: &Path) -> Result<Vec<(&'static Setting, Reading)>, EditError> {
    let Opened { kind, root, .. } = open(path)?;
    Ok(settings_for(kind)
        .iter()
        .map(|setting| (setting, read(&root, setting)))
        .collect())
}

/// Saves changes to one settings file.
///
/// `opened_as` is the [`Opened::fingerprint`] of the contents the changes
/// were made against; the save is refused if the file holds anything else.
/// Refuses if EVE is running, or might be and the user hasn't confirmed it
/// is closed. `backups` makes sure the file has a verified backup before it
/// is replaced. Returns the folder holding that backup.
pub fn save(
    path: &Path,
    opened_as: Fingerprint,
    changes: &[Change],
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
    now: &Long,
) -> Result<PathBuf, EditError> {
    if changes.is_empty() {
        return Err(EditError::NothingToSave);
    }
    save_with(
        path,
        opened_as,
        eve,
        user_confirmed_eve_closed,
        backups,
        now,
        |kind, root| resolve(root, kind, changes),
    )
}

/// Saves whatever `plan` decides to write, given the file's kind and current
/// contents, with the same checks and backup as [`save`].
pub(crate) fn save_with(
    path: &Path,
    opened_as: Fingerprint,
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
    now: &Long,
    plan: impl FnOnce(FileKind, &Value) -> Result<Vec<Write>, EditError>,
) -> Result<PathBuf, EditError> {
    match eve {
        EveStatus::Running => return Err(EditError::EveRunning),
        EveStatus::Unknown if !user_confirmed_eve_closed => return Err(EditError::EveUnknown),
        EveStatus::Unknown | EveStatus::NotRunning => {}
    }
    let name = file_name(path);
    let (kind, _) = classify(&name).ok_or_else(|| EditError::NotASettingsFile(name.clone()))?;
    let unreadable = |reason: String| EditError::Unreadable {
        file: name.clone(),
        reason,
    };
    let bytes = read_settings_bytes(path).map_err(|e| unreadable(e.to_string()))?;
    let fingerprint = Fingerprint::of(&bytes);
    if fingerprint != opened_as {
        return Err(EditError::ChangedSinceOpened);
    }
    let original = decode_settings(&bytes).map_err(|e| unreadable(e.to_string()))?;
    let writes = plan(kind, &original.value)?;
    if writes.is_empty() {
        return Err(EditError::NothingToSave);
    }
    let edited = write_pairs(&original.value, &writes, now)?;
    let data = encode(&edited, original.had_checksum)
        .map_err(|_| EditError::SelfCheck("the output can't be written"))?;
    verify_pairs(&data, &original.value, &writes, now)?;

    let backup_dir = backups.back_up(path, fingerprint)?;
    backups.replace_file(path, &data, fingerprint)?;
    Ok(backup_dir)
}
