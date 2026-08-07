// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! TPMPlaner — a desktop widget for calendar events and due tasks.
//!
//! This file knows nothing about any operating system. Everything portable —
//! the model, the calendar back ends, synchronisation, localisation and the
//! palette — lives in `tpmplaner-core`; everything platform-specific lives in
//! one front end module, selected below, and reaches the rest of the binary
//! only through the four functions this file calls.
//!
//! **The front end contract.** A platform module provides exactly these:
//!
//! | | |
//! |---|---|
//! | `install_host()` | Give the core its [`Host`] and [`LocaleBackend`]. Runs before anything touches a path, a secret or a date format. |
//! | `acquire_single_instance() -> bool` | `false` if another copy already owns the desktop. |
//! | `run() -> Result<(), String>` | The event loop. Returns only when the widget is finished, or with the reason it could not start. |
//! | `fatal(&str)` | Say why, to a user who may have no console. |
//!
//! Windows has a complete implementation. macOS and Linux have the host and a
//! text front end, which is the portable half working and the interface layer
//! still to be written — see `docs/development/porting.md`.
//!
//! [`Host`]: tpmplaner_core::host::Host
//! [`LocaleBackend`]: tpmplaner_core::host::LocaleBackend
//!
//! On Windows there is no console window: the widget is a pure graphical
//! application. The text front end on the other platforms needs one, so the
//! attribute is conditional rather than unconditional.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as frontend;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as frontend;

use tpmplaner_core::log;

fn main() {
    // First of all: without the hook the widget vanishes from the desktop
    // without a word when something goes wrong.
    log::install_panic_hook();

    // Before anything touches a path, a secret or a date format.
    frontend::install_host();

    if !frontend::acquire_single_instance() {
        return;
    }

    if let Err(e) = frontend::run() {
        // The language is already settled: `run` sets it first thing.
        frontend::fatal(&format!(
            "{}\n\n{e}",
            tpmplaner_core::i18n::global().fatal_start
        ));
    }
}
