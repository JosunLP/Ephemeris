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
//! **The fallback says what it is, and only where it applies.** A headless
//! machine, a container, a session with no keyring daemon: all real, and on any
//! of them there is nothing to store a secret in. Then the bytes go into the
//! file as they are — which is exactly what this front end did before — and a
//! warning says so once, rather than the sign-in silently failing every start.
//! Pretending to encrypt would be worse than not encrypting; so would refusing
//! to run.
//!
//! A keyring that is *present and says no* is a different answer and gets a
//! different one back. A locked keychain, a passphrase prompt the user
//! dismissed, a daemon a login item started faster than: all temporary, and
//! writing the token to disk in plain text is not. Nothing is written at all
//! there — see [`Refusal`] — which costs one sign-in and keeps the promise
//! above.
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

/// Why a keyring call did not succeed.
///
/// The distinction carries the whole promise of this module. Treating every
/// failure as "there is no keyring here" is right for a container and quite
/// wrong for a keychain that is merely locked, a passphrase prompt the user
/// dismissed, or a `gnome-keyring-daemon` that a login item started faster
/// than. Those are true now and false a second later — and the plain copy the
/// fallback writes would still be sitting in `token.bin` long after, with the
/// warning that explains it suppressed by [`FALLBACK_REPORTED`] since first
/// sign-in.
enum Refusal {
    /// Nothing on this machine can store a secret: no keyring, no daemon, no
    /// session bus. The documented fallback applies — the bytes go into the
    /// file and a warning says so.
    NoKeyring(String),
    /// There is a keyring and it said no. Nothing is written anywhere: losing
    /// a saved token costs one sign-in, and that is the cheaper mistake.
    Declined(String),
}

