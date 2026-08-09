// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The seam between portable logic and the operating system.
//!
//! Everything in this crate is meant to compile and behave the same on
//! Windows, macOS and Linux. The handful of things that genuinely cannot be
//! written once — where the settings directory lives, how a secret is stored,
//! how a browser is opened, how a locale formats a date — are declared here as
//! traits and supplied by the host application at start-up.
//!
//! Two rules keep this honest:
//!
//! * Nothing in this crate may call an operating system API directly. If it
//!   needs one, it belongs behind a trait in this file.
//! * A missing host is not a crash. Every accessor falls back to a portable
//!   default, so a unit test or a headless tool can use the crate without
//!   installing anything.

use chrono::{DateTime, Local, NaiveDate};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

/// Filesystem, secrets and shell integration.
pub trait Host: Send + Sync {
    /// Where settings, tokens, the cache and the log live.
    fn data_dir(&self) -> PathBuf;

    /// Opens a URL in the user's browser. Used for sign-in and for opening an
    /// event in its web calendar.
    fn open_url(&self, url: &str);

    /// Encrypts a secret so it survives on disk. Bound to the current user
    /// account wherever the platform allows it.
    fn protect(&self, plain: &[u8], tag: &[u8]) -> Option<Vec<u8>>;

    fn unprotect(&self, cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>>;

    /// Cryptographically secure random bytes, for PKCE verifiers and OAuth
    /// state values.
    ///
    /// `None` when the platform has no secure source to hand, and the caller
    /// must then abandon whatever it was about to secure. There is no safe
    /// substitute to return: the callers base64-encode what comes back, so
    /// fewer bytes — none at all, even — does not fail closed. An empty buffer
    /// yields an empty verifier and an empty `state`, and the callback check
    /// compares that empty `state` against whatever the callback carries, so a
    /// request with `state=` set to nothing passes. Losing PKCE and the CSRF
    /// check together, quietly, while the sign-in appears to work, is the one
    /// outcome worth refusing outright.
    fn random_bytes(&self, len: usize) -> Option<Vec<u8>>;
}

/// The schemes a URL may carry before any front end hands it to the platform.
///
/// Deliberately two, and adding a third is meant to be a deliberate act rather
/// than something inherited from "it parsed as a scheme". Everything this
/// program opens today is one of these: the loopback callback during sign-in,
/// the authorisation endpoints, and `Event::html_link`, which is an Outlook or
/// Google Calendar page. Meeting deep links (`msteams:`, `zoommtg:`) would be a
/// one-line addition here once something actually produces them.
const OPENABLE_SCHEMES: &[&str] = &["http", "https"];

/// May this be handed to the platform's opener at all?
///
/// It lives here, next to [`Host::open_url`], because every front end needs the
/// same answer and the input is not always ours. `Event::html_link` comes
/// straight out of the calendar server's JSON, and a shared calendar somebody
/// else can write to is enough to make that hostile. What the openers do with
/// what they are given is not browsing: `ShellExecuteW` with the `open` verb,
/// `open` on macOS and `xdg-open` on Linux all launch whatever is registered
/// for the scheme or the file type, so `file://…`, a UNC path like
/// `\\attacker\share\evil.exe` and any `x-whatever:` handler are all program
/// execution one click away from an agenda row.
///
/// Three rules:
///
/// * The scheme must be one of [`OPENABLE_SCHEMES`].
/// * It must not begin with `-`, or the opener parses it as one of its own
///   options — the one way a URL can act as something other than an argument
///   when no shell is involved.
/// * Something has to follow the scheme; `https:` on its own opens nothing.
///
/// The caller must open the *same* string it checked. Front ends trim first and
/// pass the trimmed value on, so that the bytes checked here and the bytes
/// handed to the opener cannot drift apart.
pub fn is_openable_url(url: &str) -> bool {
    if url.starts_with('-') {
        return false;
    }
    match url.split_once(':') {
        Some((scheme, rest)) => {
            !rest.is_empty() && OPENABLE_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
        }
        None => false,
    }
}

/// Locale-aware date and time formatting.
///
/// Kept separate from [`Host`] because the platform answers are very different
/// in quality: Windows has a complete NLS database, while a portable fallback
/// can only approximate. Splitting the traits means a host can supply a good
/// implementation for one and take the default for the other.
pub trait LocaleBackend: Send + Sync {
    /// The user's preferred BCP-47 tag, for example `de-DE`.
    fn user_default_tag(&self) -> String;

