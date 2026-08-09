// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Stored credentials on macOS and Linux.
//!
//! A refresh token is standing access to a calendar account and has no business
//! sitting on disk in plain text. Windows uses DPAPI, which encrypts a blob
//! against the user account and hands it back to be written to a file. macOS
//! and Linux have no equivalent of that, and the mapping matters:
//!
//! **The Keychain and the Secret Service are stores, not ciphers.** They take a
//! secret under a name and give it back to the account that owns it; there is
//! no "encrypt these bytes" call to substitute for `CryptProtectData`. So the
//! secret goes into the keyring under the caller's tag, and what
//! [`protect`] returns — the thing that ends up in the file — is a *reference*
//! saying where it went. [`unprotect`] sees the reference and fetches the
//! secret back.
//!
//! That is better than DPAPI's shape rather than worse. The token never
//! reaches the file at all, so a stolen `token.bin` is worth nothing without
//! the keyring, and both systems can gate access on the user unlocking it.
//!
//! **The fallback says what it is.** A headless machine, a container, a session
//! with no keyring daemon: all real, and on any of them the keyring call fails.
//! Then the bytes go into the file as they are — which is exactly what this
//! front end did before — and a warning says so once, rather than the sign-in
//! silently failing every start. Pretending to encrypt would be worse than not
//! encrypting; so would refusing to run.
//!
//! A file written by the fallback is readable after a keyring appears, and one
//! written to the keyring is not silently replaced by a plain copy: the
//! reference and the raw secret are told apart by a prefix that no token
//! begins with, so the two forms can coexist across an upgrade in either
//! direction.

use std::sync::atomic::{AtomicBool, Ordering};
use tpmplaner_core::log;

/// Marks a stored blob as a pointer into the keyring rather than the secret.
///
/// The tag follows it, so `unprotect` can find the item even if the caller's
/// tag were ever to change shape. A base64url refresh token cannot begin with
/// this — it contains no colon and no capital-and-hyphen run — and neither can
/// a DPAPI blob, which is why the two forms can be distinguished at rest.
const REFERENCE_PREFIX: &[u8] = b"TPMPlaner-keyring-v1:";

/// What the keyring lists the widget's items under.
const SERVICE: &str = "TPMPlaner";

/// So the "not encrypting" warning is written once rather than on every sync.
static FALLBACK_REPORTED: AtomicBool = AtomicBool::new(false);

/// Stores the secret in the platform's keyring and returns the reference to
/// write to disk.
///
/// Falls back to returning the secret unchanged when there is no keyring to
/// store it in, having said so.
pub fn protect(plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    match keyring_name(tag) {
        Some(name) => match platform::store(&name, plain) {
            Ok(()) => Some(reference_for(tag)),
            Err(e) => Some(fall_back(plain, &e)),
        },
        // A tag that cannot name a keyring item is this program's own bug
        // rather than the machine's, but the user still needs their calendar.
        None => Some(fall_back(
            plain,
            "the credential tag is not usable as a keyring item name",
        )),
    }
}

/// The secret behind a stored blob: fetched from the keyring for a reference,
/// returned as-is for anything else.
pub fn unprotect(cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    let Some(stored_tag) = reference_tag(cipher) else {
        // Written by the fallback, or by a build from before the keyring
        // existed. Still the user's token; still theirs to use.
        return Some(cipher.to_vec());
    };
    // The tag in the file wins over the one passed in: it is what the secret
    // was actually filed under.
    let name = keyring_name(stored_tag).or_else(|| keyring_name(tag))?;
    match platform::lookup(&name) {
        Ok(Some(secret)) => Some(secret),
        // The reference is there and the item is not: the user cleared their
        // keyring, or this is a different machine. A failed sign-in that can
        // be redone beats a confusing error.
        Ok(None) => {
            log::warn(&format!(
                "No keyring entry for '{name}' — the account has to be connected again"
            ));
            None
        }
        Err(e) => {
            log::warn(&format!("Could not read '{name}' from the keyring: {e}"));
            None
        }
    }
}

