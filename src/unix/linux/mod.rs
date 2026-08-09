// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The Linux front end: an X11 window with a Cairo renderer.
//!
//! | | |
//! |---|---|
//! | [`dl`] | Opening the desktop's libraries at run time |
//! | [`ffi`] | The slice of Xlib, Cairo and Pango that is used |
//! | [`window`] | The window, the event loop and the shell |
//! | [`canvas`] | Cairo and Pango behind [`crate::unix::canvas::Canvas`] |
//! | [`menu`] | The right-click menu, drawn — X11 has none |
//! | [`visuals`] | Appearance through the XDG settings portal |
//!
//! ## Wayland
//!
//! This is an X11 front end, and under a Wayland session it runs through
//! XWayland. That works — the widget appears, draws, and everything except its
//! place in the stacking order behaves — but whether `_NET_WM_STATE_BELOW`
//! reaches the compositor is the compositor's business, and several ignore it
//! for XWayland surfaces.
//!
//! The decision recorded in `docs/development/porting.md` was **X11 and
//! `wlr-layer-shell`, and where neither is available the widget says what it
//! cannot do rather than pretending**. The first half is here; the layer-shell
//! back end is not, and until it is, a Wayland session gets the honest half:
//! [`report_session`] writes what was detected and what it means into the log
//! at every start, so "the widget is not staying behind my windows" has an
//! answer in the file users are asked to attach rather than being a mystery.
//!
//! Silently degrading was rejected for the same reason it was rejected in the
//! decision record: a widget that quietly stops doing the one thing it is for
//! is worse than one that says so.

pub mod canvas;
pub mod dl;
pub mod ffi;
pub mod menu;
pub mod visuals;
pub mod window;

use tpmplaner_core::log;

/// Is there a display to open a window on?
///
/// `DISPLAY` covers X11 and XWayland alike; a Wayland session without
/// XWayland has `WAYLAND_DISPLAY` and no `DISPLAY`, and there is nothing this
/// front end can do there, so it falls through to the printed agenda.
pub fn has_display() -> bool {
    if std::env::var_os("DISPLAY").is_some() {
        return true;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        log::warn(
            "This is a Wayland session without XWayland. The widget needs an X11 connection, \
             so the agenda is printed instead. See the porting notes.",
        );
    }
    false
}

pub fn run() -> Result<(), String> {
    window::run()
}

/// Records what kind of session this is, and what follows from it.
///
/// Called once the window is up. Under X11 there is nothing to warn about;
/// under XWayland there is, and the log is where somebody looking for the
/// reason will be.
pub fn report_session() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    if wayland || session == "wayland" {
        log::warn(
            "Running under XWayland. Whether the widget stays below other windows is up to \
             the compositor, and several ignore _NET_WM_STATE_BELOW for X11 clients — the \
             widget may behave as an ordinary window. A native Wayland back end \
             (wlr-layer-shell) is still to be written.",
        );
    } else {
        log::info("X11 session — the widget is stacked below other windows.");
    }
}

/// Only one widget per display.
///
/// A second one would draw an identical window over the first and synchronise
/// in parallel. The lock is an abstract socket, which is a Linux feature worth
/// using here: it has no path, so nothing is left behind in `/tmp` for the
/// next start to trip over, and the kernel releases it when the process ends
/// however it ends — including a kill that no cleanup code would survive.
///
/// The name carries the display, so one widget per X display rather than one
/// per machine: two sessions on one host are two desktops.
pub fn acquire_single_instance() -> bool {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};

    let display = std::env::var("DISPLAY").unwrap_or_default();
    let name = format!("tpmplaner-{display}");
    let Ok(address) = SocketAddr::from_abstract_name(name.as_bytes()) else {
        // Too long a name to be an abstract address, which means no lock —
        // and refusing to start would be the worse mistake.
        return true;
    };
    match UnixListener::bind_addr(&address) {
        Ok(listener) => {
            // Held for the life of the process. Leaked rather than stored,
            // because there is nothing that would ever want to release it: the
            // kernel does that at exit.
            std::mem::forget(listener);
            true
        }
        Err(_) => {
            log::info("Another copy is already on this display");
            false
        }
    }
}
