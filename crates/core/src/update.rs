// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Checks GitHub Releases for a newer build.
//!
//! The widget runs unattended for weeks at a time, so it has to notice
//! updates on its own. What it does **not** do is install them behind the
//! user's back: it says a new version exists, and the context menu offers to
//! fetch it. A desktop widget replacing its own binary without being asked is
//! the kind of surprise nobody wants.
//!
//! The check is cheap — one request a day, and only after the widget has been
//! running for a while, so a machine that reboots often does not hammer the
//! API.

use serde::Deserialize;
use std::time::Duration;

const RELEASES_API: &str = "https://api.github.com/repos/JosunLP/TPMPlaner/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/JosunLP/TPMPlaner/releases/latest";

/// At most one check per day.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub url: String,
}

/// A three part version, tolerant of a leading `v` and trailing pre-release
/// markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u32, u32, u32);

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let cleaned = text.trim().trim_start_matches(['v', 'V']);
        // Drop anything after a pre-release or build marker: "1.2.3-rc1".
        let core = cleaned.split(['-', '+']).next().unwrap_or(cleaned);
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        // A two part tag is still a version; the missing parts are zero.
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some(Version(major, minor, patch))
    }
}

pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
}

/// Asks GitHub for the latest release.
///
/// Returns `None` when the network is unavailable, the repository has no
/// release yet, or the published version is not newer — none of which is worth
/// bothering the user about.
pub fn check() -> Option<Available> {
    let response = crate::provider::http()
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    if !(200..300).contains(&response.status().as_u16()) {
        return None;
    }
    let body = response.into_body().read_to_string().ok()?;
    let release: Release = serde_json::from_str(&body).ok()?;

    // Drafts and pre-releases are for people who went looking for them.
    if release.draft || release.prerelease {
        return None;
    }

    let latest = Version::parse(&release.tag_name)?;
    (latest > current()).then(|| Available {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: if release.html_url.is_empty() {
            RELEASES_PAGE.to_string()
        } else {
            release.html_url
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_parse_with_or_without_the_v() {
        assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("  V0.1.0 "), Some(Version(0, 1, 0)));
    }

    #[test]
    fn short_tags_fill_the_missing_parts_with_zero() {
        assert_eq!(Version::parse("v2"), Some(Version(2, 0, 0)));
        assert_eq!(Version::parse("v2.1"), Some(Version(2, 1, 0)));
    }

    #[test]
    fn prerelease_markers_are_ignored_for_ordering() {
        assert_eq!(Version::parse("v1.2.3-rc1"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("v1.2.3+build7"), Some(Version(1, 2, 3)));
    }

    #[test]
    fn ordering_is_numeric_not_lexicographic() {
        // The classic trap: "0.10.0" sorts before "0.9.0" as text.
        assert!(Version::parse("v0.10.0").unwrap() > Version::parse("v0.9.0").unwrap());
        assert!(Version::parse("v1.0.0").unwrap() > Version::parse("v0.99.99").unwrap());
        assert!(Version::parse("v1.2.10").unwrap() > Version::parse("v1.2.9").unwrap());
    }

    #[test]
    fn nonsense_tags_are_rejected_rather_than_guessed() {
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("v"), None);
    }

    #[test]
    fn the_compiled_version_is_readable() {
        // Guards against a Cargo.toml version that this parser cannot read,
        // which would silently make every release look like an update.
        assert!(Version::parse(env!("CARGO_PKG_VERSION")).is_some());
        assert!(current() > Version(0, 0, 0));
    }
}
