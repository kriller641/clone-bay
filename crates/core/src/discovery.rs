//! Finding settings profiles and the settings files inside them.
//!
//! Layout on disk: `<EVE root>/<install>/settings_<profile>/core_*.dat`.
//! The install folder is named after the client's install path and server, so
//! it differs from PC to PC and is never assumed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;

const PROFILE_PREFIX: &str = "settings_";
const CHARACTER_PREFIX: &str = "core_char_";
const ACCOUNT_PREFIX: &str = "core_user_";
const FILE_SUFFIX: &str = ".dat";

/// Why a folder couldn't be scanned.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("can't read the folder {path} ({source})")]
    ReadFolder {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// One settings profile: a `settings_*` folder inside an install folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The install folder's name, e.g. `c_eve_sharedcache_tq_tranquility`.
    pub install: String,
    /// The profile's name without the `settings_` prefix, e.g. `Default`.
    pub name: String,
    pub path: PathBuf,
}

/// Which kind of settings a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    /// `core_char_<characterID>.dat`
    Character,
    /// `core_user_<accountID>.dat`, shared by every character on the account.
    Account,
}

/// A settings file found in a profile folder. Nothing has been read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFileEntry {
    pub kind: FileKind,
    /// The character or account ID from the file name.
    pub id: u64,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Works out what a file name is, or `None` if it isn't a settings file.
///
/// Matching ignores case, as Windows file names do.
pub fn classify(file_name: &str) -> Option<(FileKind, u64)> {
    let lower = file_name.to_ascii_lowercase();
    let stem = lower.strip_suffix(FILE_SUFFIX)?;
    let (kind, digits) = if let Some(digits) = stem.strip_prefix(CHARACTER_PREFIX) {
        (FileKind::Character, digits)
    } else {
        (FileKind::Account, stem.strip_prefix(ACCOUNT_PREFIX)?)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((kind, digits.parse().ok()?))
}

/// A path's file name, for messages and lists. A path with no file name is
/// shown whole.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn read_dir(path: &Path) -> Result<fs::ReadDir, DiscoveryError> {
    fs::read_dir(path).map_err(|source| DiscoveryError::ReadFolder {
        path: path.to_path_buf(),
        source,
    })
}

/// Sub-folders of `path` as `(name, path)`. Entries that can't be examined,
/// or whose names aren't valid Unicode, are left out.
fn sub_folders(path: &Path) -> Result<Vec<(String, PathBuf)>, DiscoveryError> {
    let mut folders = Vec::new();
    for entry in read_dir(path)?.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = entry.path();
        if path.is_dir() {
            folders.push((name, path));
        }
    }
    Ok(folders)
}

/// Every settings profile under the EVE root folder, sorted by install and
/// then profile name.
///
/// An install folder that can't be read is skipped; only a failure to read
/// the root itself is an error.
pub fn find_profiles(eve_root: &Path) -> Result<Vec<Profile>, DiscoveryError> {
    let mut profiles = Vec::new();
    for (install, install_path) in sub_folders(eve_root)? {
        let Ok(folders) = sub_folders(&install_path) else {
            continue;
        };
        for (folder, path) in folders {
            if let Some(name) = strip_prefix_ignore_case(&folder, PROFILE_PREFIX) {
                profiles.push(Profile {
                    install: install.clone(),
                    name: name.to_owned(),
                    path,
                });
            }
        }
    }
    profiles.sort_by(|a, b| (&a.install, &a.name).cmp(&(&b.install, &b.name)));
    Ok(profiles)
}

