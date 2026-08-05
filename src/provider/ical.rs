// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! A small iCalendar (RFC 5545) reader for the parts a day view needs.
//!
//! This deliberately does **not** implement recurrence rules. The CalDAV
//! request asks the server to expand them (`<C:expand>`), so what arrives here
//! is always a concrete occurrence with a real start and end. Reimplementing
//! `RRULE` with `BYDAY`, `BYSETPOS`, `COUNT`, `UNTIL` and their daylight
//! saving edge cases is where hand written CalDAV clients usually go wrong,
//! and every server already has that code.
//!
//! What does need care is the text format itself: lines are folded at 75
//! octets, values carry escapes, and a date is not the same thing as a
//! timestamp.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// One `VEVENT`, reduced to the fields the widget shows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct VEvent {
    pub uid: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub start: Option<Stamp>,
    pub end: Option<Stamp>,
    /// `X-...CONFERENCE`, `URL` or a join link found in the description.
    pub url: Option<String>,
}

/// A point in time, or a whole day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// `VALUE=DATE` — a calendar day with no time of day.
    Date(NaiveDate),
    DateTime(DateTime<Utc>),
}

impl Stamp {
    pub fn as_local(self) -> Option<DateTime<Local>> {
        match self {
            Stamp::Date(_) => None,
            Stamp::DateTime(dt) => Some(dt.with_timezone(&Local)),
        }
    }

    pub fn is_date(self) -> bool {
        matches!(self, Stamp::Date(_))
    }
}

/// Reads every `VEVENT` in an iCalendar document.
///
/// Anything that is not understood is skipped rather than rejected: a single
/// odd property in one event must not cost the user their whole day view.
pub fn parse_events(body: &str) -> Vec<VEvent> {
    let mut events = Vec::new();
    let mut current: Option<VEvent> = None;

    for line in unfold(body) {
        let Some((name, params, value)) = split_property(&line) else {
            continue;
        };

        match name.to_ascii_uppercase().as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => {
                current = Some(VEvent::default());
            }
            "END" if value.eq_ignore_ascii_case("VEVENT") => {
                if let Some(event) = current.take() {
                    events.push(event);
                }
            }
            other => {
                let Some(event) = current.as_mut() else {
                    // Properties outside a VEVENT belong to the calendar
                    // itself and are none of our business here.
                    continue;
                };
                match other {
                    "UID" => event.uid = Some(unescape(&value)),
                    "SUMMARY" => event.summary = Some(unescape(&value)),
                    "LOCATION" => event.location = Some(unescape(&value)),
                    "DESCRIPTION" => event.description = Some(unescape(&value)),
                    "STATUS" => event.status = Some(value.to_ascii_uppercase()),
                    "URL" => event.url = Some(unescape(&value)),
                    "DTSTART" => event.start = parse_stamp(&params, &value),
                    "DTEND" => event.end = parse_stamp(&params, &value),
                    _ => {}
                }
            }
        }
    }

    events
}

/// Undoes RFC 5545 line folding.
///
/// A continued line starts with a space or a horizontal tab, and that one
/// character is the fold marker rather than content. Without this step every
/// summary longer than 75 octets would gain stray spaces.
fn unfold(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in body.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) => {
                if let Some(last) = out.last_mut() {
                    last.push_str(rest);
                } else {
                    out.push(rest.to_string());
                }
            }
            None => out.push(line.to_string()),
        }
    }
    out
}

/// Property name, its parameters, and the value.
type Property = (String, Vec<(String, String)>, String);

/// Splits `NAME;PARAM=VALUE:content` into its three parts.
///
/// The colon that ends the parameters may not be the first one in the line —
/// a URL value contains colons too — and a parameter value may be quoted and
/// contain a colon of its own.
fn split_property(line: &str) -> Option<Property> {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut colon = None;
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = line.split_at(colon);
    let value = value[1..].to_string();

    let mut parts = split_unquoted(head, ';');
    if parts.is_empty() {
        return None;
    }
    let name = parts.remove(0);
    let params = parts
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            ))
        })
        .collect();

    Some((name.trim().to_string(), params, value))
}

fn split_unquoted(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            c if c == sep && !in_quotes => out.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    out.push(current);
    out
}

