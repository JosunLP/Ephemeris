// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The Google APIs: OAuth, Calendar and Tasks.
//!
//! Everything blocking, and only ever called from the sync thread. No async
//! runtime, which saves a whole thread pool while idle.

pub mod auth;
pub mod calendar;
pub mod tasks;

use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Error {
    /// `client_secret.json` is missing or unusable; one-time setup needed.
    NeedsSetup(String),
    /// No or invalid refresh token: the user has to sign in again.
    NeedsLogin(String),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NeedsSetup(m) => write!(f, "{m}"),
            Error::NeedsLogin(m) => write!(f, "{m}"),
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Other(format!("Network error: {e}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Other(format!("Unexpected response: {e}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Other(format!("I/O error: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// One agent for the whole process: keeps the TLS session and the connection
/// to googleapis.com alive, so a sync run does not pay for a full handshake
/// every time.
pub fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            // We want to read Google's error body, not just a bare status
            // code.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("Ephemeris/0.1 (Windows Desktop Gadget)")
            .build()
            .new_agent()
    })
}

/// Minimal percent encoder for query parameters (RFC 3986 unreserved set).
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pulls Google's own error description out of the body, so the widget shows
/// more than "HTTP 403".
pub fn api_error(status: u16, body: &str) -> Error {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| {
                    e.get("message")
                        .or_else(|| e.get("error_description"))
                        .or(Some(e))
                })
                .and_then(|m| m.as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| body.chars().take(200).collect());

    match status {
        401 => Error::NeedsLogin(format!("Sign-in expired: {detail}")),
        403 if detail.contains("has not been used") || detail.contains("is disabled") => {
            Error::NeedsSetup(format!("API not enabled: {detail}"))
        }
        _ => Error::Other(format!("HTTP {status}: {detail}")),
    }
}
