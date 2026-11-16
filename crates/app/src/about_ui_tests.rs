//! Tests that drive the About page in the real UI, off-screen, against a
//! settings folder in a temp folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::{NodeT, Queryable};

use crate::about_flow::{REPOSITORY, VERSION};
use crate::app::test_support::{click, harness_with, press_esc, shows, Ui};
use crate::services::Services;
use crate::state::test_support::eve_root;
use crate::state::Screen;

/// The `Default` profile's folder in a root made by `eve_root`.
fn profile(root: &Path) -> PathBuf {
    root.join("c_eve_tq_tranquility").join("settings_Default")
}

fn about_page(root: &Path, services: Services) -> Ui {
    let mut ui = harness_with(root, services);
    click(&mut ui, "About");
    assert!(ui.state().state.about().is_some());
    ui
}

/// Every link or folder a stand-in was asked to open.
type Opened<T> = Arc<Mutex<Vec<T>>>;

/// A list for a stand-in to record into, and a second handle to read it.
fn recorder<T: Send + 'static>() -> (Opened<T>, Opened<T>) {
    let opened = Arc::new(Mutex::new(Vec::new()));
    (Arc::clone(&opened), opened)
}

#[test]
fn the_status_bar_opens_the_about_page_and_esc_leaves_it() {
    let root = eve_root();
    let mut ui = about_page(root.path(), Services::for_tests());
    ui.get_by_label("About Clone Bay");
    ui.get_by_label(&format!("Version {VERSION}"));
    ui.get_by_label_contains("Always keeps your module layouts and drone groups.");
    ui.get_by_label_contains("You choose what else is copied:");
    ui.get_by_label_contains("It won't change anything while EVE is running.");
    ui.get_by_label_contains("carbonengine/blue");
    press_esc(&mut ui);
    assert!(matches!(ui.state().state.screen, Screen::Files));
}

#[test]
fn back_to_the_file_list_leaves_the_page() {
    let root = eve_root();
    let mut ui = about_page(root.path(), Services::for_tests());
    click(&mut ui, "Back to the file list");
    assert!(matches!(ui.state().state.screen, Screen::Files));
}

#[test]
fn the_github_button_opens_the_projects_page() {
    let root = eve_root();
    let (opened, record) = recorder::<String>();
    let services = Services {
        open_link: Arc::new(move |url| {
            record.lock().unwrap().push(url.to_owned());
            Ok(())
        }),
        ..Services::for_tests()
    };
    let mut ui = about_page(root.path(), services);
    click(&mut ui, "Open the GitHub page");
    assert_eq!(*opened.lock().unwrap(), [REPOSITORY.to_owned()]);
    assert_eq!(ui.state().state.about().unwrap().problem, None);
}

#[test]
fn a_page_that_cannot_be_opened_says_why() {
    let root = eve_root();
    let services = Services {
        open_link: Arc::new(|_| Err("no browser was found".to_owned())),
        ..Services::for_tests()
    };
    let mut ui = about_page(root.path(), services);
    click(&mut ui, "Open the GitHub page");
    ui.get_by_label("Couldn't open the page: no browser was found.");
}

#[test]
fn with_no_backups_the_folder_button_is_greyed_out() {
    let root = eve_root();
    let ui = about_page(root.path(), Services::for_tests());
    ui.get_by_label_contains("Backups for the profile Default are kept in:");
    let folder = profile(root.path()).join("clone-bay-backups");
    ui.get_by_label(&folder.display().to_string());
    assert!(ui
        .get_by_label("Open the backup folder")
        .accesskit_node()
        .is_disabled());
    ui.get_by_label("No backups yet: one is made the first time a file is changed.");
}

#[test]
fn the_backup_folder_is_counted_and_opens() {
    let root = eve_root();
    let folder = profile(root.path()).join("clone-bay-backups");
    fs::create_dir_all(folder.join("20261106-140209")).unwrap();
    let (opened, record) = recorder::<PathBuf>();
    let services = Services {
        show_folder: Arc::new(move |folder| {
            record.lock().unwrap().push(folder.to_path_buf());
            Ok(())
        }),
        ..Services::for_tests()
    };
    let mut ui = about_page(root.path(), services);
    ui.get_by_label("1 backup folder, the newest from 6 Nov 2026, 14:02");
    click(&mut ui, "Open the backup folder");
    assert_eq!(*opened.lock().unwrap(), [folder]);
}

#[test]
fn a_backup_folder_that_cannot_be_opened_says_why() {
    let root = eve_root();
    fs::create_dir_all(profile(root.path()).join("clone-bay-backups").join("x")).unwrap();
    let services = Services {
        show_folder: Arc::new(|_| Err("the folder X isn't there any more".to_owned())),
        ..Services::for_tests()
    };
    let mut ui = about_page(root.path(), services);
    click(&mut ui, "Open the backup folder");
    ui.get_by_label("Couldn't open the folder: the folder X isn't there any more.");
}

#[test]
fn the_licence_texts_are_built_in_and_shown_when_asked() {
    let root = eve_root();
    let mut ui = about_page(root.path(), Services::for_tests());
    assert!(!shows(&ui, "MIT License"));
    click(&mut ui, "Full licence texts");
    assert!(ui.query_all_by_label("MIT License").next().is_some());
}

#[test]
fn the_about_page_opens_by_itself_only_the_first_time() {
    let root = eve_root();
    let app_data = tempfile::tempdir().unwrap();
    let seen = app_data.path().join("clone-bay").join("about_seen.txt");
    let services = || Services {
        about_seen_file: Some(seen.clone()),
        ..Services::for_tests()
    };

    let first = harness_with(root.path(), services());
    assert!(first.state().state.about().is_some());
    first.get_by_label("About Clone Bay");
    assert!(seen.exists());

    let second = harness_with(root.path(), services());
    assert!(matches!(second.state().state.screen, Screen::Files));
}

#[test]
fn with_nowhere_to_remember_it_the_page_never_opens_by_itself() {
    let root = eve_root();
    let ui = harness_with(root.path(), Services::for_tests());
    assert!(matches!(ui.state().state.screen, Screen::Files));
}

#[test]
fn with_no_settings_found_the_page_still_opens_and_says_where_backups_go() {
    let empty = tempfile::tempdir().unwrap();
    let ui = about_page(empty.path(), Services::for_tests());
    ui.get_by_label("Backups are kept in each settings profile's folder, in clone-bay-backups.");
    assert!(!shows(&ui, "Open the backup folder"));
}
