// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! TPMPlaner — a desktop widget for calendar events and due tasks.
//!
//! This binary is the Windows front end: the Direct2D renderer, the Win32
//! window and the operating system glue. Everything portable — the model, the
//! calendar back ends, synchronisation, localisation and the palette — lives
//! in `tpmplaner-core` and is shared with the other platforms.
//!
//! No console window: the widget is a pure graphical application.
#![windows_subsystem = "windows"]

mod host_impl;
mod platform;
mod render;
mod secure;
mod window;

use std::sync::Arc;
use tpmplaner_core::{host, log};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE, MessageBoxW};
use windows::core::PCWSTR;

fn main() {
    // First of all: without the hook the widget vanishes from the desktop
    // without a word when something goes wrong.
    log::install_panic_hook();

    // Hand the portable core its operating system implementations before
    // anything else touches a path, a secret or a date format.
    host::set_host(Arc::new(host_impl::WindowsHost));
    host::set_locale_backend(Arc::new(host_impl::WindowsLocale));

    // A second start would put an identical window on top of the first; both
    // would draw and synchronise in parallel.
    if !platform::acquire_single_instance() {
        return;
    }

    unsafe {
        // ShellExecuteW (opening a browser) expects an initialised COM
        // apartment on the calling thread.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    if let Err(e) = window::run() {
        // A failure building the graphics chain is the one thing that can
        // genuinely stop the widget — then at least say why.
        // The language is already settled: `run` sets it first thing.
        fatal(&format!(
            "{}

{e}",
            tpmplaner_core::i18n::global().fatal_start
        ));
    }
}

fn fatal(message: &str) {
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