/// `20260805` or `20260805T090000Z` or `20260805T090000`.
fn parse_stamp(params: &[(String, String)], value: &str) -> Option<Stamp> {
    let is_date = params
        .iter()
        .any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE"));

    if is_date || value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()
            .map(Stamp::Date);
    }

    if let Some(naive) = value
        .strip_suffix('Z')
        .and_then(|v| NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok())
    {
        return Some(Stamp::DateTime(Utc.from_utc_datetime(&naive)));
    }

    // No zone marker. With server side expansion this should not happen; if it
    // does, the local zone is the least surprising reading — a floating time
    // means "whatever the clock on the wall says".
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| Stamp::DateTime(dt.with_timezone(&Utc)))
}

/// Undoes the text escaping of RFC 5545 section 3.3.11.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some(',') => out.push(','),
            // An unknown escape keeps both characters rather than eating one.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:abc-123\r\n\
SUMMARY:Sprint Review\r\n\
LOCATION:Room North\r\n\
DTSTART:20260805T090000Z\r\n\
DTEND:20260805T100000Z\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:all-day\r\n\
SUMMARY:Company meeting\r\n\
DTSTART;VALUE=DATE:20260805\r\n\
DTEND;VALUE=DATE:20260806\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn reads_timed_and_all_day_events() {
        let events = parse_events(SAMPLE);
        assert_eq!(events.len(), 2);

        assert_eq!(events[0].summary.as_deref(), Some("Sprint Review"));
        assert_eq!(events[0].location.as_deref(), Some("Room North"));
        let start = events[0].start.expect("start");
        assert!(!start.is_date());
        assert_eq!(
            start.as_local().unwrap().with_timezone(&Utc).to_rfc3339(),
            "2026-08-05T09:00:00+00:00"
        );

        assert!(events[1].start.expect("start").is_date());
        assert!(events[1].start.unwrap().as_local().is_none());
    }

    #[test]
    fn folded_lines_are_joined_without_extra_spaces() {
        // The fold character itself is not content.
        let body = "BEGIN:VEVENT\r\nSUMMARY:Quarterly planning with the\r\n  whole department\r\nEND:VEVENT\r\n";
        let events = parse_events(body);
        assert_eq!(
            events[0].summary.as_deref(),
            Some("Quarterly planning with the whole department")
        );
    }

    #[test]
    fn a_colon_inside_the_value_does_not_end_the_property_name() {
        let body = "BEGIN:VEVENT\r\nURL:https://example.org/a:b\r\nEND:VEVENT\r\n";
        let events = parse_events(body);
        assert_eq!(events[0].url.as_deref(), Some("https://example.org/a:b"));
    }

    #[test]
    fn a_quoted_parameter_may_contain_a_colon() {
        let body = "BEGIN:VEVENT\r\nDTSTART;TZID=\"Europe/Berlin:extra\";VALUE=DATE:20260805\r\nEND:VEVENT\r\n";
        let events = parse_events(body);
        assert_eq!(
            events[0].start,
            Some(Stamp::Date(NaiveDate::from_ymd_opt(2026, 8, 5).unwrap()))
        );
    }

    #[test]
    fn text_escapes_are_undone() {
        assert_eq!(unescape(r"a\,b"), "a,b");
        assert_eq!(unescape(r"a\;b"), "a;b");
        assert_eq!(unescape(r"line\nbreak"), "line\nbreak");
        assert_eq!(unescape(r"back\\slash"), r"back\slash");
        // An unknown escape must not silently swallow a character.
        assert_eq!(unescape(r"a\qb"), r"a\qb");
    }

    #[test]
    fn properties_outside_an_event_are_ignored() {
        // A calendar level SUMMARY must not leak into the first event.
        let body = "BEGIN:VCALENDAR\r\nSUMMARY:My calendar\r\nBEGIN:VEVENT\r\nUID:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse_events(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, None);
    }

    #[test]
    fn an_unterminated_event_is_dropped_rather_than_guessed() {
        let body = "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Truncated\r\n";
        assert!(parse_events(body).is_empty());
    }

    #[test]
    fn lone_newlines_are_accepted_too() {
        // Not every server sends proper CRLF.
        let body = "BEGIN:VEVENT\nUID:x\nSUMMARY:Unix line endings\nEND:VEVENT\n";
        let events = parse_events(body);
        assert_eq!(events[0].summary.as_deref(), Some("Unix line endings"));
    }

    #[test]
    fn a_cancelled_event_is_marked_as_such() {
        let body = "BEGIN:VEVENT\r\nUID:x\r\nSTATUS:cancelled\r\nEND:VEVENT\r\n";
        assert_eq!(parse_events(body)[0].status.as_deref(), Some("CANCELLED"));
    }
}
