// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! CalDAV (RFC 4791) — iCloud, Nextcloud, Fastmail, Synology, mailbox.org and
//! anything else speaking the standard.
//!
//! Two decisions shape this module:
//!
//! * **The server expands recurrences.** The `calendar-query` report asks for
//!   `<C:expand>`, so every returned component is a concrete occurrence.
//!   Implementing `RRULE` here would mean reimplementing the hardest part of
//!   RFC 5545 — and every server already has it.
//! * **The password is moved out of the configuration file on first use.**
//!   CalDAV has no interactive sign-in worth the name; the realistic
//!   credential is an app specific password. Leaving it in plain text next to
//!   the settings would be careless, so it is encrypted into the account's
//!   token file and removed from the JSON.

use super::ical::{self, Stamp};
use super::{CalendarProvider, CalendarRef, Error, Result, api_error, http};
use crate::model::Event;
use crate::secure;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Local, Utc};
use quick_xml::Reader;
use quick_xml::events::Event as XmlEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
struct AccountFile {
    /// Server or collection URL. A bare domain works — discovery walks from
    /// `current-user-principal` to the calendar home.
    url: String,
    username: String,
    /// Only present until the first run moves it into the token file.
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

pub struct CalDavProvider {
    account_id: String,
    display_name: String,
    base_url: String,
    username: String,
    password: String,
    /// Discovered once per process; the calendar home rarely moves.
    home: Option<String>,
}

impl CalDavProvider {
    pub fn new(
        account_id: &str,
        display_name: &str,
        account_file: &PathBuf,
        token_file: PathBuf,
    ) -> Result<Self> {
        let raw = std::fs::read_to_string(account_file).map_err(|_| {
            Error::NeedsSetup(format!(
                "CalDAV account file is missing. Create it with the server \
                 URL, your user name and an app specific password:\n{}",
                account_file.display()
            ))
        })?;
        let mut parsed: AccountFile = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|e| Error::NeedsSetup(format!("{}: {e}", account_file.display())))?;

        let tag = format!("caldav/{account_id}");
        let password = match parsed.password.take() {
            Some(plain) if !plain.is_empty() => {
                // Move it out of the settings file straight away.
                if let Some(encrypted) = secure::protect(plain.as_bytes(), tag.as_bytes()) {
                    if let Some(dir) = token_file.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&token_file, encrypted);
                    if let Ok(json) = serde_json::to_string_pretty(&parsed) {
                        let _ = std::fs::write(account_file, json);
                    }
                    crate::log::info(&format!(
                        "CalDAV '{account_id}': password moved from the settings file into the \
                         encrypted store"
                    ));
                }
                plain
            }
            _ => std::fs::read(&token_file)
                .ok()
                .and_then(|enc| secure::unprotect(&enc, tag.as_bytes()))
                .and_then(|plain| String::from_utf8(plain).ok())
                .ok_or_else(|| {
                    Error::NeedsLogin(format!(
                        "No stored password for CalDAV account '{account_id}'. Add a \
                         \"password\" field to {} once.",
                        account_file.display()
                    ))
                })?,
        };

