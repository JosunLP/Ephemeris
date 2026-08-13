// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The desktop's appearance settings, read through the XDG settings portal.
//!
//! `org.freedesktop.appearance` is the one interface that now covers both
//! GNOME and KDE, and it carries the two values that matter most:
//! `color-scheme` (0 no preference, 1 prefer dark, 2 prefer light) and
//! `accent-color` as three floats. The portal answers whether or not the
//! program is in a Flatpak, which is what makes it worth using over either
//! desktop's own settings.
//!
//! **Through `gdbus` rather than a D-Bus library.** The same reasoning as
//! `secret-tool` in [`crate::unix::secure`]: speaking D-Bus means a C library
//! to link or a dozen crates to carry, against a binary this project states as
//! ~1.7 MB. `gdbus` ships with GLib, which is already loaded here for Pango.
//! Two short-lived processes at start-up and one per appearance change is not
//! a cost anybody can measure.
//!
//! **What the portal does not carry** is the two accessibility switches. There
//! is no portal key for "reduce motion" or "high contrast", so those come from
//! the GNOME and KDE settings directly where they can be read, and default to
//! "on" and "off" — the values that change nothing — where they cannot. That
//! is the gap `docs/development/porting.md` predicted, and it is a gap in the
//! portal rather than in this.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use tpmplaner_core::log;
use tpmplaner_core::theme::SystemVisuals;

/// Raised by the watcher thread, taken by the event loop.
static CHANGED: AtomicBool = AtomicBool::new(false);

/// Has the desktop's appearance changed since this was last asked?
pub fn appearance_changed() -> bool {
    CHANGED.swap(false, Ordering::Relaxed)
}

/// Watches the portal for appearance changes and pokes the event loop.
///
/// Polling would mean two `gdbus` processes a minute for a value that changes
/// twice a day, so the portal's own signal is what is listened to instead —
/// `gdbus monitor` prints one line per signal and this thread reads them.
///
/// The thread writes to the same pipe the sync thread uses, and raises
/// [`CHANGED`] first, so the loop can tell the two apart without a second
/// descriptor. A failure to start the monitor is logged once and then let be:
/// the widget still follows the theme it was started with, which is a great
/// deal better than not starting.
pub fn watch(wake: std::os::fd::RawFd) {
    let spawned = Command::new("gdbus")
        .args([
            "monitor",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            log::warn(&format!(
                "Could not watch the appearance portal ({e}) — light and dark will only \
                 follow the desktop at the next start"
            ));
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };

    let started = std::thread::Builder::new()
        .name("tpmplaner-appearance".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if !line.contains("org.freedesktop.appearance") {
                    continue;
                }
                CHANGED.store(true, Ordering::Relaxed);
                // Safety: `wake` is the write end of the loop's own pipe and
                // stays open for as long as the process does.
                let mut pipe =
                    unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(wake) };
                let _ = pipe.write_all(&[1]);
                // Not ours to close: the loop still needs it.
                std::mem::forget(pipe);
            }
            // The monitor stopped — the portal went away, or the session is
            // ending. Reaped so it does not linger as a zombie.
            let _ = child.wait();
        });
    if let Err(e) = started {
        log::warn(&format!("Could not start the appearance watcher: {e}"));
    }
}

pub fn read() -> SystemVisuals {
    let scheme = portal_setting("color-scheme");
    let accent = portal_setting("accent-color");
    SystemVisuals {
        // 1 is "prefer dark". No preference and "prefer light" both mean the
        // light palette, which is what every toolkit does with it.
        light: scheme.as_deref().is_none_or(|s| parse_u32(s) != Some(1)),
        accent: accent.as_deref().and_then(parse_accent),
        // No portal key for either of these, and no widely honoured setting.
        // "On" changes nothing, which is the right default for something that
        // cannot be asked.
        transparency: true,
        animations: gsetting_true("org.gnome.desktop.interface", "enable-animations")
            .is_none_or(|on| on),
        high_contrast: gsetting_true("org.gnome.desktop.a11y.interface", "high-contrast")
            .unwrap_or(false),
        // Linux has no equivalent of the Windows high-contrast palette — a
        // GTK theme is not five colours the widget can read — so the core
        // falls back to its own contrast palette. That is the documented
        // behaviour of the field, not a gap.
        contrast: None,
    }
}

/// Asks the settings portal for one key.
///
/// The reply is `gdbus`'s own rendering of a D-Bus variant, for example
/// `(<<uint32 1>>,)` or `(<<(0.2, 0.4, 0.9)>>,)`. Parsed loosely on purpose:
/// the shape has changed once already between portal versions, and the numbers
/// inside it have not.
fn portal_setting(key: &str) -> Option<String> {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.ReadOne",
            "org.freedesktop.appearance",
            key,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The first unsigned integer in a `gdbus` reply.
fn parse_u32(reply: &str) -> Option<u32> {
    let after = reply.split("uint32").nth(1)?;
    after
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// The accent colour, given as three floats from zero to one.
fn parse_accent(reply: &str) -> Option<u32> {
    let inner = reply.split('(').nth(2)?;
    let inner = inner.split(')').next()?;
    let parts: Vec<f64> = inner
        .split(',')
        .filter_map(|p| p.trim().parse::<f64>().ok())
        .collect();
    let [r, g, b] = parts[..] else { return None };
    let byte = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    Some((byte(r) << 16) | (byte(g) << 8) | byte(b))
}

/// One boolean from GNOME's settings, if `gsettings` is there to ask.
///
/// `None` when the tool, the schema or the key is missing — a KDE or XFCE
/// desktop, most often — which the caller turns into the default that changes
/// nothing. KDE has its own answers in `kdeglobals`, and reading an INI file
/// for a switch that only removes an animation is not worth the second code
/// path.
fn gsetting_true(schema: &str, key: &str) -> Option<bool> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    match text {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reply is text from another program, so the parsing has to survive
    /// the shapes it has actually had — and refuse anything else rather than
    /// inventing a colour.
    #[test]
    fn a_portal_reply_is_read_or_refused() {
        assert_eq!(parse_u32("(<<uint32 1>>,)\n"), Some(1));
        assert_eq!(parse_u32("(<uint32 2>,)"), Some(2));
        assert_eq!(parse_u32("(<<uint32 0>>,)"), Some(0));
        assert_eq!(parse_u32("()"), None);

        assert_eq!(parse_accent("(<<(0.0, 0.5, 1.0)>>,)"), Some(0x00_80FF));
        assert_eq!(parse_accent("(<<(1.0, 1.0, 1.0)>>,)"), Some(0xFF_FFFF));
        // Two components is not a colour, and neither is nothing at all.
        assert_eq!(parse_accent("(<<(0.5, 0.5)>>,)"), None);
        assert_eq!(parse_accent("nonsense"), None);
    }
}
