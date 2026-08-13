// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The portable half of TPMPlaner.
//!
//! Everything here compiles and behaves identically on Windows, macOS and
//! Linux: the data model, the calendar back ends, the synchronisation
//! schedule, localisation, the colour palette and the update check.
//!
//! Nothing in this crate calls an operating system API. The few things that
//! cannot be written once — the settings directory, secret storage, opening a
//! browser, locale-aware date formatting — go through the traits in [`host`],
//! which the application supplies at start-up.
//!
//! That boundary is enforced by continuous integration: the crate is type
//! checked for Linux and macOS on every push, so a stray platform call cannot
//! creep back in unnoticed.

pub mod anim;
pub mod config;
pub mod demo;
pub mod google;
pub mod host;
pub mod hotkey;
pub mod i18n;
pub mod layout;
pub mod log;
pub mod menu;
pub mod model;
pub mod provider;
pub mod sync;
pub mod theme;
pub mod update;
