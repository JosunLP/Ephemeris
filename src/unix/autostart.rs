// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Starting the widget with the session.
//!
//! Windows writes a value under the Run key. Neither of these platforms has
//! anything like it, and the two answers are not the same shape:
//!
//! * **Linux**: a `.desktop` file in `$XDG_CONFIG_HOME/autostart`. The
//!   specification is old, small, and honoured by GNOME, KDE, XFCE, LXQt and
//!   the session managers of the tiling compositors alike.
//! * **macOS**: a launch agent property list in `~/Library/LaunchAgents`,
//!   registered with `launchctl`. `SMAppService` is the modern route and needs
//!   the program to be a signed bundle registering itself; a launch agent
//!   works for a bare binary as well, which is what the tarball ships and what
//!   `cargo run` produces.
//!
//! Both are plain files in the user's own home, which is the property that
//! matters: nothing is written outside the profile, and removing the file is
//! all it takes to undo — the same promise the Windows installer makes.
//!
//! The file names the *current* executable. A widget moved to a different
//! directory therefore has to be re-enabled, which is honest: the alternative
//! is an entry that points at nothing and fails silently at every login.

use std::path::PathBuf;
use tpmplaner_core::log;

/// Reverse-DNS label for the launch agent, and the file name it lives under.
#[cfg(target_os = "macos")]
const AGENT_LABEL: &str = "io.github.josunlp.tpmplaner";

/// Can this platform start the widget with the session at all?
///
/// False only where the entry could not be written — a read-only home, no home
/// at all. The menu then leaves the item out rather than offering something
/// that quietly does nothing.
pub fn supported() -> bool {
    entry_path().is_some()
}