/// Stores the secret in the platform's keyring and returns the reference to
/// write to disk.
///
/// Falls back to returning the secret unchanged when there is no keyring to
/// store it in, having said so. `None` when there is one and it refused, which
/// leaves the token unsaved rather than unencrypted.
pub fn protect(plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    let Some(name) = keyring_name(tag) else {
        // A tag that cannot name a keyring item is this program's own bug
        // rather than the machine's, but the user still needs their calendar.
        return Some(fall_back(
            plain,
            "the credential tag is not usable as a keyring item name",
        ));
    };
    match platform::store(&name, plain) {
        Ok(()) => Some(reference_for(tag)),
        Err(Refusal::NoKeyring(why)) => Some(fall_back(plain, &why)),
        Err(Refusal::Declined(why)) => {
            log::warn(&format!(
                "The keyring would not store '{name}' ({why}). The token has not \
                 been saved anywhere — the next refresh stores it, or the account \
                 can be connected again."
            ));
            None
        }
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
        // Either kind of failure ends the same way — the token cannot be read
        // this run — but they are worth telling apart in the log, because one
        // of them is worth trying again and the other is a machine that has no
        // keyring and a `token.bin` that came from one that did.
        Err(Refusal::Declined(why)) => {
            log::warn(&format!(
                "The keyring would not give up '{name}' ({why}) — unlocking it and \
                 restarting should be enough"
            ));
            None
        }
        Err(Refusal::NoKeyring(why)) => {
            log::warn(&format!(
                "'{name}' was stored in a keyring this machine does not have ({why}) \
                 — the account has to be connected again"
            ));
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

    use super::Refusal;
    use crate::unix::cf::{self, CFDictionaryRef, CFStringRef, CFTypeRef};

    type OSStatus = i32;
    const ERR_SEC_SUCCESS: OSStatus = 0;
    const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;
    const ERR_SEC_DUPLICATE_ITEM: OSStatus = -25299;
    /// The keychain is locked and nothing may put a prompt on screen — a widget
    /// started at login, before the user has unlocked anything.
    const ERR_SEC_INTERACTION_NOT_ALLOWED: OSStatus = -25308;
    /// The passphrase was wrong, or the item's own access control said no.
    const ERR_SEC_AUTH_FAILED: OSStatus = -25293;
    /// The user dismissed the prompt.
    const ERR_SEC_USER_CANCELED: OSStatus = -128;

    /// Which kind of failure a `SecItem` status is.
    ///
    /// The three listed above are the keychain saying no rather than the
    /// keychain being absent, and every one of them can be gone a minute later:
    /// the machine gets unlocked, the prompt gets answered. Writing the token
    /// to disk in plain text for any of them would be a permanent answer to a
    /// temporary problem.
    ///
    /// Anything else falls back. Not because an unknown status is evidence the
    /// keychain is missing, but because the fallback is this module's
    /// documented behaviour and refusing to store a token on a status nobody
    /// anticipated would leave an account that can never stay connected, with
    /// nothing said about why.
    fn refusal(call: &str, status: OSStatus) -> Refusal {
        let message = format!("{call} failed with status {status}");
        match status {
            ERR_SEC_INTERACTION_NOT_ALLOWED | ERR_SEC_AUTH_FAILED | ERR_SEC_USER_CANCELED => {
                Refusal::Declined(message)
            }
            _ => Refusal::NoKeyring(message),
        }
    }

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

    /// A failure to build a Core Foundation object is an allocation failure and
    /// nothing to do with the keychain, so it takes the fallback rather than
    /// leaving the user unable to sign in.
    fn cannot_build(what: &str) -> Refusal {
        Refusal::NoKeyring(format!("could not build {what}"))
    }

    pub fn store(name: &str, secret: &[u8]) -> Result<(), Refusal> {
        let (service, account) = identity(name).ok_or_else(|| cannot_build("the item identity"))?;
        let value = cf::data(secret).ok_or_else(|| cannot_build("the secret"))?;

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
        .ok_or_else(|| cannot_build("the keychain attributes"))?;

        let status = unsafe { SecItemAdd(attributes.as_raw(), std::ptr::null_mut()) };
        match status {
            ERR_SEC_SUCCESS => Ok(()),
            // Already there from a previous sign-in: replace its value rather
            // than leaving the old token in place.
            ERR_SEC_DUPLICATE_ITEM => update(&service, &account, &value),
            other => Err(refusal("SecItemAdd", other)),
        }
    }

    fn update(service: &cf::Owned, account: &cf::Owned, value: &cf::Owned) -> Result<(), Refusal> {
        let query = unsafe {
            cf::dictionary(
                &[kSecClass, kSecAttrService, kSecAttrAccount],
                &[kSecClassGenericPassword, service.as_raw(), account.as_raw()],
            )
        }
        .ok_or_else(|| cannot_build("the keychain query"))?;
        let changes = unsafe { cf::dictionary(&[kSecValueData], &[value.as_raw()]) }
            .ok_or_else(|| cannot_build("the keychain update"))?;

        match unsafe { SecItemUpdate(query.as_raw(), changes.as_raw()) } {
            ERR_SEC_SUCCESS => Ok(()),
            other => Err(refusal("SecItemUpdate", other)),
        }
    }

    pub fn lookup(name: &str) -> Result<Option<Vec<u8>>, Refusal> {
        let (service, account) = identity(name).ok_or_else(|| cannot_build("the item identity"))?;
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
        .ok_or_else(|| cannot_build("the keychain query"))?;

        let mut result: CFTypeRef = std::ptr::null();
        let status = unsafe { SecItemCopyMatching(query.as_raw(), &mut result) };
        match status {
            ERR_SEC_SUCCESS => {
                // `CopyMatching`: the result is ours to release.
                let owned = cf::Owned::new(result)
                    .ok_or_else(|| Refusal::Declined("the keychain returned nothing".into()))?;
                unsafe { cf::data_bytes(owned.as_raw()) }
                    .map(Some)
                    .ok_or_else(|| Refusal::Declined("the keychain item was not data".into()))
            }
            ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            other => Err(refusal("SecItemCopyMatching", other)),
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

    use super::Refusal;
    use std::io::Write;
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    const TOOL: &str = "secret-tool";
    /// The attribute the widget files its items under. `secret-tool` matches on
    /// attribute pairs rather than on a service and account, so this is the
    /// equivalent of the Keychain's service field.
    const ATTRIBUTE: &str = "tpmplaner-credential";

    /// How long the tool may take before it is killed.
    ///
    /// It is not a background job. `unprotect` is reached from
    /// `AuthState::load` and `Session::new`, which build providers during
    /// start-up and on every sync tick — and `secret-tool lookup` against a
    /// locked collection asks the Secret Service to unlock it, which raises a
    /// passphrase prompt and waits for an answer. On a session where nothing is
    /// listening to prompt, it waits for one that is never coming.
    /// `Stdio::null()` on standard input stops a terminal prompt and does
    /// nothing about a D-Bus one, and a widget that never draws is a worse
    /// outcome than a sign-in that has to be redone.
    ///
    /// Generous on purpose: an unlocked keyring answers in milliseconds, and
    /// this only has to be shorter than "the user thinks it has hung".
    const LIMIT: Duration = Duration::from_secs(5);

    /// Is there a session bus for the Secret Service to be on?
    ///
    /// Two ways to find one: the variable every desktop session exports, and
    /// the socket systemd's user instance leaves in the runtime directory for
    /// the cases where it is not exported. Neither is what a container, a build
    /// agent and a plain `ssh` session look like — which is precisely the
    /// headless case the fallback is documented for. With a bus present, a
    /// failure is the service saying no rather than the service being absent.
    fn session_bus_present() -> bool {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
            return true;
        }
        std::env::var_os("XDG_RUNTIME_DIR")
            .is_some_and(|dir| std::path::Path::new(&dir).join("bus").exists())
    }

    /// Runs `verb` against the keyring, with the secret on standard input where
    /// there is one.
    ///
    /// The secret never becomes an argument: `ps` shows those to every account
    /// on the machine, which would defeat the whole exercise.
    fn run(verb: &str, name: &str, secret: Option<&[u8]>) -> Result<Output, Refusal> {
        let mut command = Command::new(TOOL);
        command.arg(verb);
        if verb == "store" {
            command
                .arg("--label")
                .arg(format!("{} ({name})", super::SERVICE));
        }
        command
            .arg(ATTRIBUTE)
            .arg(name)
            .stdin(if secret.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|e| {
            // Not installed at all, which is the same situation as no keyring
            // and takes the same route.
            Refusal::NoKeyring(format!("could not run {TOOL}: {e}"))
        })?;

        let written = match secret {
            None => Ok(()),
            // Taken rather than borrowed, so the pipe is closed before the
            // wait. `secret-tool store` reads to end of file, and a handle
            // still open in this process is a deadlock: it waits for input that
            // will not come, and this waits for it to exit.
            Some(secret) => match child.stdin.take() {
                None => Err(Refusal::Declined(format!("{TOOL} took no standard input"))),
                Some(mut stdin) => stdin.write_all(secret).map_err(|e| {
                    Refusal::Declined(format!("could not pass the secret to {TOOL}: {e}"))
                }),
            },
        };
        // Waited for whether the write worked or not. Returning on the write
        // error first would drop the `Child`, and `Child::drop` detaches
        // without reaping — Rust installs no `SIGCHLD` handler, so the process
        // stays in the table. `src/unix/host.rs` went to some trouble over the
        // same leak for the browser opener, and `protect` runs on every token
        // refresh.
        let finished = wait_bounded(child, verb);
        written?;
        finished
    }

    /// Waits for the child, killing it once [`LIMIT`] has passed.
    ///
    /// Polled with a backing-off delay rather than a fixed one, so the ordinary
    /// case — an unlocked keyring answering at once — costs a millisecond
    /// rather than a whole tick.
    fn wait_bounded(mut child: Child, verb: &str) -> Result<Output, Refusal> {
        let deadline = Instant::now() + LIMIT;
        let mut nap = Duration::from_millis(1);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Refusal::Declined(format!(
                        "could not wait for {TOOL} {verb}: {e}"
                    )));
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                // Reaped rather than detached, for the reason above.
                let _ = child.wait();
                return Err(Refusal::Declined(format!(
                    "{TOOL} {verb} did not answer within {} seconds — the keyring is \
                     probably waiting for a passphrase nothing can ask for",
                    LIMIT.as_secs()
                )));
            }
            std::thread::sleep(nap);
            nap = (nap * 2).min(Duration::from_millis(50));
        }
        // The child has exited, so the pipes are short reads rather than a
        // wait, and `wait_with_output` returns the status `try_wait` collected.
        child
            .wait_with_output()
            .map_err(|e| Refusal::Declined(format!("could not read {TOOL} {verb}: {e}")))
    }

    pub fn store(name: &str, secret: &[u8]) -> Result<(), Refusal> {
        let output = run("store", name, Some(secret))?;
        if output.status.success() {
            return Ok(());
        }
        Err(tool_refusal("store", &output.stderr, output.status))
    }

    pub fn lookup(name: &str) -> Result<Option<Vec<u8>>, Refusal> {
        let output = run("lookup", name, None)?;

        if !output.status.success() {
            // `secret-tool lookup` exits non-zero for "no such item", which is
            // an answer rather than a failure — and it says nothing on standard
            // error when that is all that happened.
            if output.stderr.is_empty() {
                return Ok(None);
            }
            return Err(tool_refusal("lookup", &output.stderr, output.status));
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

    /// A non-zero exit, classified by whether there is a bus to have failed on.
    fn tool_refusal(verb: &str, stderr: &[u8], status: std::process::ExitStatus) -> Refusal {
        let message = String::from_utf8_lossy(stderr);
        let message = message.trim();
        let detail = if message.is_empty() {
            format!("{TOOL} {verb} failed ({status})")
        } else {
            format!("{TOOL} {verb} failed ({status}): {message}")
        };
        if session_bus_present() {
            Refusal::Declined(detail)
        } else {
            Refusal::NoKeyring(format!("{detail}; there is no session bus"))
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