    /// Does this locale read right to left?
    fn is_rtl(&self, tag: &str) -> bool;

    /// Short time, respecting the locale's 12- or 24-hour convention.
    fn format_time(&self, tag: &str, dt: DateTime<Local>) -> Option<String>;

    /// Full weekday name.
    fn format_weekday(&self, tag: &str, date: NaiveDate) -> Option<String>;

    /// Long date **without** the weekday, in the locale's field order.
    fn format_date(&self, tag: &str, date: NaiveDate) -> Option<String>;

    /// Compact day and month, for the task due column.
    fn format_day_month(&self, tag: &str, date: NaiveDate) -> Option<String>;
}

/// Wakes the user interface after background work.
///
/// The sync thread must not know what kind of window it is talking to. On
/// Windows this posts a message; another toolkit would use its own event loop
/// proxy.
pub trait Waker: Send + Sync {
    /// New data or a changed status is available.
    fn wake(&self);
}

static HOST: OnceLock<RwLock<Arc<dyn Host>>> = OnceLock::new();
static LOCALE: OnceLock<RwLock<Arc<dyn LocaleBackend>>> = OnceLock::new();

fn host_slot() -> &'static RwLock<Arc<dyn Host>> {
    HOST.get_or_init(|| RwLock::new(Arc::new(PortableHost)))
}

fn locale_slot() -> &'static RwLock<Arc<dyn LocaleBackend>> {
    LOCALE.get_or_init(|| RwLock::new(Arc::new(PortableLocale)))
}

pub fn set_host(host: Arc<dyn Host>) {
    if let Ok(mut slot) = host_slot().write() {
        *slot = host;
    }
}

pub fn set_locale_backend(backend: Arc<dyn LocaleBackend>) {
    if let Ok(mut slot) = locale_slot().write() {
        *slot = backend;
    }
}

pub fn host() -> Arc<dyn Host> {
    host_slot()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| Arc::new(PortableHost))
}

pub fn locale_backend() -> Arc<dyn LocaleBackend> {
    locale_slot()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| Arc::new(PortableLocale))
}

/// The fallback host: correct paths on every platform, but no real secret
/// storage.
///
/// A host that does not install its own implementation still works — it just
/// keeps its refresh tokens in a file readable by the account that owns it.
/// That is stated plainly rather than dressed up, because pretending to
/// encrypt is worse than not encrypting.
pub struct PortableHost;

impl Host for PortableHost {
    fn data_dir(&self) -> PathBuf {
        // The conventional per-user configuration location of each platform.
        #[cfg(windows)]
        {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                return PathBuf::from(appdata).join("TPMPlaner");
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join("Library/Application Support/TPMPlaner");
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // XDG_CONFIG_HOME wins, then the specified default.
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
                return PathBuf::from(xdg).join("tpmplaner");
            }
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(".config/tpmplaner");
            }
        }
        PathBuf::from(".")
    }

    fn open_url(&self, _url: &str) {
        // Deliberately does nothing: guessing a shell command from a crate
        // that is not allowed to touch the operating system would be worse
        // than doing nothing visibly.
    }

    fn protect(&self, plain: &[u8], _tag: &[u8]) -> Option<Vec<u8>> {
        Some(plain.to_vec())
    }

    fn unprotect(&self, cipher: &[u8], _tag: &[u8]) -> Option<Vec<u8>> {
        Some(cipher.to_vec())
    }

    fn random_bytes(&self, _len: usize) -> Option<Vec<u8>> {
        // There is no portable secure source, so this refuses rather than
        // pretending. It used to return a buffer of zeros, which is a refusal
        // only if every caller checks — and the callers encode what they are
        // given.
        None
    }
}

