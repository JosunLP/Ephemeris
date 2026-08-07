// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The macOS and Linux front end.
//!
//! Half of one. [`host`] is real — the data directory, opening a browser and
//! cryptographic random bytes all work, and the two credential methods say
//! plainly that they do not yet. [`text`] prints the agenda instead of drawing
//! it, because the window this project is built around does not exist on these
//! platforms yet.
//!
//! That is deliberate rather than a placeholder nobody got round to replacing.
//! The window has to sit below every other window and above the desktop,
//! refuse focus, stay out of the taskbar and the window switcher, and follow
//! the system's appearance and accessibility settings at runtime. Which API
//! provides that is a different answer on macOS, on X11, on wlroots and on
//! GNOME, and picking between them is a decision with consequences that ought
//! to be written down before code is written rather than discovered halfway.
//! It is written down: `docs/development/porting.md`.
//!
//! What exists here is enough to prove the boundary holds. Continuous
//! integration builds this binary on Ubuntu and macOS and runs it, so the
//! portable core is exercised end to end on both rather than merely
//! type checked.

mod host;
mod text;

use std::sync::Arc;
use tpmplaner_core::host as core_host;

pub fn install_host() {
    // Before the first write, so the settings and the credential file land in
    // a directory only their owner can read.
    host::prepare_data_dir();
    core_host::set_host(Arc::new(host::UnixHost));

    // The locale backend is left at the portable default on purpose. It
    // formats a date as `4 August 2026` in every locale, which is wrong
    // everywhere except by accident — and writing a slightly less wrong one by
    // hand is the trap `i18n.rs` exists to warn about. The right answer is the
    // platform's own database (`NSDateFormatter` with `dateFormatFromTemplate:`
    // on macOS) or `icu4x` on Linux, and both belong with the front end that
    // needs them. See the porting notes.
}

/// Always true for now.
///
/// There is nothing to collide over yet: this front end prints once and exits.
/// A second copy of a long-lived window would draw over the first and
/// synchronise in parallel, so the real implementation is needed at the same
/// time as the window — an abstract socket on Linux, a `flock` on a pid file
/// where that is not available, and `NSRunningApplication` on macOS.
pub fn acquire_single_instance() -> bool {
    true
}

pub fn run() -> Result<(), String> {
    text::run()
}

/// There is a console here, which is exactly why the Windows front end needs a
/// message box and this does not.
pub fn fatal(message: &str) {
    eprintln!("{message}");
}
