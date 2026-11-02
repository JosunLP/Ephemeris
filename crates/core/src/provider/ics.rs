// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Subscribed calendars: an `.ics` file at a web address, or on disk.
//!
//! The one source that needs no sign-in, and the one most people already
//! have: public holidays, school terms, a team's shared calendar, a sports
//! fixture list, the "secret address in iCal format" Google and Outlook offer
//! for every calendar, or an export lying in a folder.
//!
//! Three decisions shape this module:
//!
//! * **Recurrences are expanded here.** A feed is a file; there is no server to
//!   ask. See [`super::recur`] for what is supported and why the rest is
//!   refused rather than guessed.
//! * **The address is a credential.** A private feed address grants read
//!   access to whoever holds it, so it lives in the account file next to the
//!   other credentials rather than in `config.json`, calendar ids are a hash
//!   of it rather than the address, and the log names only its host.
//! * **Unchanged feeds are not downloaded again.** The `ETag` and
//!   `Last-Modified` a server sends are handed back, and a `304` reuses what
//!   was read last time. A holiday calendar changes once a year; fetching it
//!   whole every half hour would be pure waste on both ends.

use super::ical::{self, Calendar, Stamp};
use super::{CalendarProvider, CalendarRef, Error, Result, api_error, http};
use crate::model::Event;
use chrono::{DateTime, Local};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Largest feed accepted. A full export of a busy calendar over many years
/// runs to a few megabytes; anything bigger is not a calendar.
const MAX_FEED_BYTES: u64 = 32 * 1024 * 1024;

/// A feed read within this long is not asked for again — `calendars` and
/// `events` run back to back in one sync and must not fetch twice.
const REUSE_FOR: Duration = Duration::from_secs(60);

/// Colours for feeds that name none, in the order feeds are listed.
const PALETTE: [u32; 8] = [
    0x4C8DF6, 0x33B679, 0xF4511E, 0x8E24AA, 0xF6BF26, 0x039BE5, 0xE67C73, 0x616161,
];

#[derive(Debug, Deserialize)]
struct AccountFile {
    /// One feed: the short form.
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    color: Option<String>,
    /// Several feeds in one account.
    #[serde(default)]
    feeds: Vec<FeedFile>,
}

#[derive(Debug, Deserialize)]
struct FeedFile {
    url: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Source {
    Web(String),
    File(PathBuf),
}

#[derive(Debug, Clone)]
struct Feed {
    id: String,
    source: Source,
    name: Option<String>,
    color: Option<u32>,
    index: usize,
}

struct Cached {
    calendar: Calendar,
    etag: Option<String>,
    last_modified: Option<String>,
    file_stamp: Option<SystemTime>,
    read_at: Instant,
}

pub struct IcsProvider {
    account_id: String,
    display_name: String,
    feeds: Vec<Feed>,
    cache: HashMap<String, Cached>,
}

impl IcsProvider {
    pub fn new(account_id: &str, display_name: &str, account_file: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(account_file).map_err(|_| {
            Error::NeedsSetup(format!(
                "Calendar subscription file is missing. Create it with the feed address:\n{}",
                account_file.display()
            ))
        })?;
        let parsed: AccountFile = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|e| Error::NeedsSetup(format!("{}: {e}", account_file.display())))?;
        let feeds = feeds_from(parsed, account_file.parent())?;
        Ok(Self {
            account_id: account_id.to_string(),
            display_name: display_name.to_string(),
            feeds,
            cache: HashMap::new(),
        })
    }

    /// The feed's document, from the cache when that is still good.
    fn read(&mut self, feed: &Feed) -> Result<&Calendar> {
        let fresh = self
            .cache
            .get(&feed.id)
            .is_some_and(|c| c.read_at.elapsed() < REUSE_FOR);
        if !fresh {
            let previous = self.cache.remove(&feed.id);
            let next = match &feed.source {
                Source::Web(url) => fetch_web(url, previous)?,
                Source::File(path) => read_file(path, previous)?,
            };
            self.cache.insert(feed.id.clone(), next);
        }
        Ok(&self.cache.get(&feed.id).expect("inserted above").calendar)
    }
}

