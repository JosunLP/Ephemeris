// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! [`Host`] for macOS and Linux.
//!
//! Three of the five methods are genuinely implemented here. The other two —
//! `protect` and `unprotect` — are honest placeholders, and the comment on
//! them says so rather than dressing it up: the Keychain and the Secret
//! Service both need work that has not been done, and pretending to encrypt is
//! worse than not encrypting.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tpmplaner_core::host::{Host, PortableHost};
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
        if !is_openable(url) {
            log::warn(&format!("Refusing to open '{url}': not a URL"));
            return;
        }
        // `xdg-open` on Linux, `open` on macOS. Neither goes through a shell
        // here — the URL is one argv element — so there is nothing to quote
        // and nothing to inject. Some of these URLs come from a calendar
        // server rather than from us, which is why [`is_openable`] runs first.
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        // Detached and silent: the browser's own diagnostics are not ours to
        // print, and waiting for it would block the caller for as long as the
        // browser lives.
        let spawned = Command::new(opener)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Err(e) = spawned {
            log::warn(&format!("Could not run {opener}: {e}"));
        }
    }

    /// **Not encrypted yet.** The bytes are returned unchanged, which means a
    /// refresh token is stored in a file readable by the account that owns it.
    ///
    /// The real implementations are the Keychain (`SecItemAdd` /
    /// `SecItemCopyMatching`) on macOS and the Secret Service over D-Bus on
    /// Linux, with a documented fallback for headless setups that have no
    /// keyring daemon. Until then [`prepare_data_dir`] at least keeps the
    /// directory owner-only, and this says plainly what it does rather than
    /// looking like protection that is not there.
    fn protect(&self, plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        PortableHost.protect(plain, tag)
    }

    fn unprotect(&self, cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        PortableHost.unprotect(cipher, tag)
    }

    /// Cryptographically secure random bytes from `/dev/urandom`.
    ///
    /// This is the one method whose portable fallback is actively unsafe: it
    /// returns zeros, and a PKCE verifier of zeros is no verifier at all.
    ///
    /// A failure returns nothing rather than a short buffer of zeros, so the
    /// sign-in fails at the authorisation server — visibly, and closed. A
    /// verifier that is predictable would succeed, which is the outcome worth
    /// avoiding.
    fn random_bytes(&self, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)) {
            Ok(()) => buf,
            Err(e) => {
                log::error(&format!(
                    "/dev/urandom is unreadable ({e}) — refusing to produce predictable bytes"
                ));
                Vec::new()
            }
        }
    }
}

/// Is this something an opener should be handed at all?
///
/// Two rules. It must carry a scheme, because a bare path or a search term is
/// not what any caller here means. And it must not begin with `-`, or the
/// opener parses it as one of its own options — the one way a URL can act as
/// something other than an argument when no shell is involved.
fn is_openable(url: &str) -> bool {
    let url = url.trim();
    if url.starts_with('-') {
        return false;
    }
    match url.split_once(':') {
        Some((scheme, rest)) => {
            !rest.is_empty()
                && !scheme.is_empty()
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
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

    #[test]
    fn only_real_urls_are_handed_to_the_opener() {
        for good in [
            "https://calendar.google.com/event?eid=1",
            "http://127.0.0.1:8731/callback?code=x",
            "msteams://l/meetup-join/19%3ameeting",
        ] {
            assert!(is_openable(good), "{good}");
        }
        for bad in [
            "",
            "not a url",
            "/etc/passwd",
            // Would be read as an option by the opener rather than as a target.
            "--version",
            "-x https://example.com",
            // A scheme with nothing after it opens nothing.
            "https:",
            "1https://example.com",
        ] {
            assert!(!is_openable(bad), "{bad}");
        }
    }
}
