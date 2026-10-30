//! Finding profiles and settings files in a folder tree shaped like a real
//! EVE settings folder.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use clone_bay_core::discovery::{
    find_profiles, list_settings_files, locate_profiles, DiscoveryError, FileKind,
};

fn touch(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Mirrors what was found on a real PC: two installs, several profiles,
/// backup folders and stray files alongside.
fn eve_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let base = root.path();
    let live = base.join("c_eve_sharedcache_tq_tranquility");
    touch(
        &live.join("settings_Default").join("core_char_22.dat"),
        b"char",
    );
    touch(
        &live.join("settings_Default").join("core_user_11.dat"),
        b"account",
    );
    touch(&live.join("settings_Default").join("core_char_3.dat"), b"c");
    touch(&live.join("settings_Default").join("prefs.ini"), b"");
    touch(
        &live.join("settings_Default").join("core_public__.dat"),
        b"",
    );
    touch(
        &live.join("settings_Default").join("core_char_22.dat.bak"),
        b"",
    );
    // A folder with a settings-file name is not a settings file.
    fs::create_dir_all(live.join("settings_Default").join("core_char_99.dat")).unwrap();
    touch(&live.join("settings_raT").join("core_char_5.dat"), b"x");
    fs::create_dir_all(live.join("SETTINGS_Upper")).unwrap();
    fs::create_dir_all(live.join("cache")).unwrap();
    touch(&live.join("backup june 9").join("core_char_1.dat"), b"x");
    // A file, not a folder, with a profile-like name.
    touch(&live.join("settings_file.txt"), b"");
    touch(
        &base
            .join("c_eve_sharedcache_sisi_singularity")
            .join("settings_Default")
            .join("core_user_7.dat"),
        b"x",
    );
    fs::create_dir_all(base.join("IGBR")).unwrap();
    touch(&base.join("settings.dat"), b"");
    root
}

#[test]
fn finds_every_profile_in_every_install() {
    let root = eve_root();
    let found: Vec<(String, String)> = find_profiles(root.path())
        .unwrap()
        .into_iter()
        .map(|profile| (profile.install, profile.name))
        .collect();
    let expected = [
        ("c_eve_sharedcache_sisi_singularity", "Default"),
        ("c_eve_sharedcache_tq_tranquility", "Default"),
        ("c_eve_sharedcache_tq_tranquility", "Upper"),
        ("c_eve_sharedcache_tq_tranquility", "raT"),
    ]
    .map(|(install, name)| (install.to_owned(), name.to_owned()));
    assert_eq!(found, expected);
}

#[test]
fn a_profile_path_points_at_its_folder() {
    let root = eve_root();
    let profiles = find_profiles(root.path()).unwrap();
    let rat = profiles
        .iter()
        .find(|profile| profile.name == "raT")
        .unwrap();
    assert_eq!(
        rat.path,
        root.path()
            .join("c_eve_sharedcache_tq_tranquility")
            .join("settings_raT")
    );
}

#[test]
fn lists_only_settings_files_sorted_by_kind_then_id() {
    let root = eve_root();
    let folder = root
        .path()
        .join("c_eve_sharedcache_tq_tranquility")
        .join("settings_Default");
    let files = list_settings_files(&folder).unwrap();
    let found: Vec<(FileKind, u64, u64)> = files
        .iter()
        .map(|file| (file.kind, file.id, file.size))
        .collect();
    assert_eq!(
        found,
        [
            (FileKind::Character, 3, 1),
            (FileKind::Character, 22, 4),
            (FileKind::Account, 11, 7),
        ]
    );
    assert_eq!(files[0].path, folder.join("core_char_3.dat"));
    assert!(files.iter().all(|file| file.modified.is_some()));
}

#[test]
fn an_empty_profile_lists_nothing() {
    let root = eve_root();
    let folder = root
        .path()
        .join("c_eve_sharedcache_tq_tranquility")
        .join("SETTINGS_Upper");
    assert!(list_settings_files(&folder).unwrap().is_empty());
}

#[test]
fn a_missing_folder_is_an_error_that_names_it() {
    let root = eve_root();
    let missing = root.path().join("nope");
    for result in [
        find_profiles(&missing).map(|_| ()),
        list_settings_files(&missing).map(|_| ()),
        locate_profiles(&missing).map(|_| ()),
    ] {
        let DiscoveryError::ReadFolder { path, .. } = result.unwrap_err();
        assert_eq!(path, missing);
    }
    let message = find_profiles(&missing).unwrap_err().to_string();
    assert!(message.starts_with("can't read the folder "), "{message}");
    assert!(message.contains("nope"), "{message}");
}

#[test]
fn a_root_with_no_installs_has_no_profiles() {
    let empty = tempfile::tempdir().unwrap();
    assert!(find_profiles(empty.path()).unwrap().is_empty());
    assert!(locate_profiles(empty.path()).unwrap().is_empty());
}

fn located(folder: &Path) -> Vec<(String, String)> {
    locate_profiles(folder)
        .unwrap()
        .into_iter()
        .map(|profile| (profile.install, profile.name))
        .collect()
}

#[test]
fn locating_works_from_the_root_an_install_or_a_profile() {
    let root = eve_root();
    let live = root.path().join("c_eve_sharedcache_tq_tranquility");

    // The root: every profile of every install.
    assert_eq!(located(root.path()).len(), 4);

    // An install folder: its profiles, sorted by name.
    let names: Vec<String> = located(&live).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["Default", "Upper", "raT"]);
    assert!(located(&live)
        .iter()
        .all(|(install, _)| install == "c_eve_sharedcache_tq_tranquility"));

    // A profile folder: just itself, with the install it sits in.
    let profile = live.join("settings_raT");
    assert_eq!(
        located(&profile),
        [(
            "c_eve_sharedcache_tq_tranquility".to_owned(),
            "raT".to_owned()
        )]
    );
    assert_eq!(locate_profiles(&profile).unwrap()[0].path, profile);
}

#[test]
fn any_folder_holding_settings_files_can_be_opened_directly() {
    // A backup folder isn't named settings_*, but it holds settings files.
    let root = eve_root();
    let backup = root
        .path()
        .join("c_eve_sharedcache_tq_tranquility")
        .join("backup june 9");
    assert_eq!(
        located(&backup),
        [(
            "c_eve_sharedcache_tq_tranquility".to_owned(),
            "backup june 9".to_owned()
        )]
    );
}
