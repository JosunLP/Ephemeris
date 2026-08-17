// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The Windows front end: Direct2D renderer, Win32 window, Windows host.
//!
//! What the widget *looks* like is not here — that is [`crate::paint`], shared
//! with the macOS and Linux front ends. [`canvas`] is Direct2D's implementation
//! of the trait it draws against, and [`render`] is the device chain behind it.
//!
//! Everything below this module may call the operating system freely. What it
//! exposes upwards is the small set of functions in [`crate`]'s front end
//! contract — install the host, take the single-instance lock, run, report a
//! fatal error — so `main` never learns which platform it was built for.

mod canvas;
mod host_impl;
mod platform;
mod render;
mod secure;
mod window;

use ephemeris_core::host;
use std::sync::Arc;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE, MessageBoxW};
use windows::core::PCWSTR;

/// Hands the portable core its Windows implementations.
pub fn install_host() {
    host::set_host(Arc::new(host_impl::WindowsHost));
    host::set_locale_backend(Arc::new(host_impl::WindowsLocale));
}

/// Moves the autostart entry across the program's rename. See
/// [`crate::migrate`].
pub fn migrate_autostart_entry() {
    platform::migrate_autostart_entry();
}

/// A second start would put an identical window on top of the first; both
/// would draw and synchronise in parallel.
pub fn acquire_single_instance() -> bool {
    platform::acquire_single_instance()
}

pub fn run() -> Result<(), String> {
    unsafe {
        // ShellExecuteW (opening a browser) expects an initialised COM
        // apartment on the calling thread.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    // A failure building the graphics chain is the one thing that can
    // genuinely stop the widget.
    window::run().map_err(|e| e.to_string())
}

/// Windows has no need of this: the widget registers its own global shortcut
/// with `RegisterHotKey`, so there is nobody to ask.
///
/// It is part of the front end contract because Linux does need it — a Wayland
/// compositor owns every keybinding — and a contract with a hole in it for one
/// platform is not one `main` can call without knowing which platform it is on.
pub fn peek_running_instance() -> bool {
    false
}

/// The widget has no console, so a message box is the only way to say
/// anything at all before exiting.
pub fn fatal(message: &str) {
    let text = platform::wide(message);
    let title = platform::wide("Ephemeris");
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MESSAGEBOX_STYLE(MB_OK.0 | MB_ICONERROR.0),
        );
    }
}
