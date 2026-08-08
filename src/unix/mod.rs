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
/// Not quite "nothing to collide over": this front end prints once and exits,
/// but two copies still share the agenda cache, and a timer firing over a slow
/// sync is enough to overlap them. What that collision could corrupt —
/// a half-written cache file — is prevented in `sync::write_cache` instead,
/// which replaces the file by rename rather than truncating it in place. What
/// remains is a duplicated fetch: wasteful, not harmful, and not worth a lock
/// file for a program that exits in seconds.
///
/// A long-lived window is the case that does need this: a second one would
/// draw over the first and synchronise in parallel. So the real implementation
/// is due at the same time as the window — an abstract socket on Linux, a
/// `flock` on a pid file where that is not available, and
/// `NSRunningApplication` on macOS.
pub fn acquire_single_instance() -> bool {
    true
}

pub fn run() -> Result<(), String> {
    text::run()
}

/// There is a console here, which is exactly why the Windows front end needs a
/// message box and this does not.
///
/// `writeln!` rather than `eprintln!`, which panics if stderr is gone —
/// `tpmplaner 2>&1 | head -1` reaches that, and so does a supervisor that
/// closed the inherited handle. [`text::run`] takes the same care with stdout
/// for the same reason: a panic report in the log users are asked to attach is
/// worse than no message, and this is the one path that only runs when
/// something has already gone wrong.
pub fn fatal(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{message}");
}
