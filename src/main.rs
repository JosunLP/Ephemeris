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
//! | `peek_running_instance() -> bool` | Tell a copy that is already running to come forward. |
//!
//! All three are complete: Direct2D in a Win32 window, Core Graphics in an
//! `NSWindow`, and Cairo in a Wayland or X11 surface. Where there is no
//! desktop at all the agenda is printed instead of drawn. See
//! `docs/development/porting.md`.
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

/// The exit status is part of the contract now that one of the front ends is a
/// command. On Windows nothing ever read it — `fatal` puts up a message box and
/// the process is started from a shortcut — but `tpmplaner || notify-send …`, a
/// systemd unit and the smoke test in continuous integration all read it, and a
/// program that could not start must not report success to them.
///
/// Another copy already owning the desktop is the one early return that is
/// *not* a failure: nothing went wrong, this copy simply has nothing to do.
fn main() -> std::process::ExitCode {
    // First of all: without the hook the widget vanishes from the desktop
    // without a word when something goes wrong.
    log::install_panic_hook();

    // Before anything touches a path, a secret or a date format.
    frontend::install_host();

    // `tpmplaner --peek` is not a second widget: it is one message to the one
    // already running, and then it exits. A Wayland compositor owns every
    // keyboard shortcut and will not let a client grab one, so a keybinding
    // running this is the only way the peek shortcut can work there — and it
    // works on X11 and on the other two systems for the same asking.
    if std::env::args().skip(1).any(|a| a == "--peek") {
        return match frontend::peek_running_instance() {
            true => std::process::ExitCode::SUCCESS,
            // A shell script binding a key wants to know, and a service
            // manager reads it: no widget was listening.
            false => std::process::ExitCode::FAILURE,
        };
    }

    if !frontend::acquire_single_instance() {
        return std::process::ExitCode::SUCCESS;
    }

    if let Err(e) = frontend::run() {
        // The language is already settled: `run` sets it first thing.
        frontend::fatal(&format!(
            "{}\n\n{e}",
            tpmplaner_core::i18n::global().fatal_start
        ));
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
