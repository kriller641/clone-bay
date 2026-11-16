//! Drawing the About page. All decisions live in `about_flow`.

use std::sync::LazyLock;

use eframe::egui;

use crate::about_flow::{AboutFlow, NO_BACKUPS, REPOSITORY, VERSION};
use crate::theme;
use crate::widgets::{dot, footer, problem_line, section};

/// What the user asked for on this frame that the caller has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    None,
    /// Leave the page and go back to the file list.
    Close,
}

/// The app's one-line description, as in the README.
const DESCRIPTION: &str =
    "Copy and edit EVE Online settings files. One portable Windows exe, nothing to install.";

/// What the app does, a line each: the words that stand out, then the
/// rest. Keep it in step with "What it does" in the README.
const FEATURES: [(&str, &str); 8] = [
    (
        "Copies settings",
        " from one character or account onto others.",
    ),
    (
        "Always keeps your module layouts and drone groups.",
        " Every file you copy onto keeps its own module layouts, weapon groups and drone \
         groups, so nothing about your ships is overwritten.",
    ),
    (
        "You choose what else is copied:",
        " the overview, keyboard shortcuts, window layout, chat channels, sound, camera and \
         more, ticked one by one, or everything at once.",
    ),
    (
        "Edits individual settings,",
        " such as orbit and warp-to distances, scanning, the autopilot, camera and sound.",
    ),
    ("Edits probe formations", " and shares them with friends."),
    ("Puts overview and chat tabs in order.", ""),
    ("Restores a file", " from one of its backups."),
    (
        "Updates itself",
        " with one click when a new version is out.",
    ),
];

/// Why the app is safe to use, a line each.
const SAFETY: [&str; 3] = [
    "It won't change anything while EVE is running. Close EVE first.",
    "Every file is backed up before it is changed, and a copy is checked before anything is \
     written.",
    "Anything it can't read with certainty is left alone.",
];

/// What goes online, as in the README.
const ONLINE: &str = "Character, ship and system names are looked up from CCP's public API, and \
                      once a day it checks GitHub for a newer version. Nothing else leaves your \
                      PC.";

/// Who the app builds on: the name that stands out, then the rest. Keep it
/// in step with "Credits" in the README.
const CREDITS: [(&str, &str); 6] = [
    (
        "CCP hf.",
        ": ship names from the Static Data Export and character names from ESI, under CCP's \
         Developer License Agreement.",
    ),
    (
        "carbonengine/blue",
        " by CCP Games (MIT): the settings file format.",
    ),
    (
        "blue-marshal-rs",
        " by TrueBrain (MIT): the file format code follows it.",
    ),
    (
        "EVE Wrench",
        " by Tim Kunze (MIT): a guide to what the settings mean.",
    ),
    ("EANM", " by Bertral (MIT): where this project started."),
    ("egui", " and the other libraries built into the app."),
];

const CCP_NOTICE: &str = "© 2014 CCP hf. All rights reserved. \"EVE\", \"EVE Online\", \"CCP\", \
                          and all related logos and images are trademarks or registered \
                          trademarks of CCP hf. Clone Bay is a third-party tool. It is not made, \
                          endorsed or supported by CCP hf.";

/// The app's own licence and the notices of everything built into it, so
/// they travel with the exe even when it is passed on alone.
const LICENCE: &str = include_str!("../../../LICENSE");
const NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.txt");

/// The licence texts a line each, split once.
static LICENCE_LINES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    LICENCE
        .lines()
        .chain(std::iter::once(""))
        .chain(NOTICES.lines())
        .collect()
});

/// The height of the box the licence texts scroll in.
const LICENCE_BOX_HEIGHT: f32 = 320.0;

