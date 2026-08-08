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

/// The fallback locale backend: ISO ordering and English names.
///
/// Deliberately plain. A real implementation reaches into the platform's
/// locale database — the Windows host does exactly that — and this exists so
/// tests and non-graphical tools have something predictable.
pub struct PortableLocale;

impl LocaleBackend for PortableLocale {
    fn user_default_tag(&self) -> String {
        // The POSIX convention, reduced to a BCP-47 tag: "de_DE.UTF-8" is
        // "de-DE".
        for key in ["LC_ALL", "LC_TIME", "LANG"] {
            if let Some(value) = std::env::var_os(key) {
                let raw = value.to_string_lossy().to_string();
                let tag = raw.split('.').next().unwrap_or(&raw).replace('_', "-");
                if !tag.is_empty() && tag != "C" && tag != "POSIX" {
                    return tag;
                }
            }
        }
        "en-US".to_string()
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
        let convert = |raw: &str| raw.split('.').next().unwrap_or(raw).replace('_', "-");
        assert_eq!(convert("de_DE.UTF-8"), "de-DE");
        assert_eq!(convert("en_US"), "en-US");
        assert_eq!(convert("fr"), "fr");
    }

    #[test]
    fn accessors_work_before_any_host_is_installed() {
        // Nothing here may panic just because the application has not started
        // yet — unit tests rely on it.
        assert!(!host().data_dir().as_os_str().is_empty());
        assert!(!locale_backend().user_default_tag().is_empty());
    }
}