        Ok(Self {
            account_id: account_id.to_string(),
            display_name: display_name.to_string(),
            base_url: parsed.url.trim_end_matches('/').to_string(),
            username: parsed.username,
            password,
            home: None,
        })
    }

    fn auth_header(&self) -> String {
        format!(
            "Basic {}",
            BASE64.encode(format!("{}:{}", self.username, self.password))
        )
    }

    /// Sends a WebDAV request. `PROPFIND` and `REPORT` are not among the
    /// convenience helpers, so the request is built by hand.
    fn dav(&self, method: &str, url: &str, depth: &str, body: String) -> Result<String> {
        let request = ureq::http::Request::builder()
            .method(method)
            .uri(url)
            .header("Authorization", self.auth_header())
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .map_err(|e| Error::Other(format!("Malformed request: {e}")))?;

        let response = http()
            .run(request)
            .map_err(|e| Error::Other(format!("Network error: {e}")))?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| Error::Other(format!("Unreadable response: {e}")))?;

        if status == 401 || status == 403 {
            return Err(Error::NeedsLogin(format!(
                "{} rejected the credentials for '{}'",
                self.base_url, self.username
            )));
        }
        if !(200..300).contains(&status) {
            return Err(api_error(status, &text));
        }
        Ok(text)
    }

    /// Walks `current-user-principal` → `calendar-home-set`.
    ///
    /// A user who already pasted their calendar home (or a single collection)
    /// should not be punished for it, so a failed discovery falls back to the
    /// configured URL.
    fn discover_home(&mut self) -> Result<String> {
        if let Some(home) = &self.home {
            return Ok(home.clone());
        }

        let principal = self
            .propfind_href(
                &self.base_url,
                r#"<d:prop><d:current-user-principal/></d:prop>"#,
                "current-user-principal",
            )
            .unwrap_or_default();

        let home = if principal.is_empty() {
            self.base_url.clone()
        } else {
            self.propfind_href(
                &principal,
                r#"<d:prop><c:calendar-home-set/></d:prop>"#,
                "calendar-home-set",
            )
            .unwrap_or_else(|| self.base_url.clone())
        };

        self.home = Some(home.clone());
        Ok(home)
    }

    fn propfind_href(&self, url: &str, prop: &str, wanted: &str) -> Option<String> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{prop}</d:propfind>"#
        );
        let xml = self.dav("PROPFIND", url, "0", body).ok()?;
        let href = parse_responses(&xml)
            .into_iter()
            .find_map(|r| r.nested_href.get(wanted).cloned())?;
        Some(self.absolute(&href))
    }

    /// WebDAV returns paths, not URLs.
    fn absolute(&self, href: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            return href.trim_end_matches('/').to_string();
        }
        let origin = origin_of(&self.base_url);
        format!("{origin}{}", href.trim_end_matches('/'))
    }
}

/// `https://host:port/anything` → `https://host:port`
fn origin_of(url: &str) -> String {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    match url[after_scheme..].find('/') {
        Some(slash) => url[..after_scheme + slash].to_string(),
        None => url.to_string(),
    }
}

impl CalendarProvider for CalDavProvider {
    fn account_id(&self) -> &str {
        &self.account_id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn ensure_authorized(&mut self) -> Result<()> {
        // Basic authentication has no separate sign-in step; the first request
        // is the test.
        self.discover_home().map(|_| ())
    }

    fn forget(&mut self) {
        // The password lives in the token file and stays there — there is no
        // token to invalidate, and dropping it would need the user to type it
        // again for no gain.
        self.home = None;
    }

    fn calendars(&mut self) -> Result<Vec<CalendarRef>> {
        let home = self.discover_home()?;
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"
            xmlns:a="http://apple.com/ns/ical/">
  <d:prop>
    <d:resourcetype/>
    <d:displayname/>
    <a:calendar-color/>
    <c:supported-calendar-component-set/>
  </d:prop>
</d:propfind>"#
            .to_string();

        let xml = self.dav("PROPFIND", &home, "1", body)?;
        Ok(parse_responses(&xml)
            .into_iter()
            // Only collections that actually are calendars, and only those
            // holding events — a task-only collection would come back empty.
            .filter(|r| r.is_calendar && r.supports_events)
            .map(|r| CalendarRef {
                color: parse_color(r.props.get("calendar-color").map(String::as_str))
                    .unwrap_or(0x6C_8EBF),
                name: r
                    .props
                    .get("displayname")
                    .filter(|n| !n.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| r.href.clone()),
                id: self_absolute(&self.base_url, &r.href),
            })
            .collect())
    }

    fn events(
        &mut self,
        calendar: &CalendarRef,
        from: DateTime<Local>,
        to: DateTime<Local>,
        _hide_declined: bool,
    ) -> Result<Vec<Event>> {
        let start = from
            .with_timezone(&Utc)
            .format("%Y%m%dT%H%M%SZ")
            .to_string();
        let end = to.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string();

        // `<c:expand>` is what makes recurring events usable without an RRULE
        // implementation on this side.
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <c:calendar-data>
      <c:expand start="{start}" end="{end}"/>
    </c:calendar-data>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{start}" end="{end}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
        );

        let xml = self.dav("REPORT", &calendar.id, "1", body)?;

        let mut events = Vec::new();
        for response in parse_responses(&xml) {
            let Some(data) = response.props.get("calendar-data") else {
                continue;
            };
            for vevent in ical::parse_events(data) {
                if vevent.status.as_deref() == Some("CANCELLED") {
                    continue;
                }
                let all_day = vevent.start.map(Stamp::is_date).unwrap_or(false);
                events.push(Event {
                    title: vevent
                        .summary
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "(no title)".into()),
                    start: vevent.start.and_then(Stamp::as_local),
                    end: vevent.end.and_then(Stamp::as_local),
                    all_day,
                    location: vevent.location.filter(|s| !s.trim().is_empty()),
                    html_link: None,
                    join_url: vevent.url,
                    color: calendar.color,
                    calendar_name: calendar.name.clone(),
                });
            }
        }
        Ok(events)
    }
}