/// What a POSIX locale name says about the language to use.
///
/// Three-valued on purpose, because "unset" and "`C`" are not the same answer.
/// The first is a question the next source in line gets asked; the second is a
/// deliberate choice of no language, which is how a script or a service unit
/// asks a program for reproducible output. Collapsing the two is how a widget
/// ends up disagreeing with every other program on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PosixLocale {
    /// A language, as a BCP-47 tag.
    Language(String),
    /// `C` or `POSIX`.
    Neutral,
    /// Unset, empty, or nothing but whitespace — a variable that was never
    /// really set.
    Unset,
}

/// Reads a POSIX locale name as a language tag.
///
/// `de_DE.UTF-8@euro` is `de-DE`: the codeset and the modifier are the C
/// library's business and no part of the language.
///
/// Public because the Unix host reads the same variables and has to apply the
/// same rule. It used to carry its own copy — better, and fixed on its own,
/// which is exactly what a second parser does.
pub fn posix_locale(raw: &str) -> PosixLocale {
    let name = raw.split(['.', '@']).next().unwrap_or(raw).trim();
    if name.is_empty() {
        return PosixLocale::Unset;
    }
    if name == "C" || name == "POSIX" {
        return PosixLocale::Neutral;
    }
    PosixLocale::Language(name.replace('_', "-"))
}

/// The locale the POSIX environment asks for: `LC_ALL`, then `LC_TIME`, then
/// `LANG`.
///
/// That is the order the C library resolves them in, and the first variable
/// that says anything decides — including when what it says is `C`. The C
/// library does not fall through there, so neither does this: under
/// `LC_ALL=C LANG=de_DE.UTF-8` the answer is the neutral locale, not German.
pub fn posix_environment_locale() -> PosixLocale {
    for key in ["LC_ALL", "LC_TIME", "LANG"] {
        let Some(value) = std::env::var_os(key) else {
            continue;
        };
        match posix_locale(&value.to_string_lossy()) {
            // Set to nothing is not set. A launcher that exports an empty
            // `LC_ALL` has not chosen a language, and the C library ignores it
            // as well.
            PosixLocale::Unset => continue,
            answered => return answered,
        }
    }
    PosixLocale::Unset
}

/// The fallback locale backend: ISO ordering and English names.
///
/// Deliberately plain. A real implementation reaches into the platform's
/// locale database — the Windows host does exactly that — and this exists so
/// tests and non-graphical tools have something predictable.
pub struct PortableLocale;

impl LocaleBackend for PortableLocale {
    fn user_default_tag(&self) -> String {
        // The neutral locale and an environment that said nothing both land on
        // the same answer here: this backend has one set of names and they are
        // English, so English *is* what `C` asks for.
        match posix_environment_locale() {
            PosixLocale::Language(tag) => tag,
            PosixLocale::Neutral | PosixLocale::Unset => "en-US".to_string(),
        }
    }

    fn is_rtl(&self, tag: &str) -> bool {
        // The languages written right to left, by primary subtag.
        const RTL: &[&str] = &["ar", "he", "fa", "ur", "ps", "sd", "yi", "dv", "ckb"];
        let primary = tag
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        RTL.contains(&primary.as_str())
    }

    fn format_time(&self, _tag: &str, dt: DateTime<Local>) -> Option<String> {
        use chrono::Timelike;
        Some(format!("{:02}:{:02}", dt.hour(), dt.minute()))
    }

    fn format_weekday(&self, _tag: &str, date: NaiveDate) -> Option<String> {
        Some(date.format("%A").to_string())
    }

    fn format_date(&self, _tag: &str, date: NaiveDate) -> Option<String> {
        Some(date.format("%-d %B %Y").to_string())
    }

