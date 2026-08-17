// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! OAuth 2.0 for installed applications: loopback redirect with PKCE.
//!
//! The old `urn:ietf:wg:oauth:2.0:oob` flow has been switched off by Google.
//! Instead: a short lived HTTP listener on `127.0.0.1` with a random port, the
//! browser redirected there, and the code exchanged for tokens.
//!
//! The refresh token is stored encrypted; the access token never leaves
//! memory.

use super::{Error, Result, agent, api_error, urlencode};
use crate::config;
use crate::i18n;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// Calendars are read only; tasks need write access because the widget can
/// complete them, for which `tasks.readonly` would not be enough.
const SCOPES: &str = "https://www.googleapis.com/auth/calendar.readonly \
                      https://www.googleapis.com/auth/calendar.events.readonly \
                      https://www.googleapis.com/auth/tasks";

/// Contents of the client file downloaded from the Cloud Console.
#[derive(Debug, Deserialize)]
struct ClientSecretFile {
    #[serde(alias = "web")]
    installed: ClientCredentials,
}

#[derive(Debug, Clone, Deserialize)]
struct ClientCredentials {
    client_id: String,
    #[serde(default)]
    client_secret: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredToken {
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

pub struct Auth {
    creds: ClientCredentials,
    refresh_token: Option<String>,
    /// The access token and its expiry, held in memory only.
    access: Option<(String, Instant)>,
}

impl Auth {
    /// Loads the client credentials and any stored refresh token.
    pub fn load() -> Result<Self> {
        let path = config::client_secret_path();
        let raw = std::fs::read_to_string(&path).map_err(|_| {
            Error::NeedsSetup(format!(
                "{}\n{}",
                i18n::global().err_missing_client,
                path.display()
            ))
        })?;

        let parsed: ClientSecretFile = serde_json::from_str(&raw)
            .map_err(|e| Error::NeedsSetup(format!("client_secret.json: {e}")))?;

        let refresh_token = std::fs::read(config::token_path())
            .ok()
            .and_then(|enc| crate::host::host().unprotect(&enc, b"google-token"))
            .and_then(|plain| serde_json::from_slice::<StoredToken>(&plain).ok())
            .map(|t| t.refresh_token);

        Ok(Self {
            creds: parsed.installed,
            refresh_token,
            access: None,
        })
    }

    pub fn has_refresh_token(&self) -> bool {
        self.refresh_token.is_some()
    }

    /// A valid access token, refreshed if needed.
    ///
    /// Sixty seconds of headroom before expiry, so a request cannot fall into
    /// the gap.
    pub fn access_token(&mut self) -> Result<String> {
        if let Some((tok, expiry)) = &self.access
            && Instant::now() + Duration::from_secs(60) < *expiry
        {
            return Ok(tok.clone());
        }

        let refresh = self
            .refresh_token
            .clone()
            .ok_or_else(|| Error::NeedsLogin(i18n::global().err_not_connected.into()))?;

        let body = format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            urlencode(&self.creds.client_id),
            urlencode(&self.creds.client_secret),
            urlencode(&refresh),
        );
        let token = self.post_token(&body).map_err(|e| match e {
            // A rejected refresh token stays rejected: drop it locally, or
            // every following sync runs into the same error.
            Error::NeedsLogin(m) | Error::Other(m) if m.contains("invalid_grant") => {
                self.forget();
                Error::NeedsLogin(i18n::global().err_grant_expired.into())
            }
            other => other,
        })?;

        self.store_access(token)
    }

    /// Interactive first sign-in. Blocks until the user has agreed in the
    /// browser, or the timeout hits.
    pub fn interactive_login(&mut self) -> Result<()> {
        // Port 0 lets the operating system pick a free one.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}");

        // PKCE protects the authorization code should another local process
        // intercept it. Mandatory for loopback redirects. Both secrets are
        // taken before anything is sent: without a secure source there is no
        // sign-in to attempt, and the request must not go out with a verifier
        // and a `state` that protect nothing.
        let verifier = URL_SAFE_NO_PAD.encode(secret(48)?);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = URL_SAFE_NO_PAD.encode(secret(16)?);

        let url = format!(
            "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}\
             &code_challenge={}&code_challenge_method=S256&state={}\
             &access_type=offline&prompt=consent",
            urlencode(&self.creds.client_id),
            urlencode(&redirect_uri),
            urlencode(SCOPES),
            urlencode(&challenge),
            urlencode(&state),
        );

        crate::host::host().open_url(&url);

        let code = wait_for_code(listener, &state)?;

        let body = format!(
            "client_id={}&client_secret={}&code={}&code_verifier={}\
             &grant_type=authorization_code&redirect_uri={}",
            urlencode(&self.creds.client_id),
            urlencode(&self.creds.client_secret),
            urlencode(&code),
            urlencode(&verifier),
            urlencode(&redirect_uri),
        );

        let token = self.post_token(&body)?;
        let refresh = token
            .refresh_token
            .clone()
            .ok_or_else(|| Error::Other(i18n::global().err_no_refresh_token.into()))?;

        self.refresh_token = Some(refresh.clone());
        persist_refresh_token(&refresh);
        self.store_access(token)?;
        Ok(())
    }

    /// Discards the stored credential; the "sign in again" menu entry.
    pub fn forget(&mut self) {
        self.refresh_token = None;
        self.access = None;
        let _ = std::fs::remove_file(config::token_path());
    }

    fn store_access(&mut self, token: TokenResponse) -> Result<String> {
        if token.access_token.is_empty() {
            return Err(Error::Other("Empty access token".into()));
        }
        let ttl = Duration::from_secs(token.expires_in.unwrap_or(3600));
        self.access = Some((token.access_token.clone(), Instant::now() + ttl));
        Ok(token.access_token)
    }

    fn post_token(&self, body: &str) -> Result<TokenResponse> {
        let resp = agent()
            .post(TOKEN_ENDPOINT)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body)?;
        let status = resp.status().as_u16();
        let text = resp.into_body().read_to_string()?;
        if !(200..300).contains(&status) {
            return Err(api_error(status, &text));
        }
        Ok(serde_json::from_str(&text)?)
    }
}