/// Reports once, then hands the caller its own bytes back.
fn fall_back(plain: &[u8], reason: &str) -> Vec<u8> {
    if !FALLBACK_REPORTED.swap(true, Ordering::Relaxed) {
        log::warn(&format!(
            "No keyring available ({reason}). The access token is being stored \
             unencrypted, readable by this account only. See \
             docs/development/porting.md."
        ));
    }
    plain.to_vec()
}

fn reference_for(tag: &[u8]) -> Vec<u8> {
    let mut out = REFERENCE_PREFIX.to_vec();
    out.extend_from_slice(tag);
    out
}

/// The tag inside a reference, or `None` if this blob is not one.
fn reference_tag(blob: &[u8]) -> Option<&[u8]> {
    blob.strip_prefix(REFERENCE_PREFIX)
}

/// The keyring item name for a caller's tag.
///
/// `None` for a tag that is not printable ASCII: both back ends name items with
/// strings, the Linux one passes the name to another process, and a tag
/// carrying a newline or a control character has no business in either. Every
/// tag the core actually uses — `google-token`, and the per-account CalDAV and
/// OAuth tags — is plain text already, so this rejects nothing real.
fn keyring_name(tag: &[u8]) -> Option<String> {
    let tag = std::str::from_utf8(tag).ok()?;
    if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        return None;
    }
    Some(tag.to_owned())
}

// --- macOS -------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod platform {
    //! The Keychain, through `SecItemAdd` and `SecItemCopyMatching`.
    //!
    //! A generic password item, keyed by service and account, which is the
    //! shape every command-line tool on the system uses and what
    //! `security find-generic-password` will show the user.

    use crate::unix::cf::{self, CFDictionaryRef, CFStringRef, CFTypeRef};

    type OSStatus = i32;
    const ERR_SEC_SUCCESS: OSStatus = 0;
    const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;
    const ERR_SEC_DUPLICATE_ITEM: OSStatus = -25299;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
        fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
        fn SecItemUpdate(query: CFDictionaryRef, attributesToUpdate: CFDictionaryRef) -> OSStatus;

        static kSecClass: CFStringRef;
        static kSecClassGenericPassword: CFStringRef;
        static kSecAttrService: CFStringRef;
        static kSecAttrAccount: CFStringRef;
        static kSecValueData: CFStringRef;
        static kSecReturnData: CFStringRef;
        static kSecMatchLimit: CFStringRef;
        static kSecMatchLimitOne: CFStringRef;
        static kSecAttrAccessible: CFStringRef;
        static kSecAttrAccessibleAfterFirstUnlock: CFStringRef;
    }

    /// The service and account pair that identifies this widget's item for
    /// `name`.
    fn identity(name: &str) -> Option<(cf::Owned, cf::Owned)> {
        Some((cf::string(super::SERVICE)?, cf::string(name)?))
    }

    pub fn store(name: &str, secret: &[u8]) -> Result<(), String> {
        let (service, account) = identity(name).ok_or("could not build the item identity")?;
        let value = cf::data(secret).ok_or("could not wrap the secret")?;

        // `AfterFirstUnlock` rather than `WhenUnlocked`: the widget starts at
        // login and syncs on a timer, so it has to reach the token without the
        // user being at the keyboard. It still requires the machine to have
        // been unlocked once since it booted.
        let attributes = unsafe {
            cf::dictionary(
                &[
                    kSecClass,
                    kSecAttrService,
                    kSecAttrAccount,
                    kSecValueData,
                    kSecAttrAccessible,
                ],
                &[
                    kSecClassGenericPassword,
                    service.as_raw(),
                    account.as_raw(),
                    value.as_raw(),
                    kSecAttrAccessibleAfterFirstUnlock,
                ],
            )
        }
        .ok_or("could not build the keychain attributes")?;

        let status = unsafe { SecItemAdd(attributes.as_raw(), std::ptr::null_mut()) };
        match status {
            ERR_SEC_SUCCESS => Ok(()),
            // Already there from a previous sign-in: replace its value rather
            // than leaving the old token in place.
            ERR_SEC_DUPLICATE_ITEM => update(&service, &account, &value),
            other => Err(format!("SecItemAdd failed with status {other}")),
        }
    }

    fn update(service: &cf::Owned, account: &cf::Owned, value: &cf::Owned) -> Result<(), String> {
        let query = unsafe {
            cf::dictionary(
                &[kSecClass, kSecAttrService, kSecAttrAccount],
                &[kSecClassGenericPassword, service.as_raw(), account.as_raw()],
            )
        }
        .ok_or("could not build the keychain query")?;
        let changes = unsafe { cf::dictionary(&[kSecValueData], &[value.as_raw()]) }
            .ok_or("could not build the keychain update")?;

        match unsafe { SecItemUpdate(query.as_raw(), changes.as_raw()) } {
            ERR_SEC_SUCCESS => Ok(()),
            other => Err(format!("SecItemUpdate failed with status {other}")),
        }
    }

    pub fn lookup(name: &str) -> Result<Option<Vec<u8>>, String> {
        let (service, account) = identity(name).ok_or("could not build the item identity")?;
        let query = unsafe {
            cf::dictionary(
                &[
                    kSecClass,
                    kSecAttrService,
                    kSecAttrAccount,
                    kSecReturnData,
                    kSecMatchLimit,
                ],
                &[
                    kSecClassGenericPassword,
                    service.as_raw(),
                    account.as_raw(),
                    cf::kCFBooleanTrue,
                    kSecMatchLimitOne,
                ],
            )
        }
        .ok_or("could not build the keychain query")?;

        let mut result: CFTypeRef = std::ptr::null();
        let status = unsafe { SecItemCopyMatching(query.as_raw(), &mut result) };
        match status {
            ERR_SEC_SUCCESS => {
                // `CopyMatching`: the result is ours to release.
                let owned = cf::Owned::new(result).ok_or("the keychain returned nothing")?;
                unsafe { cf::data_bytes(owned.as_raw()) }
                    .map(Some)
                    .ok_or_else(|| "the keychain item was not data".to_string())
            }
            ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            other => Err(format!("SecItemCopyMatching failed with status {other}")),
        }
    }
}

