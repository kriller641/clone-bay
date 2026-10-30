//! Clone Bay: a desktop app for EVE Online settings files.
//!
//! This crate is only the window. Everything that reads a settings file
//! lives in `clone-bay-core`.

#![forbid(unsafe_code)]
// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
mod list_model;
mod loader;
#[cfg(test)]
mod names_ui_tests;
mod saving;
mod services;
mod state;
mod tab_flow;
#[cfg(test)]
mod tab_ui_tests;
mod tab_view;
mod theme;
mod viewer;
mod widgets;

use std::path::PathBuf;

use eframe::egui;

const APP_NAME: &str = "Clone Bay";

fn main() -> eframe::Result {
    // An optional folder to open instead of the detected one.
    let folder = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size(app::DEFAULT_WINDOW_SIZE)
            .with_min_inner_size([720.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |creation| Ok(Box::new(app::App::new(&creation.egui_ctx, folder)))),
    )
}
