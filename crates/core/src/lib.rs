//! Finding, reading and safely changing EVE Online settings files.
//!
//! The UI never touches a settings file directly. The only code that writes
//! one is in [`safe_write`]. Its callers are [`copy`], [`edit`] (which
//! [`formations`] and [`tab_order`] save through) and [`prefs`], and every
//! write goes through a [`safe_write::BackupSession`].

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod bundled;
pub mod copy;
pub mod discovery;
pub mod edit;
pub mod formation_share;
pub mod formations;
pub mod groups;
pub mod inspect;
pub mod names;
pub mod platform;
pub mod prefs;
pub mod protect;
pub mod restore;
pub mod safe_write;
pub mod tab_order;