    fn format_day_month(&self, _tag: &str, date: NaiveDate) -> Option<String> {
        Some(date.format("%-d %b").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_directory_follows_each_platforms_convention() {
        let dir = PortableHost.data_dir();
        let text = dir.to_string_lossy().to_lowercase();
        assert!(!text.is_empty());
        #[cfg(windows)]
        assert!(text.contains("tpmplaner"));
        #[cfg(all(unix, not(target_os = "macos")))]
        assert!(text.contains("tpmplaner"));
        #[cfg(target_os = "macos")]
        assert!(text.contains("application support"));
    }

    #[test]
    fn right_to_left_languages_are_recognised_without_the_platform() {
        let l = PortableLocale;
        assert!(l.is_rtl("ar-SA"));
        assert!(l.is_rtl("he"));
        assert!(l.is_rtl("fa_IR"));
        assert!(!l.is_rtl("de-DE"));
        assert!(!l.is_rtl("en-US"));
        assert!(!l.is_rtl(""));
    }

    #[test]
    fn a_posix_locale_becomes_a_bcp47_tag() {
        // The conversion itself, independent of the environment.
        let language = |raw: &str| match posix_locale(raw) {
            PosixLocale::Language(tag) => tag,
            other => panic!("{raw} should name a language, got {other:?}"),
        };
        assert_eq!(language("de_DE.UTF-8"), "de-DE");
        assert_eq!(language("en_US"), "en-US");
        assert_eq!(language("fr"), "fr");
        // The modifier is no more part of the language than the codeset is,
        // and a variable can arrive with the whitespace a shell script left on
        // it.
        assert_eq!(language("de_DE.UTF-8@euro"), "de-DE");
        assert_eq!(language("ca_ES@valencia"), "ca-ES");
        assert_eq!(language(" de_DE.UTF-8 "), "de-DE");
    }

    #[test]
    fn the_c_locale_is_an_answer_rather_than_a_gap() {
        // The distinction the widget needs is three-valued. `LC_ALL=C` is how
        // a script asks for reproducible output, and reading it as "nothing
        // set" makes `LANG` win — so the widget would print a German agenda
        // where every other program on the machine printed English.
        assert_eq!(posix_locale("C"), PosixLocale::Neutral);
        assert_eq!(posix_locale("POSIX"), PosixLocale::Neutral);
        assert_eq!(posix_locale("C.UTF-8"), PosixLocale::Neutral);
        assert_eq!(posix_locale(""), PosixLocale::Unset);
        assert_eq!(posix_locale("   "), PosixLocale::Unset);
    }

    #[test]
    fn only_web_urls_are_handed_to_an_opener() {
        for good in [
            "https://calendar.google.com/event?eid=1",
            "http://127.0.0.1:8731/callback?code=x",
            // The scheme is case-insensitive; the rest is not ours to judge.
            "HTTPS://outlook.office365.com/owa/",
        ] {
            assert!(is_openable_url(good), "{good}");
        }
        for bad in [
            "",
            "not a url",
            "/etc/passwd",
            // A calendar server can put any of these in `htmlLink`, and every
            // one of them is a program launch rather than a page.
            "file:///Applications/Calculator.app",
            r"\\attacker\share\evil.exe",
            "smb://attacker/share",
            "x-anything://run",
            "msteams://l/meetup-join/19%3ameeting",
            // Would be read as an option by the opener rather than as a target.
            "--version",
            "-x https://example.com",
            // A scheme with nothing after it opens nothing.
            "https:",
            "1https://example.com",
        ] {
            assert!(!is_openable_url(bad), "{bad}");
        }
    }

    #[test]
    fn accessors_work_before_any_host_is_installed() {
        // Nothing here may panic just because the application has not started
        // yet — unit tests rely on it.
        assert!(!host().data_dir().as_os_str().is_empty());
        assert!(!locale_backend().user_default_tag().is_empty());
    }
}