fn feeds_from(file: AccountFile, base: Option<&Path>) -> Result<Vec<Feed>> {
    let mut entries: Vec<FeedFile> = Vec::new();
    if let Some(url) = file.url {
        entries.push(FeedFile {
            url,
            name: file.name,
            color: file.color,
        });
    }
    entries.extend(file.feeds);
    if entries.is_empty() {
        return Err(Error::NeedsSetup(
            "The subscription file lists no feed. Add \"url\": \"https://…/calendar.ics\"".into(),
        ));
    }
    entries
        .into_iter()
        .enumerate()
        .map(|(index, f)| {
            let source = source_of(&f.url, base).ok_or_else(|| {
                Error::NeedsSetup(format!(
                    "'{}' is neither a web address nor a file",
                    redact(&f.url)
                ))
            })?;
            Ok(Feed {
                id: feed_id(&source),
                source,
                name: f.name.filter(|n| !n.trim().is_empty()),
                color: f.color.as_deref().and_then(ical::parse_color),
                index,
            })
        })
        .collect()
}

/// `webcal://` is `https://` with a hint for the operating system to open a
/// calendar app; every service that hands one out serves the same file over
/// HTTPS. Plain `http://` stays as it is — some intranet servers have nothing
/// else — and anything without a scheme is a file.
fn source_of(raw: &str, base: Option<&Path>) -> Option<Source> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    for scheme in ["webcals://", "webcal://"] {
        if lower.starts_with(scheme) {
            return Some(Source::Web(format!("https://{}", &raw[scheme.len()..])));
        }
    }
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(Source::Web(raw.to_string()));
    }
    if let Some(rest) = lower
        .strip_prefix("file://")
        .map(|_| &raw["file://".len()..])
    {
        // `file:///C:/x.ics` on Windows, `file:///home/x.ics` elsewhere.
        let rest = rest
            .strip_prefix('/')
            .filter(|r| r.get(1..2) == Some(":"))
            .unwrap_or(rest);
        return Some(Source::File(PathBuf::from(percent_decode(rest))));
    }
    if lower.contains("://") {
        return None;
    }
    let path = PathBuf::from(raw);
    Some(Source::File(match base {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A stable id that does not give the address away: it ends up in
/// `config.json` when the user picks calendars from the menu.
fn feed_id(source: &Source) -> String {
    let key = match source {
        Source::Web(u) => u.clone(),
        Source::File(p) => p.to_string_lossy().into_owned(),
    };
    let digest = Sha256::digest(key.as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("ics-{hex}")
}

/// Only the host of an address, for the log and the status line.
fn redact(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Credentials in the authority part are the most secret bit of all.
    host.rsplit('@').next().unwrap_or(host).to_string()
}

fn fallback_name(source: &Source) -> String {
    match source {
        Source::Web(url) => redact(url),
        Source::File(path) => path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Calendar".into()),
    }
}

fn fetch_web(url: &str, previous: Option<Cached>) -> Result<Cached> {
    let host = redact(url);
    let mut request = http().get(url).header("Accept", "text/calendar, */*;q=0.5");
    if let Some(prev) = &previous {
        if let Some(etag) = &prev.etag {
            request = request.header("If-None-Match", etag);
        }
        if let Some(lm) = &prev.last_modified {
            request = request.header("If-Modified-Since", lm);
        }
    }
    let response = request
        .call()
        .map_err(|e| Error::Other(format!("{host}: network error: {e}")))?;
    let status = response.status().as_u16();

    if status == 304
        && let Some(mut prev) = previous
    {
        prev.read_at = Instant::now();
        return Ok(prev);
    }

    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let etag = header("etag");
    let last_modified = header("last-modified");

    let body = response
        .into_body()
        .into_with_config()
        .limit(MAX_FEED_BYTES)
        .read_to_string()
        .map_err(|e| Error::Other(format!("{host}: unreadable feed: {e}")))?;

    match status {
        200..=299 => {}
        401 | 403 => {
            return Err(Error::NeedsSetup(format!(
                "{host} refused the calendar subscription (HTTP {status}). A private feed \
                 address stops working when it is reset; copy the current one into the \
                 subscription file"
            )));
        }
        404 | 410 => {
            return Err(Error::NeedsSetup(format!(
                "{host} has no calendar at that address (HTTP {status})"
            )));
        }
        _ => return Err(api_error(status, &body)),
    }

    Ok(Cached {
        calendar: parse_checked(&body, &host)?,
        etag,
        last_modified,
        file_stamp: None,
        read_at: Instant::now(),
    })
}

fn read_file(path: &Path, previous: Option<Cached>) -> Result<Cached> {
    let name = path.display().to_string();
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some(mut prev) = previous
        && stamp.is_some()
        && prev.file_stamp == stamp
    {
        prev.read_at = Instant::now();
        return Ok(prev);
    }
    let meta = std::fs::metadata(path).map_err(|e| Error::NeedsSetup(format!("{name}: {e}")))?;
    if meta.len() > MAX_FEED_BYTES {
        return Err(Error::Other(format!(
            "{name} is too large to be a calendar"
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| Error::NeedsSetup(format!("{name}: {e}")))?;
    let body = String::from_utf8_lossy(&bytes);
    Ok(Cached {
        calendar: parse_checked(&body, &name)?,
        etag: None,
        last_modified: None,
        file_stamp: stamp,
        read_at: Instant::now(),
    })
}

/// An HTML sign-in page served with status 200 is the usual way a feed
/// address goes wrong; say that instead of showing an empty day.
fn parse_checked(body: &str, origin: &str) -> Result<Calendar> {
    let body = body.trim_start_matches('\u{feff}');
    if !body
        .get(..body.len().min(4096))
        .unwrap_or(body)
        .to_ascii_uppercase()
        .contains("BEGIN:VCALENDAR")
    {
        return Err(Error::Other(format!(
            "{origin} did not return a calendar — is the address the iCal/ICS one?"
        )));
    }
    Ok(ical::parse_calendar(body))
}

impl CalendarProvider for IcsProvider {
    fn account_id(&self) -> &str {
        &self.account_id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn ensure_authorized(&mut self) -> Result<()> {
        Ok(())
    }

    fn forget(&mut self) {
        self.cache.clear();
    }

    /// Reads each feed once to learn the name and colour its publisher gave
    /// it. A feed that cannot be read still gets an entry, under its host
    /// name, so it stays selectable and its error surfaces from `events`.
    fn calendars(&mut self) -> Result<Vec<CalendarRef>> {
        let feeds = self.feeds.clone();
        Ok(feeds
            .iter()
            .map(|feed| {
                let (published_name, published_color) = match self.read(feed) {
                    Ok(cal) => (cal.name.clone(), cal.color),
                    Err(e) => {
                        crate::log::warn(&format!("Subscription '{}': {e}", self.account_id));
                        (None, None)
                    }
                };
                CalendarRef {
                    id: feed.id.clone(),
                    name: feed
                        .name
                        .clone()
                        .or(published_name)
                        .unwrap_or_else(|| fallback_name(&feed.source)),
                    color: feed
                        .color
                        .or(published_color)
                        .unwrap_or(PALETTE[feed.index % PALETTE.len()]),
                }
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
        let Some(feed) = self.feeds.iter().find(|f| f.id == calendar.id).cloned() else {
            // Removed from the file since the directory was cached.
            return Ok(Vec::new());
        };
        let occurrences = self.read(&feed)?.occurrences(from, to);
        Ok(occurrences
            .into_iter()
            .map(|v| {
                let all_day = v.start.map(Stamp::is_date).unwrap_or(false);
                Event {
                    title: v
                        .summary
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "(no title)".into()),
                    start: v.start.and_then(Stamp::as_local),
                    end: v.end.and_then(Stamp::as_local),
                    all_day,
                    days: ical::day_span(v.start, v.end),
                    location: v.location.filter(|s| !s.trim().is_empty()),
                    html_link: None,
                    join_url: v.url.filter(|u| u.starts_with("https://")),
                    color: calendar.color,
                    calendar_name: calendar.name.clone(),
                    calendar_id: calendar.id.clone(),
                    account_id: String::new(),
                    task_list_id: None,
                    task_id: None,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    #[test]
    fn webcal_is_https_and_bare_paths_are_files() {
        assert_eq!(
            source_of("webcal://example.org/a.ics", None),
            Some(Source::Web("https://example.org/a.ics".into()))
        );
        assert_eq!(
            source_of("WEBCALS://example.org/a.ics", None),
            Some(Source::Web("https://example.org/a.ics".into()))
        );
        assert_eq!(
            source_of("http://intranet/a.ics", None),
            Some(Source::Web("http://intranet/a.ics".into()))
        );
        assert_eq!(
            source_of("holidays.ics", Some(Path::new("data"))),
            Some(Source::File(Path::new("data").join("holidays.ics")))
        );
        assert_eq!(
            source_of("file:///C:/Users/me/My%20Calendar.ics", None),
            Some(Source::File(PathBuf::from("C:/Users/me/My Calendar.ics")))
        );
        assert_eq!(
            source_of("file:///home/me/a.ics", None),
            Some(Source::File(PathBuf::from("/home/me/a.ics")))
        );
        assert_eq!(source_of("ftp://x/a.ics", None), None);
        assert_eq!(source_of("  ", None), None);
    }

    #[test]
    fn the_address_never_reaches_the_log_or_the_settings() {
        let secret =
            "https://calendar.google.com/calendar/ical/me%40gmail.com/private-0123abcd/basic.ics";
        assert_eq!(redact(secret), "calendar.google.com");
        assert_eq!(
            redact("https://user:pw@dav.example.org/x"),
            "dav.example.org"
        );
        let id = feed_id(&Source::Web(secret.into()));
        assert!(id.starts_with("ics-") && !id.contains("private"));
        assert_eq!(
            id,
            feed_id(&Source::Web(secret.into())),
            "stable across runs"
        );
    }

    #[test]
    fn short_and_long_forms_of_the_account_file() {
        let short: AccountFile = serde_json::from_str(
            r##"{ "url": "webcal://a/x.ics", "name": "Holidays", "color": "#33b679" }"##,
        )
        .unwrap();
        let feeds = feeds_from(short, None).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].name.as_deref(), Some("Holidays"));
        assert_eq!(feeds[0].color, Some(0x33B679));

        let long: AccountFile = serde_json::from_str(
            r#"{ "feeds": [ { "url": "https://a/1.ics" }, { "url": "https://a/2.ics" } ] }"#,
        )
        .unwrap();
        let feeds = feeds_from(long, None).unwrap();
        assert_eq!(feeds.len(), 2);
        assert_ne!(feeds[0].id, feeds[1].id);

        let empty: AccountFile = serde_json::from_str("{}").unwrap();
        assert!(matches!(feeds_from(empty, None), Err(Error::NeedsSetup(_))));
    }

    #[test]
    fn a_login_page_is_reported_as_such() {
        let html = "<!doctype html><html><body>Sign in</body></html>";
        assert!(parse_checked(html, "host").is_err());
        assert!(parse_checked("\u{feff}BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", "host").is_ok());
    }

    #[test]
    fn a_feed_on_disk_is_read_expanded_and_named() {
        let dir = std::env::temp_dir().join(format!("ephemeris-ics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("team.ics");
        std::fs::write(
            &file,
            "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Team\r\n\
BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Weekly\r\nDTSTART:20261102T090000Z\r\nDTEND:20261102T100000Z\r\n\
RRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261110\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n",
        )
        .unwrap();
        let account = dir.join("ics-team.json");
        std::fs::write(&account, r#"{ "url": "team.ics" }"#).unwrap();

        let mut provider = IcsProvider::new("team", "Team", &account).unwrap();
        let calendars = provider.calendars().unwrap();
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].name, "Team", "the feed's own name is used");
        assert_eq!(calendars[0].color, PALETTE[0]);

        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let from = crate::model::local_day_start(day);
        let to = crate::model::local_day_start(day + chrono::Duration::days(2));
        let events = provider.events(&calendars[0], from, to, false).unwrap();
        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"Holiday"), "{titles:?}");
        let weekly = events
            .iter()
            .find(|e| e.title == "Weekly")
            .expect("expanded");
        assert_eq!(
            weekly.start,
            Some(
                Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0)
                    .unwrap()
                    .with_timezone(&Local)
            )
        );
        let holiday = events.iter().find(|e| e.title == "Holiday").unwrap();
        assert!(holiday.all_day);
        assert_eq!(
            holiday.days.map(|d| d.0),
            NaiveDate::from_ymd_opt(2026, 11, 10)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    use chrono::Utc;
}