// --- Linux and the rest of Unix ----------------------------------------------

#[cfg(not(target_os = "macos"))]
mod platform {
    //! The Secret Service, through `secret-tool`.
    //!
    //! **Why a command and not D-Bus.** Speaking the Secret Service protocol
    //! directly means a D-Bus client, and every one available is either a C
    //! library to link against or a pure-Rust stack of a dozen crates. This
    //! project states a ~1.7 MB binary and an under-4 MB resident footprint as
    //! properties, and `secret-tool` is the reference client for exactly this
    //! job, present wherever the Secret Service itself is, and works against
    //! GNOME Keyring and KWallet alike. When the widget's own window arrives on
    //! Linux it will already need a D-Bus connection for the appearance and
    //! global-shortcut portals; that is the moment to speak the protocol
    //! directly, and this becomes a fallback rather than the route.
    //!
    //! **The secret never touches a command line.** It goes in on standard
    //! input and comes back on standard output. An argument would be visible in
    //! `ps` to every account on the machine, which would defeat the entire
    //! exercise. The attribute *name* is an argument, which is why the caller
    //! restricts it to printable text.

    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    const TOOL: &str = "secret-tool";
    /// The attribute the widget files its items under. `secret-tool` matches on
    /// attribute pairs rather than on a service and account, so this is the
    /// equivalent of the Keychain's service field.
    const ATTRIBUTE: &str = "tpmplaner-credential";

