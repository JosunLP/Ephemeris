//! Google-API-Anbindung: OAuth, Kalender, Tasks.
//!
//! Alles synchron/blockierend — laeuft ausschliesslich im Sync-Thread. Kein
//! async-Runtime, das spart im Leerlauf einen ganzen Thread-Pool.

pub mod auth;
pub mod calendar;
pub mod tasks;

use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Error {
    /// `client_secret.json` fehlt oder ist unbrauchbar — einmalige Einrichtung noetig.
    NeedsSetup(String),
    /// Kein/ungueltiger Refresh-Token: Benutzer muss sich (neu) anmelden.
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
        Error::Other(format!("Netzwerkfehler: {e}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Other(format!("Unerwartete Antwort: {e}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Other(format!("E/A-Fehler: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Ein einziger Agent fuer den gesamten Prozess: haelt die TLS-Session und
/// die Verbindung zu googleapis.com am Leben, sodass ein Sync-Lauf nicht
/// jedesmal einen kompletten Handshake zahlt.
pub fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            // Wir wollen den Fehler-Body von Google lesen koennen, statt nur
            // einen nackten Statuscode zu bekommen.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("TPMPlaner/0.1 (Windows Desktop Gadget)")
            .build()
            .new_agent()
    })
}

/// Minimaler Percent-Encoder fuer Query-Parameter (RFC 3986 unreserved).
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

/// Zieht die von Google gelieferte Fehlerbeschreibung aus dem Body, damit im
/// Widget nicht nur "HTTP 403" steht.
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
        401 => Error::NeedsLogin(format!("Anmeldung abgelaufen: {detail}")),
        403 if detail.contains("has not been used") || detail.contains("is disabled") => {
            Error::NeedsSetup(format!("API nicht aktiviert: {detail}"))
        }
        _ => Error::Other(format!("HTTP {status}: {detail}")),
    }
}
