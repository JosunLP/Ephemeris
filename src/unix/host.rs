// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! [`Host`] for macOS and Linux.
//!
//! All five methods are implemented: the data directory follows each system's
//! convention and is created readable by its owner alone, a browser opens
//! through `open` or `xdg-open`, random bytes come from `/dev/urandom`, and
//! credentials go to the platform's keyring — see [`crate::unix::secure`],
//! which also explains what happens on a machine that has none.

use crate::unix::secure;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tpmplaner_core::host::{Host, PortableHost, is_openable_url};
use tpmplaner_core::log;

pub struct UnixHost;

impl Host for UnixHost {
    /// `$XDG_CONFIG_HOME/tpmplaner` on Linux, `~/Library/Application
    /// Support/TPMPlaner` on macOS.
    ///
    /// Delegated: the portable fallback already knows every platform's
    /// convention, and a second copy of that knowledge is a second place for
    /// it to be wrong.
    fn data_dir(&self) -> PathBuf {
        PortableHost.data_dir()
    }

    fn open_url(&self, url: &str) {
        let Some(url) = openable_target(url) else {
            log::warn(&format!("Refusing to open '{url}': not an openable URL"));
            return;
        };
        // `xdg-open` on Linux, `open` on macOS. Neither goes through a shell
        // here — the URL is one argv element — so there is nothing to quote
        // and nothing to inject.
        //
        // Both of this trait method's callers hand it a URL this program built
        // — the sign-in endpoints and the loopback callback — so the scheme
        // check above is not what stands between them and the platform. It is
        // here for the caller that is coming: `Event::html_link` arrives in the
        // calendar server's JSON, and an opener launches whatever is registered
        // for the scheme rather than merely browsing. The rule, and why it is
        // an allowlist, is in `tpmplaner_core::host::is_openable_url`.
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        // Silent: the browser's own diagnostics are not ours to print. And not
        // waited for here, because that would block the caller for as long as
        // the browser lives.
        let spawned = Command::new(opener)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            // Dropping a `Child` detaches the handle but does not reap the
            // process — Rust installs no `SIGCHLD` handler — so once the
            // opener exited it stayed in the process table as a zombie with
            // nothing left to collect it. That is nothing in a one-shot tool
            // and an accumulating leak in a window front end that lives for
            // days and spawns one of these per event click.
            //
            // A thread per opener rather than a `waitpid` loop somewhere
            // central: it costs one short-lived thread, it is the only place
            // that knows a child exists, and it keeps the caller unblocked,
            // which was the point of not waiting in the first place.
            Ok(mut child) => {
                let opener = opener.to_owned();
                let reaper = std::thread::Builder::new()
                    .name("tpmplaner-opener".into())
                    .spawn(move || {
                        if let Err(e) = child.wait() {
                            log::warn(&format!("Could not wait for {opener}: {e}"));
                        }
                    });
                if let Err(e) = reaper {
                    log::warn(&format!("Could not start the reaper thread: {e}"));
                }
            }
            Err(e) => log::warn(&format!("Could not run {opener}: {e}")),
        }
    }

    /// The platform's keyring: the Keychain on macOS, the Secret Service on
    /// Linux.
    ///
    /// Unlike DPAPI these store rather than encrypt, so what goes in the file
    /// is a reference and the secret itself never lands there. Where there is
    /// no keyring — a container, a headless session — the bytes go into the
    /// file as they are and a warning says so, because a widget that cannot
    /// sign in at all would be a poor trade. See [`crate::unix::secure`].
    ///
    /// [`prepare_data_dir`] keeps the directory owner-only either way.
    fn protect(&self, plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        secure::protect(plain, tag)
    }

    fn unprotect(&self, cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        secure::unprotect(cipher, tag)
    }

    /// Cryptographically secure random bytes from `/dev/urandom`.
    ///
    /// `None` on failure, which fails the sign-in and leaves the rest of the
    /// widget running. This used to abort. The reasoning was right — there is
    /// no value it could return that is safe, see [`Host::random_bytes`] — but
    /// the conclusion was not: opening a file has transient failure modes that
    /// have nothing to do with the randomness. `EMFILE` and `ENFILE` mean the
    /// process or the machine is briefly out of descriptors, and with
    /// `panic = "abort"` set for release, a passing spike took the whole widget
    /// down instead of one sign-in that can simply be tried again. The Windows
    /// side is not the same case: `BCryptGenRandom` needs no descriptor and has
    /// no such failure mode.
    fn random_bytes(&self, len: usize) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; len];
        match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)) {
            Ok(()) => Some(buf),
            Err(e) => {
                log::error(&format!(
                    "/dev/urandom is unreadable ({e}) — cannot generate a sign-in secret"
                ));
                None
            }
        }
    }
}

/// The string to hand the opener, or `None` if it must not be opened at all.
///
/// Exists so the trimming and the checking cannot come apart. Checking one set
/// of bytes and passing another is how a check stops meaning anything, even
/// when — as here — the difference is only whitespace: returning the value that
/// passed makes it the only one the caller has.
fn openable_target(url: &str) -> Option<&str> {
    let url = url.trim();
    is_openable_url(url).then_some(url)
}

/// Creates the data directory readable by its owner only.
///
/// Until `protect` really encrypts, the directory permission is the only thing
/// standing between a refresh token and every other account on the machine.
/// `create_dir_all` elsewhere in the core would produce 0755 minus the umask,
/// so this has to happen first — it is a no-op once the directory exists,
/// including its mode, which is deliberate: a directory the user has
/// relaxed on purpose is not ours to tighten behind their back.
pub fn prepare_data_dir() {
    use std::os::unix::fs::DirBuilderExt;

    let dir = PortableHost.data_dir();
    if dir.exists() {
        return;
    }
    // The parents first, at whatever the umask says. `DirBuilder` carries one
    // mode and applies it to every level `recursive` creates, so a single call
    // would make `~/.config` — or, on a Mac where it is somehow missing,
    // `~/Library/Application Support` — owner-only as well. Those belong to the
    // platform and to every other program that keeps something there; only the
    // leaf is ours to lock down.
    if let Some(parent) = dir.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        log::warn(&format!("Could not create {}: {e}", parent.display()));
    }
    // Still `recursive`, which is what makes it a no-op if another copy won the
    // race between the check above and this line.
    let created = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir);
    if let Err(e) = created {
        log::warn(&format!("Could not create {}: {e}", dir.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule itself is tested where it lives, in
    /// `tpmplaner_core::host::is_openable_url`. What is this front end's own is
    /// that the opener is handed exactly the string that passed the check.
    #[test]
    fn what_is_checked_is_what_is_opened() {
        assert_eq!(
            openable_target("  https://example.com  "),
            Some("https://example.com")
        );
        // Padding does not get an option or a launchable scheme past the check
        // either.
        assert_eq!(openable_target("  --version  "), None);
        assert_eq!(openable_target("  file:///bin/sh  "), None);
        assert_eq!(openable_target(""), None);
    }
}