/// The profiles at a folder the user pointed the app at, whatever level of
/// the layout it is: a profile folder itself, an install folder, or the EVE
/// root.
pub fn locate_profiles(folder: &Path) -> Result<Vec<Profile>, DiscoveryError> {
    if !list_settings_files(folder)?.is_empty() {
        return Ok(vec![profile_for(folder_name(folder.parent()), folder)]);
    }
    let install = folder_name(Some(folder));
    let mut profiles: Vec<Profile> = sub_folders(folder)?
        .into_iter()
        .filter(|(name, _)| strip_prefix_ignore_case(name, PROFILE_PREFIX).is_some())
        .map(|(_, path)| profile_for(install.clone(), &path))
        .collect();
    if profiles.is_empty() {
        return find_profiles(folder);
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

fn folder_name(folder: Option<&Path>) -> String {
    folder
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn profile_for(install: String, path: &Path) -> Profile {
    let folder = folder_name(Some(path));
    let name = strip_prefix_ignore_case(&folder, PROFILE_PREFIX).unwrap_or(&folder);
    Profile {
        install,
        name: name.to_owned(),
        path: path.to_path_buf(),
    }
}

/// The profile to show first: the live server's `Default` profile when there
/// is one, otherwise the first.
pub fn default_profile_index(profiles: &[Profile]) -> Option<usize> {
    profiles
        .iter()
        .position(|profile| {
            profile.install.to_ascii_lowercase().contains("tranquility")
                && profile.name.eq_ignore_ascii_case("Default")
        })
        .or(if profiles.is_empty() { None } else { Some(0) })
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// The settings files directly inside a profile folder, sorted by kind and
/// then ID. Other files and sub-folders are ignored.
pub fn list_settings_files(profile_dir: &Path) -> Result<Vec<SettingsFileEntry>, DiscoveryError> {
    let mut files = Vec::new();
    for entry in read_dir(profile_dir)?.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some((kind, id)) = classify(&name) else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        files.push(SettingsFileEntry {
            kind,
            id,
            path: entry.path(),
            size: metadata.len(),
            modified: metadata.modified().ok(),
        });
    }
    files.sort_by_key(|file| (file.kind, file.id));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognises_both_kinds() {
        assert_eq!(
            classify("core_char_2112345678.dat"),
            Some((FileKind::Character, 2_112_345_678))
        );
        assert_eq!(
            classify("core_user_12345678.dat"),
            Some((FileKind::Account, 12_345_678))
        );
    }

    #[test]
    fn classify_ignores_case() {
        assert_eq!(classify("CORE_CHAR_7.DAT"), Some((FileKind::Character, 7)));
        assert_eq!(classify("Core_User_8.Dat"), Some((FileKind::Account, 8)));
    }

    #[test]
    fn classify_rejects_everything_else() {
        for name in [
            "",
            "core_char_.dat",
            "core_user_.dat",
            "core_char__.dat",
            "core_char_12a.dat",
            "core_char_-12.dat",
            "core_char_+12.dat",
            "core_char_12.dat.bak",
            "core_char_12.txt",
            "core_char_12",
            "xcore_char_12.dat",
            "core_public__.dat",
            "prefs.ini",
            // Too large for any real ID.
            "core_char_99999999999999999999999.dat",
        ] {
            assert_eq!(classify(name), None, "{name:?}");
        }
    }

    #[test]
    fn classify_accepts_the_largest_id() {
        let name = format!("core_user_{}.dat", u64::MAX);
        assert_eq!(classify(&name), Some((FileKind::Account, u64::MAX)));
    }

    #[test]
    fn prefix_stripping_is_case_insensitive_and_safe_on_short_or_odd_text() {
        assert_eq!(
            strip_prefix_ignore_case("settings_Default", "settings_"),
            Some("Default")
        );
        assert_eq!(
            strip_prefix_ignore_case("SETTINGS_x", "settings_"),
            Some("x")
        );
        assert_eq!(strip_prefix_ignore_case("settings_", "settings_"), Some(""));
        assert_eq!(strip_prefix_ignore_case("settings", "settings_"), None);
        assert_eq!(strip_prefix_ignore_case("cache", "settings_"), None);
        // A multi-byte character straddling the prefix length must not panic.
        assert_eq!(strip_prefix_ignore_case("settingsé", "settings_"), None);
    }

    fn profile(install: &str, name: &str) -> Profile {
        Profile {
            install: install.to_owned(),
            name: name.to_owned(),
            path: PathBuf::from(name),
        }
    }

    #[test]
    fn the_live_default_profile_is_shown_first() {
        let profiles = [
            profile("c_eve_sisi_singularity", "Default"),
            profile("c_eve_tq_Tranquility", "alt"),
            profile("c_eve_tq_Tranquility", "default"),
        ];
        assert_eq!(default_profile_index(&profiles), Some(2));
        assert_eq!(default_profile_index(&profiles[..2]), Some(0));
        assert_eq!(default_profile_index(&[]), None);
    }
}