fn self_absolute(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.trim_end_matches('/').to_string()
    } else {
        format!("{}{}", origin_of(base), href.trim_end_matches('/'))
    }
}

/// One `<response>` element, reduced to what this module asks for.
#[derive(Debug, Default)]
struct DavResponse {
    href: String,
    props: HashMap<String, String>,
    /// `href` values nested inside a property, such as `calendar-home-set`.
    nested_href: HashMap<String, String>,
    is_calendar: bool,
    supports_events: bool,
}

/// Reads a WebDAV multistatus document.
///
/// Namespace prefixes differ between servers (`d:`, `D:`, none at all), so
/// everything is matched on local names.
fn parse_responses(xml: &str) -> Vec<DavResponse> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut out: Vec<DavResponse> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut current: Option<DavResponse> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "response" => current = Some(DavResponse::default()),
                    "calendar" if stack.iter().any(|s| s == "resourcetype") => {
                        if let Some(c) = current.as_mut() {
                            c.is_calendar = true;
                        }
                    }
                    "comp" => {
                        // <c:comp name="VEVENT"/> inside the supported set.
                        if let Some(c) = current.as_mut()
                            && attr_value(&e, "name").as_deref() == Some("VEVENT")
                        {
                            c.supports_events = true;
                        }
                    }
                    _ => {}
                }
                stack.push(name);
            }
            Ok(XmlEvent::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                if let Some(c) = current.as_mut() {
                    if name == "calendar" && stack.iter().any(|s| s == "resourcetype") {
                        c.is_calendar = true;
                    }
                    if name == "comp" && attr_value(&e, "name").as_deref() == Some("VEVENT") {
                        c.supports_events = true;
                    }
                }
            }
            Ok(XmlEvent::Text(t)) => {
                let text = t.decode().map(|c| c.into_owned()).unwrap_or_default();
                if text.is_empty() {
                    buf.clear();
                    continue;
                }
                if let Some(c) = current.as_mut() {
                    match stack.last().map(String::as_str) {
                        // The first href belongs to the response itself;
                        // later ones sit inside a property.
                        Some("href") => match stack.iter().rev().nth(1).map(String::as_str) {
                            Some("response") => c.href = text,
                            Some(parent) => {
                                c.nested_href.insert(parent.to_string(), text);
                            }
                            None => {}
                        },
                        Some(other) => {
                            c.props.insert(other.to_string(), text);
                        }
                        None => {}
                    }
                }
            }
            Ok(XmlEvent::CData(t)) => {
                // Some servers wrap calendar-data in CDATA.
                if let Some(c) = current.as_mut()
                    && let Some(name) = stack.last()
                {
                    let text = String::from_utf8_lossy(&t).into_owned();
                    c.props.insert(name.clone(), text);
                }
            }
            Ok(XmlEvent::End(e)) => {
                let name = local_name(e.name().as_ref());
                stack.pop();
                if name == "response"
                    && let Some(done) = current.take()
                {
                    out.push(done);
                }
            }
            Ok(XmlEvent::Eof) => break,
            // A malformed document costs this account its refresh, not the
            // whole sync run.
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    out
}

fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit(':')
        .next()
        .unwrap_or(&name)
        .to_ascii_lowercase()
}

fn attr_value(e: &quick_xml::events::BytesStart, wanted: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (local_name(a.key.as_ref()) == wanted)
            .then(|| String::from_utf8_lossy(&a.value).into_owned())
    })
}