fn persist_refresh_token(refresh: &str) {
    let dir = config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let payload = serde_json::to_vec(&StoredToken {
        refresh_token: refresh.to_owned(),
    });
    if let Ok(bytes) = payload
        && let Some(enc) = crate::host::host().protect(&bytes, b"google-token")
    {
        let _ = std::fs::write(config::token_path(), enc);
    }
}

/// Accepts exactly one loopback request and extracts the code.
///
/// Browsers like to follow up with `/favicon.ico`, so this loops until a
/// request carrying `code=` or `error=` arrives.
fn wait_for_code(listener: TcpListener, expected_state: &str) -> Result<String> {
    // Non-blocking so the deadline is actually a deadline. `accept` has no
    // timeout of its own, so the ordinary cancel — the user closes the consent
    // tab and Google never redirects to the loopback — would otherwise park
    // this call, and with it the sync thread, for the life of the process.
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(300);

    while Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        // BSD and macOS hand the accepted socket the listener's non-blocking
        // flag; Linux does not.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;

        let request_line = {
            let mut reader = BufReader::new(&stream);
            match read_request_line(&mut reader, deadline) {
                Some(line) => line,
                // Nothing usable came in: a stalled client, a stray port scan.
                // Drop it and wait for the redirect that matters.
                None => continue,
            }
        };

        // "GET /?code=...&state=... HTTP/1.1"
        let target = request_line.split_whitespace().nth(1).unwrap_or("");
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut code = None;
        let mut state = None;
        let mut error = None;
        for pair in query.split('&') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            let v = percent_decode(v);
            match k {
                "code" => code = Some(v),
                "state" => state = Some(v),
                "error" => error = Some(v),
                _ => {}
            }
        }

        let cat = i18n::global();
        if let Some(err) = error {
            respond(
                &mut stream,
                cat,
                cat.auth_cancelled_title,
                cat.auth_connected_body,
            );
            return Err(Error::NeedsLogin(format!("Google: {err}")));
        }

        if let Some(code) = code {
            // State check against cross-site request forgery: only our own
            // request counts.
            if state.as_deref() != Some(expected_state) {
                respond(&mut stream, cat, cat.auth_cancelled_title, cat.auth_waiting);
                return Err(Error::NeedsLogin("OAuth state mismatch".into()));
            }
            respond(
                &mut stream,
                cat,
                cat.auth_connected_title,
                cat.auth_connected_body,
            );
            return Ok(code);
        }
        // Background noise such as a favicon request.
        respond(&mut stream, cat, "Ephemeris", cat.auth_waiting);
    }

    Err(Error::NeedsLogin(i18n::global().err_timeout.into()))
}

