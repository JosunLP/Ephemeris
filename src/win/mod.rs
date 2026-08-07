// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The Windows front end: Direct2D renderer, Win32 window, Windows host.
//!
//! Everything below this module may call the operating system freely. What it
//! exposes upwards is the small set of functions in [`crate`]'s front end
//! contract — install the host, take the single-instance lock, run, report a
//! fatal error — so `main` never learns which platform it was built for.

mod host_impl;
mod platform;
mod render;
mod secure;
mod window;

use std::sync::Arc;
use tpmplaner_core::host;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE, MessageBoxW};
use windows::core::PCWSTR;

/// Hands the portable core its Windows implementations.
pub fn install_host() {
    host::set_host(Arc::new(host_impl::WindowsHost));
    host::set_locale_backend(Arc::new(host_impl::WindowsLocale));
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

/// The widget has no console, so a message box is the only way to say
/// anything at all before exiting.
pub fn fatal(message: &str) {
    let text = platform::wide(message);
    let title = platform::wide("TPMPlaner");
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MESSAGEBOX_STYLE(MB_OK.0 | MB_ICONERROR.0),
        );
    }
}