/// Apple's `calendar-color` is `#RRGGBB` or `#RRGGBBAA`.
fn parse_color(value: Option<&str>) -> Option<u32> {
    let s = value?.trim().trim_start_matches('#');
    let hex = s.get(..6)?;
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALENDARS: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"
               xmlns:a="http://apple.com/ns/ical/">
  <d:response>
    <d:href>/calendars/jane/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/></d:resourcetype>
    </d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/calendars/jane/work/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
      <d:displayname>Work</d:displayname>
      <a:calendar-color>#FF5733FF</a:calendar-color>
      <c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set>
    </d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/calendars/jane/chores/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
      <d:displayname>Chores</d:displayname>
      <c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set>
    </d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn only_event_calendars_are_offered() {
        let responses = parse_responses(CALENDARS);
        assert_eq!(responses.len(), 3);

        let usable: Vec<&DavResponse> = responses
            .iter()
            .filter(|r| r.is_calendar && r.supports_events)
            .collect();
        assert_eq!(
            usable.len(),
            1,
            "the plain collection and the task-only calendar must drop out"
        );
        assert_eq!(usable[0].props.get("displayname").unwrap(), "Work");
    }

    #[test]
    fn namespace_prefixes_are_irrelevant() {
        // The same document a different server would send: upper case prefix,
        // a different letter, and the CalDAV namespace bound elsewhere.
        let odd = r#"<?xml version="1.0"?>
<X:multistatus xmlns:X="DAV:" xmlns:CAL="urn:ietf:params:xml:ns:caldav">
  <X:response>
    <X:href>/calendars/jane/work/</X:href>
    <X:propstat><X:prop>
      <X:resourcetype><X:collection/><CAL:calendar/></X:resourcetype>
      <X:displayname>Work</X:displayname>
      <CAL:supported-calendar-component-set><CAL:comp name="VEVENT"/></CAL:supported-calendar-component-set>
    </X:prop></X:propstat>
  </X:response>
</X:multistatus>"#;
        let responses = parse_responses(odd);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].href, "/calendars/jane/work/");
        assert!(responses[0].is_calendar);
        assert!(responses[0].supports_events);
        assert_eq!(responses[0].props.get("displayname").unwrap(), "Work");
    }

    #[test]
    fn a_nested_href_is_kept_apart_from_the_response_href() {
        let xml = r#"<multistatus xmlns="DAV:">
          <response>
            <href>/principals/jane/</href>
            <propstat><prop>
              <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav">
                <href xmlns="DAV:">/calendars/jane/</href>
              </calendar-home-set>
            </prop></propstat>
          </response>
        </multistatus>"#;
        let responses = parse_responses(xml);
        assert_eq!(responses[0].href, "/principals/jane/");
        assert_eq!(
            responses[0].nested_href.get("calendar-home-set").unwrap(),
            "/calendars/jane/"
        );
    }

    #[test]
    fn apple_colours_ignore_the_alpha_suffix() {
        assert_eq!(parse_color(Some("#FF5733FF")), Some(0xFF_5733));
        assert_eq!(parse_color(Some("#FF5733")), Some(0xFF_5733));
        assert_eq!(parse_color(Some("nonsense")), None);
        assert_eq!(parse_color(None), None);
    }

    #[test]
    fn relative_hrefs_are_resolved_against_the_server_origin() {
        assert_eq!(
            origin_of("https://dav.example.org/cal/jane"),
            "https://dav.example.org"
        );
        assert_eq!(
            origin_of("https://dav.example.org:8443/x"),
            "https://dav.example.org:8443"
        );
        assert_eq!(
            origin_of("https://dav.example.org"),
            "https://dav.example.org"
        );
        assert_eq!(
            self_absolute("https://dav.example.org/cal", "/calendars/jane/work/"),
            "https://dav.example.org/calendars/jane/work"
        );
        // An absolute href must be left alone.
        assert_eq!(
            self_absolute("https://dav.example.org/cal", "https://other.example/x/"),
            "https://other.example/x"
        );
    }

    #[test]
    fn a_truncated_document_does_not_panic() {
        assert!(parse_responses("<multistatus><response><href>/a/").is_empty());
        assert!(parse_responses("not xml at all").is_empty());
        assert!(parse_responses("").is_empty());
    }
}