/// Largest request line taken from the loopback callback.
///
/// The redirect Google sends is a few hundred bytes; past this the sender is
/// either broken or feeding the buffer bytes it never means to terminate.
const MAX_REQUEST_LINE: usize = 8 * 1024;

/// Reads one request line, giving up on the read timeout, on `deadline`, or
/// once [`MAX_REQUEST_LINE`] bytes arrived without a newline.
///
/// `set_read_timeout` only bounds a single `read`, so a client dribbling one
/// byte at a time renews it forever and `read_line` would hold the sync thread
/// for as long as the bytes keep coming. Hence the overall deadline between
/// reads and the cap on what is buffered.
///
/// `None` means the connection yielded no line — the caller drops it and waits
/// for the next request rather than failing the whole sign-in.
///
/// Shared with [`crate::provider::oauth`], which listens on the same kind of
/// loopback socket for the same kind of redirect and needs the same patience
/// with everything else that connects to it.
pub(crate) fn read_request_line<R: Read>(reader: &mut R, deadline: Instant) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while Instant::now() < deadline {
        match reader.read(&mut byte) {
            // The peer closed before finishing the line.
            Ok(0) => return None,
            Ok(_) => match byte[0] {
                b'\n' => return String::from_utf8(line).ok(),
                b'\r' => {}
                b => {
                    if line.len() >= MAX_REQUEST_LINE {
                        return None;
                    }
                    line.push(b);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            // A read timeout arrives as `WouldBlock` or `TimedOut` depending on
            // the platform; either way this connection is done talking.
            Err(_) => return None,
        }
    }
    None
}

/// Random bytes for a sign-in secret, or the error that abandons the sign-in.
///
/// The host has already logged why it could not produce any; this only has to
/// stop the flow. See [`crate::host::Host::random_bytes`] for why there is
/// nothing to fall back to.
fn secret(len: usize) -> Result<Vec<u8>> {
    crate::host::host().random_bytes(len).ok_or_else(|| {
        Error::Other(
            "No secure random source available — the sign-in cannot be started safely.".into(),
        )
    })
}

fn respond(stream: &mut std::net::TcpStream, cat: &i18n::Catalog, title: &str, subtitle: &str) {
    // The catalogue is the caller's, not the global one read afresh: the
    // callback can be waited on for minutes, and a language switched in
    // the meantime would put `lang`/`dir` from one catalogue on a page
    // whose text came from another.
    let attrs = cat.html_attrs();
    let html = format!(
        "<!doctype html><html {attrs}><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"font-family:Segoe UI,sans-serif;background:#1b1f24;color:#e8edf3;\
         display:flex;flex-direction:column;align-items:center;justify-content:center;\
         height:100vh;margin:0\"><h2 style=\"font-weight:600\">{title}</h2>\
         <p style=\"opacity:.65\">{subtitle}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hands out `data` a byte at a time, then reports the socket read timeout
    /// — a client that opened the connection and went quiet mid-line.
    struct Trickle {
        data: &'static [u8],
        pos: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            if self.pos >= self.data.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "timed out",
                ));
            }
            buf[0] = self.data[self.pos];
            self.pos += 1;
            Ok(1)
        }
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(300)
    }

    #[test]
    fn a_complete_request_line_is_returned_without_its_terminator() {
        let mut reader = Trickle {
            data: b"GET /?code=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            pos: 0,
        };
        assert_eq!(
            read_request_line(&mut reader, deadline()).as_deref(),
            Some("GET /?code=abc&state=xyz HTTP/1.1")
        );
    }

    #[test]
    fn partial_input_without_a_newline_gives_up_instead_of_blocking() {
        let mut reader = Trickle {
            data: b"GET /?code=abc",
            pos: 0,
        };
        assert_eq!(read_request_line(&mut reader, deadline()), None);
    }

    #[test]
    fn a_line_that_never_ends_is_capped() {
        let mut reader = std::io::repeat(b'A');
        assert_eq!(read_request_line(&mut reader, deadline()), None);
    }

    #[test]
    fn an_expired_deadline_reads_nothing() {
        let mut reader = Trickle {
            data: b"GET / HTTP/1.1\r\n",
            pos: 0,
        };
        let past = Instant::now() - Duration::from_secs(1);
        assert_eq!(read_request_line(&mut reader, past), None);
        assert_eq!(reader.pos, 0);
    }
}
