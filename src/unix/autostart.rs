// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
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

use ephemeris_core::log;
use std::path::PathBuf;

/// Reverse-DNS label for the launch agent, and the file name it lives under.
#[cfg(target_os = "macos")]
const AGENT_LABEL: &str = "io.github.josunlp.ephemeris";

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
        Some(base.join("autostart/ephemeris.desktop"))
    }
}

/// The same entry, as the widget wrote it under its former name.
///
/// Built from the old name rather than from [`entry_path`] so the two cannot
/// drift apart: this one must keep describing what an installation predating
/// the rename actually has on disk.
fn legacy_entry_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(home()?.join("Library/LaunchAgents/io.github.josunlp.tpmplaner.plist"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| Some(home()?.join(".config")))?;
        Some(base.join("autostart/tpmplaner.desktop"))
    }
}

/// Moves an autostart entry written under the former name to the current one.
///
/// Rewritten rather than renamed, because the file's *contents* are stale too:
/// it names the old executable, at the old install path, under the old
/// `Name=`. Writing a fresh entry from [`entry_contents`] gets all three right
/// at once, and it points at this very binary — the one that just started, and
/// therefore the one the user wants at the next login.
///
/// Called once at start-up from [`crate::migrate`]; see there for why none of
/// this is allowed to fail loudly.
pub fn migrate_entry() {
    let Some(old) = legacy_entry_path() else {
        return;
    };
    if !old.exists() {
        return;
    }
    // `launchctl unload` parses the plist at the path it is given, so this has
    // to happen while the file is still there — the same ordering [`set`]
    // observes when switching autostart off.
    unregister(&old);
    if let Err(e) = std::fs::remove_file(&old) {
        // Left in place, it would start the old binary at the next login
        // alongside this one. Worth a warning, not worth refusing to start.
        log::warn(&format!("Could not remove {}: {e}", old.display()));
    }
    set(true);
    log::info("Moved the autostart entry to the new program name");
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
        // Before the file goes: `launchctl unload` parses the plist at the
        // path it is given, so unregistering after the deletion would fail on
        // a path that is no longer there — silently, because launchctl's own
        // output is discarded.
        if path.exists() {
            unregister(&path);
        }
        // Missing is the desired state, so "not found" is a success.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
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
         Name=Ephemeris\n\
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
    // `%` introduces a field code, and an unrecognised one is dropped rather
    // than reported: `/opt/My%20Apps/ephemeris` is launched as
    // `/opt/My0Apps/ephemeris`, which does not exist, and the session says
    // nothing. A literal percent is written `%%`, quoted or not.
    let path = path.replace('%', "%%");
    const RESERVED: [char; 20] = [
        ' ', '\t', '\n', '\r', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#',
        '(', ')', '`',
    ];
    if !path.contains(RESERVED) {
        return path;
    }
    // Inside quotes `"`, `` ` ``, `$` and `\` have to be escaped with a
    // backslash — and the general string-value rule, which unescapes `\\` to
    // `\`, has already run by then. So each of them takes two backslashes, and
    // a literal backslash takes four.
    let mut escaped = String::with_capacity(path.len() + 2);
    for ch in path.chars() {
        match ch {
            '\\' => escaped.push_str(r"\\\\"),
            '"' => escaped.push_str("\\\\\""),
            '$' => escaped.push_str("\\\\$"),
            '`' => escaped.push_str("\\\\`"),
            // A key's value is one line. Written literally, these three end the
            // `Exec=` line part-way through the path and leave the remainder as
            // a stray line the parser rejects — silently, at every login. The
            // general string-value rule spells them, so they go in spelled.
            '\n' => escaped.push_str(r"\n"),
            '\t' => escaped.push_str(r"\t"),
            '\r' => escaped.push_str(r"\r"),
            _ => escaped.push(ch),
        }
    }
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

/// The two platforms write entirely different files, so they get entirely
/// different tests. `cfg!` at run time would not do: the branch that is false
/// is still *compiled*, and it names functions that do not exist on the other
/// system — which is a compile error there rather than a skipped assertion.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// `launchd` reads this. A missing header or a missing `RunAtLoad` is an
    /// entry that does nothing at login and says nothing about it.
    #[test]
    fn the_launch_agent_is_in_the_format_launchd_expects() {
        let text = entry_contents(std::path::Path::new("/opt/ephemeris/ephemeris"));
        assert!(text.starts_with("<?xml"), "{text}");
        assert!(
            text.contains("<string>/opt/ephemeris/ephemeris</string>"),
            "{text}"
        );
        assert!(text.contains("RunAtLoad"), "{text}");
        // A widget quit from its own menu must stay quit until the next login,
        // not be restarted a second later.
        assert!(!text.contains("KeepAlive"), "{text}");
    }

    /// A home directory with a space in it is ordinary, and one with an
    /// ampersand in it is legal — the second would break the XML.
    #[test]
    fn a_path_is_escaped_for_xml_and_nothing_else() {
        let text = entry_contents(std::path::Path::new("/home/a b/My Apps/ephemeris"));
        assert!(
            text.contains("<string>/home/a b/My Apps/ephemeris</string>"),
            "{text}"
        );
        assert_eq!(xml_escaped("a & b < c"), "a &amp; b &lt; c");
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    /// The session reads this at login. Without the two required keys it is
    /// skipped in silence.
    #[test]
    fn the_desktop_entry_is_in_the_format_the_session_expects() {
        let text = entry_contents(std::path::Path::new("/opt/ephemeris/ephemeris"));
        assert!(text.starts_with("[Desktop Entry]"), "{text}");
        assert!(text.contains("Type=Application"), "{text}");
        assert!(text.contains("Exec=/opt/ephemeris/ephemeris\n"), "{text}");
    }

    /// `Exec` is a command line, not a path: the specification gives `"` and
    /// `\` a meaning inside it, and a home directory containing either would
    /// otherwise produce an entry that fails at login with nothing said.
    #[test]
    fn a_path_that_needs_quoting_gets_it() {
        let text = entry_contents(std::path::Path::new("/home/a b/My Apps/ephemeris"));
        assert!(
            text.contains("Exec=\"/home/a b/My Apps/ephemeris\"\n"),
            "{text}"
        );
        // Two backslashes for a quote and four for a backslash: the general
        // string-value rule unescapes `\\` to `\` before the quoting rule is
        // applied, so each escape has to survive being read twice.
        assert_eq!(exec_quoted(r#"/home/a"b\c"#), r#""/home/a\\"b\\\\c""#);
        // A plain path is left exactly as it is.
        assert_eq!(exec_quoted("/usr/bin/ephemeris"), "/usr/bin/ephemeris");
        // `%` is a field code, so a literal one is doubled — quoting or not.
        assert_eq!(exec_quoted("/opt/My%20Apps/x"), "/opt/My%%20Apps/x");
        assert_eq!(
            exec_quoted("/opt/My %Apps/x"),
            "\"/opt/My %%Apps/x\"",
            "quoting must not lose the doubling"
        );
        // A key's value is one line, so the characters that would end it early
        // are spelled rather than written — an `Exec=` cut in half is an entry
        // the session skips without a word.
        assert_eq!(exec_quoted("/home/a\nb/x"), r#""/home/a\nb/x""#);
        assert_eq!(exec_quoted("/home/a\tb/x"), r#""/home/a\tb/x""#);
        assert!(!exec_quoted("/home/a\r\nb/x").contains(['\n', '\r']));
    }
}
