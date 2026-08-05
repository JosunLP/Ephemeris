//! TPMPlaner — Desktop-Widget fuer Google Kalender und Google Tasks.
//!
//! Kein Konsolenfenster: das Widget ist eine reine GUI-Anwendung.
#![windows_subsystem = "windows"]

mod anim;
mod config;
mod demo;
mod google;
mod i18n;
mod log;
mod model;
mod platform;
mod render;
mod secure;
mod sync;
mod theme;
mod window;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE, MessageBoxW};
use windows::core::PCWSTR;

fn main() {
    // Als Allererstes: ohne den Haken verschwindet das Widget bei einem
    // Fehler wortlos vom Desktop.
    log::install_panic_hook();

    // Ein zweiter Start wuerde ein deckungsgleiches Fenster auf das erste
    // legen; beide zeichnen und synchronisieren dann parallel.
    if !platform::acquire_single_instance() {
        return;
    }

    unsafe {
        // ShellExecuteW (Browser oeffnen) erwartet ein initialisiertes
        // COM-Apartment auf dem aufrufenden Thread.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    if let Err(e) = window::run() {
        // Ein Fehler beim Aufbau der Grafikkette ist das einzige, was das
        // Widget wirklich stoppen kann — dann wenigstens sagen, warum.
        // Die Sprache steht hier bereits fest: `run` setzt sie als Erstes.
        fatal(&format!("{}\n\n{e}", i18n::global().fatal_start));
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
