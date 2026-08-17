// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Carrying an existing installation across the rename.
//!
//! The widget was called something else until this release, and its former
//! name is baked into more than the binary: the settings directory, the log,
//! the autostart entry the session reads at login and — on Windows — the
//! entropy the stored refresh tokens were encrypted with. A rename that
//! ignored all that would look, from the user's side, exactly like a fresh
//! install: an empty widget asking to be connected to a calendar again, while
//! the old settings and the old autostart entry sat somewhere unreferenced.
//!
//! So this runs once, at the first start under the new name, and moves what it
//! finds. It is deliberately conservative:
//!
//! * Nothing is ever overwritten. An entry that already exists under the new
//!   name is left alone and the old one is left where it is.
//! * The old directory is only removed once it is empty, so anything that
//!   could not be moved stays where the user can still find it.
//! * Every step is best-effort and logged. A failure here must not stop the
//!   widget from starting — the worst case is the fresh-install experience,
//!   which is still a working widget.
//!
//! Once no installation predating the rename is plausible, this module and its
//! two callers in the front ends can go.

use ephemeris_core::log;
use std::path::{Path, PathBuf};

/// The lower-case form of the former name, which is what the file names inside
/// the data directory were built from.
const LEGACY_PREFIX: &str = "tpmplaner";

/// Moves an installation made under the former name to the current one.
///
/// Call once at start-up, after the host is installed — the destination comes
/// from it — and after the single-instance lock is held, so two copies racing
/// to start cannot both move the same directory.
pub fn from_legacy_name() {
    if let Some(old) = legacy_data_dir() {
        move_data_dir(&old, &ephemeris_core::config::data_dir());
    }
    crate::frontend::migrate_autostart_entry();
}

/// Where the settings lived under the former name, if that directory is still
/// there.
///
/// Mirrors [`ephemeris_core::host::PortableHost::data_dir`] rule for rule: the
/// same environment variables in the same order, with the same fallbacks. The
/// two have to agree, because the whole point is to find the directory the
/// previous version would have written.
fn legacy_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Some(PathBuf::from(appdata).join("TPMPlaner"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home).join("Library/Application Support/TPMPlaner"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join(LEGACY_PREFIX));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home).join(".config").join(LEGACY_PREFIX));
        }
    }
    // No home directory to look in, which is also the case in which the
    // previous version had nowhere to write.
    None
}

/// Moves the contents of the old data directory into the new one.
///
/// Entry by entry rather than a single directory rename, because the new
/// directory usually exists by the time this runs: the Unix front end creates
/// it while installing the host, and a log line written during start-up creates
/// it anywhere.
fn move_data_dir(old: &Path, new: &Path) {
    if old == new || !old.is_dir() {
        return;
    }
    // The settings file is the one entry that is always there once the widget
    // has run, so its presence under the new name means this has already
    // happened — or that the user has since set the new installation up by
    // hand, which is just as good a reason not to touch it.
    if new.join("config.json").exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(new) {
        log::warn(&format!("Could not create {}: {e}", new.display()));
        return;
    }
    let entries = match std::fs::read_dir(old) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn(&format!("Could not read {}: {e}", old.display()));
            return;
        }
    };

    let (mut moved, mut kept) = (0usize, 0usize);
    for entry in entries.flatten() {
        let from = entry.path();
        let to = new.join(renamed(&entry.file_name().to_string_lossy()));
        // Never overwrite. Whatever is already there was written under the
        // current name and is therefore the newer of the two.
        if to.exists() {
            kept += 1;
            continue;
        }
        // Both directories sit under the same parent on every platform, so
        // this stays within one filesystem and a rename is enough — no copy
        // fallback for a case that cannot arise.
        match std::fs::rename(&from, &to) {
            Ok(()) => moved += 1,
            Err(e) => {
                log::warn(&format!(
                    "Could not move {} to {}: {e}",
                    from.display(),
                    to.display()
                ));
                kept += 1;
            }
        }
    }

    if moved == 0 && kept == 0 {
        // An empty leftover directory: remove it and say nothing.
        let _ = std::fs::remove_dir(old);
        return;
    }
    if kept == 0 {
        // Succeeds only when the directory really is empty, which is the point.
        let _ = std::fs::remove_dir(old);
    }
    log::info(&format!(
        "Moved {moved} item(s) from {} to {}{}",
        old.display(),
        new.display(),
        match kept {
            0 => String::new(),
            n => format!(" — {n} left behind, see the warnings above"),
        }
    ));
}

/// The name a file from the old data directory takes in the new one.
///
/// Only the log is affected: it was named after the program, and rotation adds
/// a `.1` to it. Everything else — `config.json`, `token.bin`, `cache.json`,
/// the themes — was already named after what it holds.
fn renamed(file_name: &str) -> String {
    match file_name.strip_prefix(LEGACY_PREFIX) {
        Some(rest) => format!("ephemeris{rest}"),
        None => file_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log is the only file carrying the program's name, and rotation
    /// means there are two of them.
    #[test]
    fn only_the_log_is_renamed() {
        assert_eq!(renamed("tpmplaner.log"), "ephemeris.log");
        assert_eq!(renamed("tpmplaner.log.1"), "ephemeris.log.1");
        assert_eq!(renamed("config.json"), "config.json");
        assert_eq!(renamed("token.bin"), "token.bin");
        assert_eq!(renamed("midnight.theme.json"), "midnight.theme.json");
    }

    /// The destination directory is normally already there — the Unix front
    /// end creates it, and so does the first log line — so the move has to work
    /// into an existing directory, and it must not clobber what is in it.
    #[test]
    fn a_move_fills_the_gaps_and_overwrites_nothing() {
        let base = std::env::temp_dir().join(format!(
            "ephemeris-migrate-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let (old, new) = (base.join("old"), base.join("new"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("token.bin"), b"old-token").unwrap();
        std::fs::write(old.join("cache.json"), b"old-cache").unwrap();
        std::fs::write(new.join("cache.json"), b"new-cache").unwrap();

        move_data_dir(&old, &new);

        assert_eq!(std::fs::read(new.join("token.bin")).unwrap(), b"old-token");
        // Already present under the new name, so it stays as it is — and the
        // old copy stays where it was rather than being thrown away.
        assert_eq!(std::fs::read(new.join("cache.json")).unwrap(), b"new-cache");
        assert!(old.join("cache.json").exists());
        assert!(!old.join("token.bin").exists());

        let _ = std::fs::remove_dir_all(&base);
    }
}
