// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! A small file log.
//!
//! The status line in the widget has room for roughly 56 characters, nowhere
//! near enough for a provider's error message. Without a log, diagnosing a
//! problem in the field is guesswork, so the full text also goes to
//! `tpmplaner.log` in the data directory, reachable from the context menu.
//!
//! Deliberately without a logging crate: a `Mutex<File>` and rotation by size
//! are all this needs.

use crate::config;
use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Rotates to `.1` beyond this size. Two files of 256 KB cover several days
/// of operation and are unnoticeable on any disk.
const MAX_BYTES: u64 = 256 * 1024;

fn path() -> PathBuf {
    config::data_dir().join("tpmplaner.log")
}

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Writes one line. Errors are swallowed: a log that cannot be written must
/// never stop the widget.
pub fn write(level: &str, message: &str) {
    let _guard = lock().lock();
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= MAX_BYTES {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        // One line per entry with fixed column widths, so the log can be
        // filtered with ordinary tools.
        let _ = writeln!(
            file,
            "{} {:<5} {}",
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            level,
            message.replace('\n', " | ")
        );
    }
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn warn(message: &str) {
    write("WARN", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

pub fn file_path() -> PathBuf {
    path()
}

/// Writes every panic to the log before the process ends.
///
/// The widget runs without a console and with `panic = "abort"`. Without this
/// hook it simply vanishes from the desktop when something goes wrong: no
/// message, no trace, and nobody can say why. The hook runs before the abort
/// and costs nothing in normal operation.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(unknown cause)".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "(unknown location)".to_string());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        error(&format!(
            "PANIC in thread '{thread}' at {location}: {payload}"
        ));
        previous(info);
    }));
}
