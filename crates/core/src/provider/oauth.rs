// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! OAuth 2.0 for installed applications: loopback redirect with PKCE.
//!
//! Google and Microsoft use the same flow; only the endpoints, the scopes and
//! whether a client secret exists differ. The old out-of-band flow
//! (`urn:ietf:wg:oauth:2.0:oob`) is switched off at both providers, so the
//! only remaining option is a short lived HTTP listener on `127.0.0.1` with a
//! random port.
//!
//! Refresh tokens are stored encrypted per account. Access tokens never leave
//! memory.

use crate::provider::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Everything that differs between services.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub auth_url: &'static str,
    pub token_url: &'static str,
    /// Space separated, as the specification requires.
    pub scopes: &'static str,
    /// Appended to the authorization request. Google needs
    /// `access_type=offline` to hand out a refresh token at all.
    pub extra_auth_params: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone)]
pub struct ClientCredentials {
    pub client_id: String,
    /// Google desktop clients ship a "secret" that is not actually secret.
    /// Microsoft public clients have none and reject the parameter.
    pub client_secret: Option<String>,
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

pub struct Session {
    endpoints: Endpoints,
    client: ClientCredentials,
    token_file: PathBuf,
    /// Mixed into the encryption so one account's stored token cannot be
    /// decrypted in the context of another.
    entropy_tag: String,
    /// Used in error messages so the user knows which account is complaining.
    account_label: String,
    refresh_token: Option<String>,
    access: Option<(String, Instant)>,
}

impl Session {
    pub fn new(
        endpoints: Endpoints,
        client: ClientCredentials,
        token_file: PathBuf,
        entropy_tag: impl Into<String>,
        account_label: impl Into<String>,
    ) -> Self {
        let entropy_tag = entropy_tag.into();
        let refresh_token = std::fs::read(&token_file)
            .ok()
            .and_then(|enc| crate::host::host().unprotect(&enc, entropy_tag.as_bytes()))
            .and_then(|plain| serde_json::from_slice::<StoredToken>(&plain).ok())
            .map(|t| t.refresh_token);

        Self {
            endpoints,
            client,
            token_file,
            entropy_tag,
            account_label: account_label.into(),
            refresh_token,
            access: None,
        }
    }

    pub fn has_refresh_token(&self) -> bool {
        self.refresh_token.is_some()
    }

    /// Drops the stored credential so the next run signs in again.
    pub fn forget(&mut self) {
        self.refresh_token = None;
        self.access = None;
        let _ = std::fs::remove_file(&self.token_file);
    }

    /// A valid access token, refreshed if needed.
    ///
    /// One minute of headroom before expiry, so a request cannot fall into the
    /// gap between the check and the server's clock.
    pub fn access_token(&mut self) -> Result<String> {
        if let Some((token, expiry)) = &self.access
            && Instant::now() + Duration::from_secs(60) < *expiry
        {
            return Ok(token.clone());
        }

        let refresh = self
            .refresh_token
            .clone()
            .ok_or_else(|| Error::NeedsLogin(crate::i18n::global().err_not_connected.into()))?;

        let mut body = format!(
            "client_id={}&refresh_token={}&grant_type=refresh_token&scope={}",
            urlencode(&self.client.client_id),
            urlencode(&refresh),
            urlencode(self.endpoints.scopes),
        );
        if let Some(secret) = &self.client.client_secret {
            body.push_str(&format!("&client_secret={}", urlencode(secret)));
        }

        let token = self.post_token(&body).map_err(|e| match e {
            // A rejected refresh token stays rejected. Keeping it would make
            // every following sync fail the same way.
            Error::NeedsLogin(m) | Error::Other(m) if m.contains("invalid_grant") => {
                self.forget();
                Error::NeedsLogin(crate::i18n::global().err_grant_expired.into())
            }
            other => other,
        })?;

        self.store_access(token)
    }

    /// Interactive sign-in. Blocks until the browser round trip completes.
    pub fn interactive_login(&mut self) -> Result<()> {
        // Port 0 lets the operating system pick a free one.
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_err)?;
        let port = listener.local_addr().map_err(io_err)?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}");

        // PKCE protects the authorization code should another local process
        // intercept it. Mandatory for loopback redirects. Both secrets are
        // taken before anything is sent: without a secure source there is no
        // sign-in to attempt, and the request must not go out with a verifier
        // and a `state` that protect nothing.
        let verifier = URL_SAFE_NO_PAD.encode(secret(48)?);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = URL_SAFE_NO_PAD.encode(secret(16)?);

