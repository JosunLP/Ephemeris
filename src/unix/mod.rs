// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The macOS and Linux front end.
//!
//! [`host`] gives the core its data directory, its browser, its random bytes
//! and its credential store; [`locale`] gives it the platform's own date and
//! time database; [`secure`] puts the refresh token in the Keychain or the
//! Secret Service. On top of that sits the widget itself, split three ways:
//!
//! | | |
//! |---|---|
//! | [`app`] | What the widget does — input, timers, commands. Shared. |
//! | [`paint`] | What the widget looks like, against [`canvas::Canvas`]. Shared. |
//! | [`mac`] / [`linux`] | The window, the renderer behind that trait, and the event loop. |
//!
//! [`text`] is what runs when there is no display to open a window on: a
//! headless container, an SSH session, a continuous-integration runner. It
//! prints the agenda and exits, which is a useful thing in its own right and
//! is what CI smoke-tests the portable half with.
//!
//! Choosing between them is a runtime question rather than a build-time one.
//! The same binary has to work over SSH and on the desktop of the machine it
//! is compiled on, and a widget that aborted because `DISPLAY` was unset would
//! be no use in either place.

mod app;
mod autostart;
mod canvas;
#[cfg(target_os = "macos")]
mod cf;
mod host;
#[cfg(target_os = "linux")]
mod linux;
mod locale;
#[cfg(target_os = "macos")]
mod mac;
mod paint;
mod secure;
mod text;

use std::sync::Arc;
use tpmplaner_core::host as core_host;
use tpmplaner_core::log;

pub fn install_host() {
    // Before the first write, so the settings and the credential file land in
    // a directory only their owner can read.
    host::prepare_data_dir();
    core_host::set_host(Arc::new(host::UnixHost));

    // The platform's own locale database: Core Foundation's date formatter on
    // macOS, the C library's on Linux. What it replaced formatted every locale
    // as `4 August 2026` on a twenty-four-hour clock — right for no one in
    // particular.
    core_host::set_locale_backend(Arc::new(locale::UnixLocale));
}

/// A second copy would draw an identical window over the first and
/// synchronise in parallel.
///
/// Only asked where there is a window to collide over. The text front end
/// prints once and exits; two of those share the agenda cache, and what that
/// could corrupt — a half-written file — is prevented in `sync::write_cache`
/// instead, which replaces the file by rename rather than truncating it in
/// place. What remains is a duplicated fetch: wasteful, not harmful, and not
/// worth a lock file for a program that exits in seconds.
pub fn acquire_single_instance() -> bool {
    if !windowed() {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        mac::acquire_single_instance()
    }
    #[cfg(target_os = "linux")]
    {
        linux::acquire_single_instance()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        true
    }
}

pub fn run() -> Result<(), String> {
    if !windowed() {
        return text::run();
    }
    #[cfg(target_os = "macos")]
    {
        mac::window::run()
    }
    #[cfg(target_os = "linux")]
    {
        linux::run()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        text::run()
    }
}

/// Is there a desktop to put a window on?
///
/// `TPMPLANER_TEXT=1` forces the text front end whatever the answer, which is
/// what makes the printed agenda usable as a command on a machine that does
/// have a display — and what lets continuous integration exercise it on a
/// runner that has one.
fn windowed() -> bool {
    if std::env::var_os("TPMPLANER_TEXT").is_some_and(|v| v != "0") {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let yes = mac::has_window_server();
        if !yes {
            log::info("No window server in this session — printing the agenda instead");
        }
        yes
    }
    #[cfg(target_os = "linux")]
    {
        let yes = linux::has_display();
        if !yes {
            log::info("No display in this session — printing the agenda instead");
        }
        yes
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Tells a copy that is already running to come forward for a moment.
///
/// False when nothing was listening, which is the ordinary answer when the
/// widget is not running at all.
///
/// Only Linux can do this, and only Linux needs to: it is how the peek
/// shortcut works under Wayland, where the compositor owns every keybinding
/// and a client may not grab one. macOS registers its own shortcut through
/// Carbon and has nothing to ask.
pub fn peek_running_instance() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::send_peek()
    }
    #[cfg(not(target_os = "linux"))]
    {
        log::warn("--peek is only available on Linux");
        false
    }
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
///
/// A widget started from the Dock or an autostart entry has no console to read
/// it, so the message goes to the log as well — that is the file a bug report
/// is asked for, and a start-up failure is precisely what it should contain.
pub fn fatal(message: &str) {
    use std::io::Write;
    log::error(message);
    let _ = writeln!(std::io::stderr(), "{message}");
}