pub fn draw(ui: &mut egui::Ui, flow: &mut AboutFlow) -> Request {
    let mut request = Request::None;
    // The button first, so it keeps its place at the bottom (see `footer`).
    footer(ui, |ui| {
        problem_line(ui, flow.problem.as_deref());
        if ui.button("Back to the file list").clicked() {
            request = Request::Close;
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("about")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(egui::RichText::new("About Clone Bay").strong());
            ui.add_space(6.0);
            section(ui, None, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Clone Bay").size(18.0).strong());
                    ui.weak(format!("Version {VERSION}"));
                });
                ui.label(DESCRIPTION);
                ui.add_space(4.0);
                if ui
                    .button("Open the GitHub page")
                    .on_hover_text(REPOSITORY)
                    .clicked()
                {
                    flow.open_github();
                }
            });
            ui.columns(2, |columns| {
                what_it_does(&mut columns[0]);
                why_safe(&mut columns[0]);
                backups(&mut columns[1], flow);
                credits(&mut columns[1]);
            });
        });
    request
}

/// A line whose first words stand out.
fn lead_line(ui: &mut egui::Ui, lead: &str, rest: &str) {
    let mut job = egui::text::LayoutJob::default();
    let style = ui.style();
    egui::RichText::new(lead).strong().append_to(
        &mut job,
        style,
        egui::FontSelection::Default,
        egui::Align::Center,
    );
    egui::RichText::new(rest).append_to(
        &mut job,
        style,
        egui::FontSelection::Default,
        egui::Align::Center,
    );
    ui.label(job);
}

fn what_it_does(ui: &mut egui::Ui) {
    section(ui, Some("What it does"), |ui| {
        for (lead, rest) in FEATURES {
            lead_line(ui, lead, rest);
            ui.add_space(3.0);
        }
    });
}

fn why_safe(ui: &mut egui::Ui) {
    section(ui, Some("Why it's safe"), |ui| {
        for line in SAFETY {
            ui.horizontal_wrapped(|ui| {
                dot(ui, theme::OK);
                ui.label(line);
            });
            ui.add_space(3.0);
        }
        ui.weak(ONLINE);
    });
}

fn backups(ui: &mut egui::Ui, flow: &mut AboutFlow) {
    section(ui, Some("Your backups"), |ui| {
        let (Some(profile), Some(folder)) = (&flow.profile_name, &flow.backups_folder) else {
            ui.label("Backups are kept in each settings profile's folder, in clone-bay-backups.");
            return;
        };
        ui.label(format!("Backups for the profile {profile} are kept in:"));
        ui.label(egui::RichText::new(folder.display().to_string()).monospace());
        ui.add_space(4.0);
        let mut open = false;
        ui.horizontal_wrapped(|ui| {
            open = ui
                .add_enabled(
                    flow.has_backups(),
                    egui::Button::new("Open the backup folder"),
                )
                .on_hover_text("Shows the folder in File Explorer")
                .on_disabled_hover_text(NO_BACKUPS)
                .clicked();
            if flow.has_backups() {
                ui.weak(flow.backups_line());
            }
        });
        if !flow.has_backups() {
            ui.weak(flow.backups_line());
        }
        ui.add_space(4.0);
        ui.weak(
            "Each backup is a file as it was before Clone Bay changed it. Clone Bay never \
             deletes them. To undo a change, select the file in the list and choose \"Restore \
             from a backup\".",
        );
        if open {
            flow.open_backups();
        }
    });
}

fn credits(ui: &mut egui::Ui) {
    section(ui, Some("Credits"), |ui| {
        for (lead, rest) in CREDITS {
            lead_line(ui, lead, rest);
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Full licence texts")
            .id_salt("licences")
            .show(ui, |ui| {
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                egui::ScrollArea::both()
                    .id_salt("licence texts")
                    .max_height(LICENCE_BOX_HEIGHT)
                    .auto_shrink([false, true])
                    .show_rows(ui, row_height, LICENCE_LINES.len(), |ui, range| {
                        for line in &LICENCE_LINES[range] {
                            ui.add(
                                egui::Label::new(egui::RichText::new(*line).monospace()).extend(),
                            );
                        }
                    });
            });
        ui.add_space(4.0);
        ui.weak(CCP_NOTICE);
        ui.weak("Clone Bay is free software under the MIT licence.");
    });
}
