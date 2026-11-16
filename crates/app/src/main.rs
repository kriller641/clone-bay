//! Clone Bay: a desktop app for EVE Online settings files.
//!
//! This crate is only the window. Everything that reads a settings file
//! lives in `clone-bay-core`.

#![forbid(unsafe_code)]
// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about_flow;
#[cfg(test)]
mod about_ui_tests;
mod about_view;
mod app;
mod copy_flow;
#[cfg(test)]
mod copy_ui_tests;
mod copy_view;
mod edit_flow;
#[cfg(test)]
mod edit_ui_tests;
mod edit_view;
mod formation_flow;
#[cfg(test)]
mod formation_ui_tests;
mod formation_view;
mod launch;
mod list_model;
mod loader;
#[cfg(test)]
mod names_ui_tests;
mod restore_flow;
#[cfg(test)]
mod restore_ui_tests;
mod restore_view;
mod saving;
mod services;
mod state;
mod tab_flow;
#[cfg(test)]
mod tab_ui_tests;
mod tab_view;
mod theme;
mod update_flow;
#[cfg(test)]
mod update_ui_tests;
mod viewer;
mod widgets;
mod window_fit;

use std::time::Duration;

use clone_bay_core::platform::{app_data_dir, claim_single_window_waiting, SingleWindow};
use eframe::egui;

const APP_NAME: &str = "Clone Bay";

/// The window's icon, as unpacked pixels (`tools/make_icon.py` writes them
/// from `assets/icon.png`). The exe's own icon is built in by `build.rs`.
const ICON_RGBA: &[u8] = include_bytes!("../assets/icon.rgba");
const ICON_SIDE: u32 = 64;

fn icon() -> egui::IconData {
    egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: ICON_SIDE,
        height: ICON_SIDE,
    }
}

fn main() -> eframe::Result {
    let (updated_from, folder) = match launch::parse(std::env::args_os().skip(1)) {
        // Proves this exe runs, for the version about to be replaced by it.
        // No window, no lock, nothing read or written.
        launch::Launch::SelfTest(wanted) => std::process::exit(launch::self_test_code(&wanted)),
        launch::Launch::Open {
            updated_from,
            folder,
        } => (updated_from, folder),
    };
    // Held until the app ends. If it can't be checked the app opens anyway:
    // a second window only means a second, harmless set of backups. A
    // version that has just replaced another waits for that one to close.
    let patience = if updated_from.is_some() {
        Duration::from_secs(10)
    } else {
        Duration::ZERO
    };
    let _window = match app_data_dir().map(|folder| claim_single_window_waiting(&folder, patience))
    {
        Some(SingleWindow::AlreadyOpen) => return show_already_open(),
        other => other,
    };
    // What an update left beside the exe. The replaced exe can't be
    // deleted until its window has fully closed, so this keeps trying for
    // a few seconds without holding up the window.
    if let Ok(exe) = std::env::current_exe() {
        std::thread::spawn(move || {
            clone_bay_core::install::clean_leftovers(&exe, 20, Duration::from_millis(250));
        });
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size(app::DEFAULT_WINDOW_SIZE)
            .with_min_inner_size(app::MIN_WINDOW_SIZE)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |creation| {
            Ok(Box::new(app::App::new(
                &creation.egui_ctx,
                folder,
                updated_from,
            )))
        }),
    )
}

/// A small window saying another window of the app is already open.
fn show_already_open() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size(app::ALREADY_OPEN_WINDOW_SIZE)
            .with_resizable(false)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|creation| Ok(Box::new(app::AlreadyOpen::new(&creation.egui_ctx)))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_pixels_fill_a_square_of_the_stated_side() {
        let side = super::ICON_SIDE as usize;
        assert_eq!(super::ICON_RGBA.len(), side * side * 4);
    }
}