    pub fn store(name: &str, secret: &[u8]) -> Result<(), String> {
        let mut child = Command::new(TOOL)
            .arg("store")
            .arg("--label")
            .arg(format!("{} ({name})", super::SERVICE))
            .arg(ATTRIBUTE)
            .arg(name)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run {TOOL}: {e}"))?;

        // Taken rather than borrowed, so the pipe is closed before the wait.
        // `secret-tool store` reads to end of file, and a handle still open in
        // this process is a deadlock: it waits for input that will not come,
        // and this waits for it to exit.
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| format!("{TOOL} took no standard input"))?;
            stdin
                .write_all(secret)
                .map_err(|e| format!("could not pass the secret to {TOOL}: {e}"))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("could not wait for {TOOL}: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        Err(tool_error("store", &output.stderr, output.status))
    }

    pub fn lookup(name: &str) -> Result<Option<Vec<u8>>, String> {
        let output = Command::new(TOOL)
            .arg("lookup")
            .arg(ATTRIBUTE)
            .arg(name)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run {TOOL}: {e}"))?;

        if !output.status.success() {
            // `secret-tool lookup` exits non-zero for "no such item", which is
            // an answer rather than a failure — and it says nothing on standard
            // error when that is all that happened.
            if output.stderr.is_empty() {
                return Ok(None);
            }
            return Err(tool_error("lookup", &output.stderr, output.status));
        }
        if output.stdout.is_empty() {
            return Ok(None);
        }
        // The tool prints the secret with no trailing newline, but a version
        // that added one would corrupt every token it returned.
        let mut secret = output.stdout;
        if secret.last() == Some(&b'\n') {
            secret.pop();
        }
        Ok(Some(secret))
    }

    fn tool_error(verb: &str, stderr: &[u8], status: std::process::ExitStatus) -> String {
        let mut message = String::new();
        let _ = (&stderr[..]).read_to_string(&mut message);
        let message = message.trim();
        if message.is_empty() {
            format!("{TOOL} {verb} failed ({status})")
        } else {
            format!("{TOOL} {verb} failed ({status}): {message}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_told_apart_from_a_secret() {
        let reference = reference_for(b"google-token");
        assert_eq!(reference_tag(&reference), Some(&b"google-token"[..]));

        // What a token actually looks like, and what the fallback would have
        // written: neither may be mistaken for a reference, or the widget
        // would go looking in the keyring for something sitting in front of it.
        for secret in [
            &b"1//09abcDEF-ghiJKL_mnoPQR"[..],
            &b""[..],
            &b"TPMPlaner"[..],
            // A near miss: the right words, the wrong version.
            &b"TPMPlaner-keyring-v2:google-token"[..],
        ] {
            assert!(reference_tag(secret).is_none(), "{secret:?}");
        }
    }

    #[test]
    fn a_reference_survives_the_round_trip_for_every_tag_the_core_uses() {
        // The three shapes in the core: the Google token, a CalDAV account and
        // an OAuth account.
        for tag in ["google-token", "caldav:work@example.com", "oauth-graph-0"] {
            let reference = reference_for(tag.as_bytes());
            assert_eq!(reference_tag(&reference), Some(tag.as_bytes()));
            assert_eq!(keyring_name(tag.as_bytes()).as_deref(), Some(tag));
        }
    }

    #[test]
    fn a_tag_that_could_not_name_a_keyring_item_is_refused() {
        // Not fatal — the caller falls back — but it must not reach a command
        // line or a keychain attribute.
        assert!(keyring_name(b"").is_none());
        assert!(keyring_name(b"has\nnewline").is_none());
        assert!(keyring_name(b"has\0nul").is_none());
        assert!(keyring_name(&[0xFF, 0xFE]).is_none());
        // Printable text with a space is fine.
        assert_eq!(
            keyring_name(b"work calendar").as_deref(),
            Some("work calendar")
        );
    }

    #[test]
    fn a_blob_written_before_the_keyring_existed_still_reads_back() {
        // The upgrade path: a plain token already in `token.bin` is returned
        // unchanged rather than sent to a keyring that never stored it.
        let plain = b"1//09abcDEF".to_vec();
        assert_eq!(unprotect(&plain, b"google-token"), Some(plain));
    }
}