/// Where the autostart entry lives.
///
/// `None` when the home directory cannot be determined, which is the one case
/// with nowhere to put it.
fn entry_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(home()?.join(format!("Library/LaunchAgents/{AGENT_LABEL}.plist")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        // `$XDG_CONFIG_HOME` when set and absolute, `~/.config` otherwise —
        // the specification's own rule, and the same one the data directory
        // follows.
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| Some(home()?.join(".config")))?;
        Some(base.join("autostart/tpmplaner.desktop"))
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

pub fn enabled() -> bool {
    entry_path().is_some_and(|p| p.exists())
}

/// Writes or removes the entry.
///
/// Failures are logged rather than reported: this is a menu toggle, and the
/// next time the menu opens it will show the state that actually took effect,
/// because [`enabled`] asks the filesystem rather than remembering.
pub fn set(on: bool) {
    let Some(path) = entry_path() else {
        log::warn("No home directory — cannot change the autostart entry");
        return;
    };

    if !on {
        // Missing is the desired state, so "not found" is a success.
        match std::fs::remove_file(&path) {
            Ok(()) => unregister(&path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn(&format!("Could not remove {}: {e}", path.display())),
        }
        return;
    }

    let Some(exe) = current_exe() else {
        return;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn(&format!("Could not create {}: {e}", dir.display()));
        return;
    }
    match std::fs::write(&path, entry_contents(&exe)) {
        Ok(()) => register(&path),
        Err(e) => log::warn(&format!("Could not write {}: {e}", path.display())),
    }
}

/// The running binary's path, resolved through symlinks.
///
/// `current_exe` on Linux reads `/proc/self/exe`, which is already resolved;
/// on macOS it returns the path the process was launched from, which may be a
/// symlink from a package manager's `bin` directory. Canonicalising both keeps
/// the entry pointing at the real file rather than at a link a future upgrade
/// may repoint or remove.
fn current_exe() -> Option<PathBuf> {
    match std::env::current_exe() {
        Ok(exe) => Some(std::fs::canonicalize(&exe).unwrap_or(exe)),
        Err(e) => {
            log::warn(&format!("Cannot find this program's own path: {e}"));
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn entry_contents(exe: &std::path::Path) -> String {
    // `RunAtLoad` starts it with the session; `KeepAlive` is deliberately
    // absent, because a widget the user quit from its own menu must stay quit
    // until the next login rather than being restarted a second later.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        xml_escaped(&exe.to_string_lossy())
    )
}

#[cfg(not(target_os = "macos"))]
fn entry_contents(exe: &std::path::Path) -> String {
    // `Exec` is a command line, not a path: the specification gives `"` and
    // `\` a meaning inside it, and a home directory containing either would
    // otherwise produce an entry that fails at login with nothing said.
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TPMPlaner\n\
         Comment=Today's calendar events and due tasks on your desktop\n\
         Exec={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_quoted(&exe.to_string_lossy())
    )
}

/// A path as a single `Exec` argument, quoted the way the desktop entry
/// specification asks for.
#[cfg(not(target_os = "macos"))]
fn exec_quoted(path: &str) -> String {
    if !path.contains([' ', '\t', '"', '\\', '\'', '$', '`']) {
        return path.to_owned();
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Text as XML character data.
#[cfg(target_os = "macos")]
fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Tells `launchd` about the new agent, so it also takes effect without
/// logging out. On Linux the session reads the directory at login and there is
/// nothing to register.
#[cfg(target_os = "macos")]
fn register(path: &std::path::Path) {
    launchctl(&["load", "-w", &path.to_string_lossy()]);
}

#[cfg(target_os = "macos")]
fn unregister(path: &std::path::Path) {
    launchctl(&["unload", "-w", &path.to_string_lossy()]);
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) {
    use std::process::{Command, Stdio};

    // Failure is not fatal: the plist is on disk either way and `launchd`
    // reads the directory at the next login. Nothing here goes through a
    // shell, so a path with spaces needs no quoting.
    let run = Command::new("launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Err(e) = run {
        log::warn(&format!("Could not run launchctl {}: {e}", args[0]));
    }
}

#[cfg(not(target_os = "macos"))]
fn register(_path: &std::path::Path) {}

#[cfg(not(target_os = "macos"))]
fn unregister(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The entry has to be readable by the thing that reads it, and both
    /// formats have a required header.
    #[test]
    fn the_entry_is_in_the_format_its_reader_expects() {
        let text = entry_contents(std::path::Path::new("/opt/tpmplaner/tpmplaner"));
        assert!(text.contains("/opt/tpmplaner/tpmplaner"), "{text}");
        if cfg!(target_os = "macos") {
            assert!(text.starts_with("<?xml"), "{text}");
            assert!(text.contains("RunAtLoad"), "{text}");
            // A widget quit from its own menu must stay quit.
            assert!(!text.contains("KeepAlive"), "{text}");
        } else {
            assert!(text.starts_with("[Desktop Entry]"), "{text}");
            assert!(text.contains("Type=Application"), "{text}");
        }
    }

    /// A home directory with a space in it is ordinary on both systems, and an
    /// entry that breaks on one is an entry that fails at login with nothing
    /// said.
    #[test]
    fn a_path_that_needs_quoting_gets_it() {
        let text = entry_contents(std::path::Path::new("/home/a b/My Apps/tpmplaner"));
        if cfg!(target_os = "macos") {
            // XML needs no quoting for a space, only for its own three
            // characters.
            assert!(text.contains("<string>/home/a b/My Apps/tpmplaner</string>"));
            #[cfg(target_os = "macos")]
            assert_eq!(xml_escaped("a & b < c"), "a &amp; b &lt; c");
        } else {
            assert!(
                text.contains("Exec=\"/home/a b/My Apps/tpmplaner\"\n"),
                "{text}"
            );
            // And a quote or a backslash in the path is escaped rather than
            // ending the argument early.
            assert_eq!(exec_quoted(r#"/home/a"b\c"#), r#""/home/a\"b\\c""#);
            // A plain path is left exactly as it is.
            assert_eq!(exec_quoted("/usr/bin/tpmplaner"), "/usr/bin/tpmplaner");
        }
    }
}