        let mut url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}\
             &code_challenge={}&code_challenge_method=S256&state={}",
            self.endpoints.auth_url,
            urlencode(&self.client.client_id),
            urlencode(&redirect_uri),
            urlencode(self.endpoints.scopes),
            urlencode(&challenge),
            urlencode(&state),
        );
        for (key, value) in self.endpoints.extra_auth_params {
            url.push_str(&format!("&{key}={}", urlencode(value)));
        }

        crate::host::host().open_url(&url);
        let code = self.wait_for_code(listener, &state)?;

        let mut body = format!(
            "client_id={}&code={}&code_verifier={}&grant_type=authorization_code&redirect_uri={}",
            urlencode(&self.client.client_id),
            urlencode(&code),
            urlencode(&verifier),
            urlencode(&redirect_uri),
        );
        if let Some(secret) = &self.client.client_secret {
            body.push_str(&format!("&client_secret={}", urlencode(secret)));
        }

        let token = self.post_token(&body)?;
        let refresh = token
            .refresh_token
            .clone()
            .ok_or_else(|| Error::Other(crate::i18n::global().err_no_refresh_token.into()))?;

        self.refresh_token = Some(refresh.clone());
        self.persist(&refresh);
        self.store_access(token)?;
        Ok(())
    }

    fn persist(&self, refresh: &str) {
        if let Some(dir) = self.token_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let payload = serde_json::to_vec(&StoredToken {
            refresh_token: refresh.to_owned(),
        });
        if let Ok(bytes) = payload
            && let Some(encrypted) =
                crate::host::host().protect(&bytes, self.entropy_tag.as_bytes())
        {
            let _ = std::fs::write(&self.token_file, encrypted);
        }
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
        let response = super::http()
            .post(self.endpoints.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body)
            .map_err(|e| Error::Other(format!("Network error: {e}")))?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| Error::Other(format!("Unreadable response: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(super::api_error(status, &text));
        }
        serde_json::from_str(&text).map_err(|e| Error::Other(format!("Unexpected response: {e}")))
    }

    /// Accepts exactly one loopback request and extracts the code.
    ///
    /// Browsers like to follow up with `/favicon.ico`, so this loops until a
    /// request carrying `code=` or `error=` arrives.
    fn wait_for_code(&self, listener: TcpListener, expected_state: &str) -> Result<String> {
        let cat = crate::i18n::global();
        // Non-blocking so the deadline is actually a deadline. `accept` has no
        // timeout of its own, and a blocking one is only interrupted by a
        // connection — so the ordinary cancel (the user closes the consent tab,
        // and the provider never redirects to the loopback) would park this
        // call forever, and with it the sync thread, for the life of the
        // process: no further sync, no queued completion, no quit.
        listener.set_nonblocking(true).map_err(io_err)?;
        let deadline = Instant::now() + Duration::from_secs(300);

        while Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(e) => return Err(io_err(e)),
            };
            // BSD and macOS hand the accepted socket the listener's
            // non-blocking flag; Linux does not. Setting it either way is what
            // makes the read below behave the same on all three.
            stream.set_nonblocking(false).map_err(io_err)?;
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .map_err(io_err)?;

            // Not `read_line`, and not `?`: a connection that is accepted and
            // then says nothing — a browser's speculative preconnect to the
            // loopback port is the ordinary case, a port scan the other — would
            // time out and abandon the whole sign-in while the real redirect
            // was still waiting in the accept queue. No line means this
            // connection had nothing to say; the next one may.
            let mut reader = BufReader::new(&stream);
            let Some(request_line) = crate::google::auth::read_request_line(&mut reader, deadline)
            else {
                continue;
            };

            // "GET /?code=...&state=... HTTP/1.1"
            let target = request_line.split_whitespace().nth(1).unwrap_or("");
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let (mut code, mut state, mut error) = (None, None, None);
            for pair in query.split('&') {
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                let value = percent_decode(value);
                match key {
                    "code" => code = Some(value),
                    "state" => state = Some(value),
                    "error" => error = Some(value),
                    _ => {}
                }
            }

            if let Some(err) = error {
                respond(
                    &mut stream,
                    cat,
                    cat.auth_cancelled_title,
                    cat.auth_connected_body,
                );
                return Err(Error::NeedsLogin(format!("{}: {err}", self.account_label)));
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

        Err(Error::NeedsLogin(cat.err_timeout.into()))
    }
}

fn io_err(e: std::io::Error) -> Error {
    Error::Other(format!("I/O error: {e}"))
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

fn respond(
    stream: &mut std::net::TcpStream,
    cat: &crate::i18n::Catalog,
    title: &str,
    subtitle: &str,
) {
    // The catalogue is the caller's, not the global one read afresh: the
    // callback can be waited on for minutes, and a language switched in
    // the meantime would put `lang`/`dir` from one catalogue on a page
    // whose text came from another.
    let attrs = cat.html_attrs();
    let html = format!(
        "<!doctype html><html {attrs}><meta charset=\"utf-8\"><title>{title}</title>\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <body style=\"font-family:Segoe UI,system-ui,sans-serif;background:#1b1f24;\
         color:#e8edf3;display:flex;flex-direction:column;align-items:center;\
         justify-content:center;height:100vh;margin:0\">\
         <h2 style=\"font-weight:600\">{title}</h2>\
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

    #[test]
    fn urlencode_escapes_everything_outside_the_unreserved_set() {
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("scope one/two"), "scope%20one%2Ftwo");
        assert_eq!(urlencode("ä"), "%C3%A4");
    }

    #[test]
    fn percent_decode_reverses_the_browser_encoding() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%C3%A4"), "ä");
        // A stray percent must not swallow the rest of the value.
        assert_eq!(percent_decode("100%"), "100%");
    }
}
