// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! A small iCalendar (RFC 5545) reader for the parts a day view needs.
//!
//! Two kinds of document arrive here, and they need different amounts of
//! work:
//!
//! * **From CalDAV**, every recurrence is already expanded. The request asks
//!   the server for that (`<C:expand>`), so each component is one concrete
//!   occurrence and [`parse_events`] only has to read it.
//! * **From a subscribed feed** nothing is expanded — a feed is a file — so
//!   [`Calendar::occurrences`] does it here, with the rule engine in
//!   [`super::recur`], `EXDATE` and `RDATE`, and the moved or cancelled single
//!   occurrences that carry a `RECURRENCE-ID`.
//!
//! Both need time zones. A `TZID` names a zone and the document carries its
//! definition as a `VTIMEZONE`: the offsets and the rules for when they change.
//! Those are read and applied, so the widget needs no time zone database of
//! its own and stays correct for any zone a feed defines — including the
//! Windows names Outlook writes. A `TZID` with no definition is read as the
//! local wall clock and logged, once per name.
//!
//! What needs care besides that is the text format itself: lines are folded
//! at 75 octets, values carry escapes, a date is not the same thing as a
//! timestamp, and an alarm inside an event has a `DESCRIPTION` of its own
//! that must not replace the event's.

use super::recur::{Rule, Unsupported, Until};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::collections::{HashMap, HashSet};

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
    /// `URL`, or else an online meeting link found in the location or the
    /// description — where Outlook and Zoom put theirs.
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

/// The days an all-day entry covers, end exclusive.
///
/// `None` unless the start is a plain date. RFC 5545 makes a missing `DTEND`
/// on a date-valued start a one-day entry.
pub fn day_span(start: Option<Stamp>, end: Option<Stamp>) -> Option<(NaiveDate, NaiveDate)> {
    let Some(Stamp::Date(first)) = start else {
        return None;
    };
    let after = match end {
        Some(Stamp::Date(e)) if e > first => e,
        _ => first + Duration::days(1),
    };
    Some((first, after))
}

/// Reads every `VEVENT` in an iCalendar document, as written.
///
/// For a document whose recurrences are already expanded — what CalDAV
/// returns. A recurring master comes back as its first occurrence only; use
/// [`Calendar::occurrences`] where nothing has been expanded.
///
/// Anything that is not understood is skipped rather than rejected: a single
/// odd property in one event must not cost the user their whole day view.
pub fn parse_events(body: &str) -> Vec<VEvent> {
    let calendar = parse_calendar(body);
    calendar
        .events
        .iter()
        .map(|e| calendar.resolve_single(e))
        .collect()
}

/// A wall-clock or absolute time, before its zone has been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawStamp {
    Date(NaiveDate),
    Utc(NaiveDateTime),
    /// Wall-clock time in a named zone, or floating when `None`.
    Zoned(NaiveDateTime, Option<String>),
}

impl RawStamp {
    fn naive(&self) -> NaiveDateTime {
        match self {
            RawStamp::Date(d) => d.and_time(chrono::NaiveTime::MIN),
            RawStamp::Utc(t) | RawStamp::Zoned(t, _) => *t,
        }
    }

    /// The same kind of stamp, moved to another wall-clock time.
    fn with_naive(&self, at: NaiveDateTime) -> RawStamp {
        match self {
            RawStamp::Date(_) => RawStamp::Date(at.date()),
            RawStamp::Utc(_) => RawStamp::Utc(at),
            RawStamp::Zoned(_, zone) => RawStamp::Zoned(at, zone.clone()),
        }
    }
}

/// One `VEVENT` as written, recurrence and all.
#[derive(Debug, Default, Clone)]
struct RawEvent {
    uid: Option<String>,
    summary: Option<String>,
    location: Option<String>,
    description: Option<String>,
    status: Option<String>,
    url: Option<String>,
    start: Option<RawStamp>,
    end: Option<RawStamp>,
    duration: Option<Duration>,
    rrule: Option<String>,
    rdates: Vec<RawStamp>,
    exdates: Vec<RawStamp>,
    recurrence_id: Option<RawStamp>,
}

/// One `STANDARD` or `DAYLIGHT` block of a `VTIMEZONE`.
#[derive(Debug, Default, Clone)]
struct Observance {
    /// Local wall-clock time of the first onset, in `offset_from`.
    start: Option<NaiveDateTime>,
    offset_from: i32,
    offset_to: i32,
    rrule: Option<String>,
    rdates: Vec<NaiveDateTime>,
}

/// A `VTIMEZONE`, prepared as a sorted list of `(UTC instant, new offset)`.
#[derive(Debug, Clone, Default)]
struct Zone {
    transitions: Vec<(NaiveDateTime, i32)>,
    /// The offset before the first transition.
    initial: i32,
    /// Every distinct offset the zone uses, sorted. Worked out once, since a
    /// zone defined from 1601 has hundreds of transitions but two offsets.
    offsets: Vec<i32>,
}

/// How far ahead the daylight saving transitions of a zone are worked out.
/// Anything a two-day view asks for is far inside it.
const ZONE_HORIZON_YEARS: i32 = 3;

impl Zone {
    fn build(observances: &[Observance]) -> Zone {
        let horizon = NaiveDate::from_ymd_opt(Local::now().year() + ZONE_HORIZON_YEARS, 12, 31)
            .unwrap_or(NaiveDate::MAX);
        let mut transitions = Vec::new();
        for obs in observances {
            let Some(start) = obs.start else { continue };
            let mut onsets: Vec<NaiveDateTime> = match obs.rrule.as_deref().map(Rule::parse) {
                Some(Ok(rule)) => {
                    let until = rule.until.map(|u| match u {
                        Until::Date(d) => d,
                        Until::DateTime { at, utc } => {
                            if utc {
                                (at + Duration::seconds(i64::from(obs.offset_from))).date()
                            } else {
                                at.date()
                            }
                        }
                    });
                    rule.dates(start.date(), until, horizon)
                        .into_iter()
                        .map(|d| d.and_time(start.time()))
                        .collect()
                }
                Some(Err(e)) => {
                    crate::log::warn(&format!(
                        "Time zone rule not understood ({e}); using its first onset only"
                    ));
                    vec![start]
                }
                None => vec![start],
            };
            onsets.extend(obs.rdates.iter().copied());
            for local in onsets {
                // An onset is written in the offset that was in force before it.
                transitions.push((
                    local - Duration::seconds(i64::from(obs.offset_from)),
                    obs.offset_to,
                ));
            }
        }
        transitions.sort_by_key(|t| t.0);
        let initial = observances
            .iter()
            .filter(|o| o.start.is_some())
            .min_by_key(|o| o.start)
            .map(|o| o.offset_from)
            .unwrap_or(0);
        let mut offsets: Vec<i32> = transitions.iter().map(|t| t.1).collect();
        offsets.push(initial);
        offsets.sort_unstable();
        offsets.dedup();
        Zone {
            transitions,
            initial,
            offsets,
        }
    }

    fn offset_at_utc(&self, utc: NaiveDateTime) -> i32 {
        match self.transitions.partition_point(|t| t.0 <= utc) {
            0 => self.initial,
            n => self.transitions[n - 1].1,
        }
    }

    /// Wall-clock time to UTC.
    ///
    /// A time that happens twice, in the hour the clocks go back, is the first
    /// of the two; a time that does not happen, in the hour they go forward,
    /// is read with the offset from before the change. Both are what RFC 5545
    /// section 3.3.5 prescribes.
    fn to_utc(&self, local: NaiveDateTime) -> NaiveDateTime {
        self.offsets
            .iter()
            .map(|&o| local - Duration::seconds(i64::from(o)))
            .filter(|&u| {
                let o = self.offset_at_utc(u);
                u + Duration::seconds(i64::from(o)) == local
            })
            .min()
            .unwrap_or_else(|| {
                let before = self.offset_at_utc(local - Duration::days(1));
                local - Duration::seconds(i64::from(before))
            })
    }
}

/// A whole iCalendar document.
#[derive(Debug, Default)]
pub struct Calendar {
    /// `X-WR-CALNAME`, the name the publisher gave the feed.
    pub name: Option<String>,
    /// `X-APPLE-CALENDAR-COLOR` or RFC 7986 `COLOR` as `0xRRGGBB`.
    pub color: Option<u32>,
    events: Vec<RawEvent>,
    zones: HashMap<String, Zone>,
}

/// Reads a whole document: events, zone definitions, and the feed's own name.
pub fn parse_calendar(body: &str) -> Calendar {
    let mut calendar = Calendar::default();
    // Which components we are inside. Only the innermost one matters for a
    // property, which is what keeps an alarm's DESCRIPTION off its event.
    let mut stack: Vec<String> = Vec::new();
    let mut event: Option<RawEvent> = None;
    let mut zone_id: Option<String> = None;
    let mut observances: Vec<Observance> = Vec::new();
    let mut observance: Option<Observance> = None;

    for line in unfold(body) {
        let Some((name, params, value)) = split_property(&line) else {
            continue;
        };
        let name = name.to_ascii_uppercase();

        if name == "BEGIN" {
            let component = value.trim().to_ascii_uppercase();
            match component.as_str() {
                "VEVENT" if stack.last().map(String::as_str) != Some("VEVENT") => {
                    event = Some(RawEvent::default())
                }
                "VTIMEZONE" => {
                    zone_id = None;
                    observances.clear();
                }
                "STANDARD" | "DAYLIGHT" => observance = Some(Observance::default()),
                _ => {}
            }
            stack.push(component);
            continue;
        }
        if name == "END" {
            let component = value.trim().to_ascii_uppercase();
            // Pop back to the matching BEGIN; an END for something never
            // opened is ignored rather than unwinding everything.
            if let Some(at) = stack.iter().rposition(|c| *c == component) {
                stack.truncate(at);
            } else {
                continue;
            }
            match component.as_str() {
                "VEVENT" => {
                    if let Some(e) = event.take() {
                        calendar.events.push(e);
                    }
                }
                "STANDARD" | "DAYLIGHT" => {
                    if let Some(o) = observance.take() {
                        observances.push(o);
                    }
                }
                "VTIMEZONE" => {
                    if let Some(id) = zone_id.take() {
                        calendar.zones.insert(id, Zone::build(&observances));
                    }
                    observances.clear();
                }
                _ => {}
            }
            continue;
        }

        match stack.last().map(String::as_str) {
            Some("VEVENT") => {
                let Some(e) = event.as_mut() else { continue };
                match name.as_str() {
                    "UID" => e.uid = Some(unescape(&value)),
                    "SUMMARY" => e.summary = Some(unescape(&value)),
                    "LOCATION" => e.location = Some(unescape(&value)),
                    "DESCRIPTION" => e.description = Some(unescape(&value)),
                    "STATUS" => e.status = Some(value.trim().to_ascii_uppercase()),
                    "URL" => e.url = Some(unescape(&value)),
                    "DTSTART" => e.start = parse_raw(&params, &value),
                    "DTEND" => e.end = parse_raw(&params, &value),
                    "DURATION" => e.duration = parse_duration(&value),
                    "RRULE" => e.rrule = Some(value.trim().to_string()),
                    "RDATE" => e.rdates.extend(parse_list(&params, &value)),
                    "EXDATE" => e.exdates.extend(parse_list(&params, &value)),
                    "RECURRENCE-ID" => e.recurrence_id = parse_raw(&params, &value),
                    _ => {}
                }
            }
            Some("VTIMEZONE") if name == "TZID" => zone_id = Some(value.trim().to_string()),
            Some("STANDARD" | "DAYLIGHT") => {
                let Some(o) = observance.as_mut() else {
                    continue;
                };
                match name.as_str() {
                    "DTSTART" => o.start = parse_naive(&value),
                    "TZOFFSETFROM" => o.offset_from = parse_offset(&value).unwrap_or(0),
                    "TZOFFSETTO" => o.offset_to = parse_offset(&value).unwrap_or(0),
                    "RRULE" => o.rrule = Some(value.trim().to_string()),
                    "RDATE" => o.rdates.extend(value.split(',').filter_map(parse_naive)),
                    _ => {}
                }
            }
            Some("VCALENDAR") => match name.as_str() {
                "X-WR-CALNAME" | "NAME" => {
                    calendar.name = Some(unescape(&value)).filter(|n| !n.trim().is_empty())
                }
                "X-APPLE-CALENDAR-COLOR" | "COLOR" => {
                    calendar.color = calendar.color.or_else(|| parse_color(&value))
                }
                _ => {}
            },
            _ => {}
        }
    }
    calendar
}

impl Calendar {
    /// Number of `VEVENT`s, recurring masters and overrides alike.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Every occurrence that overlaps `from..to`, recurrences expanded.
    ///
    /// Cancelled occurrences are left out, both whole cancelled series and a
    /// single cancelled date of one. A rule the engine would get wrong is not
    /// guessed at: the series shows its first occurrence and the log says why.
    pub fn occurrences(&self, from: DateTime<Local>, to: DateTime<Local>) -> Vec<VEvent> {
        let from_utc = from.naive_utc();
        let to_utc = to.naive_utc();
        let (from_day, to_day) = (from.date_naive(), to.date_naive());

        // Moved or cancelled single occurrences, by series and original start.
        let mut overridden: HashSet<(String, NaiveDateTime)> = HashSet::new();
        for e in &self.events {
            if let (Some(uid), Some(rid)) = (&e.uid, &e.recurrence_id) {
                overridden.insert((uid.clone(), self.instant(rid)));
            }
        }

        let mut out = Vec::new();
        for e in &self.events {
            if e.status.as_deref() == Some("CANCELLED") {
                continue;
            }
            let Some(start) = &e.start else { continue };
            let length = self.length(e, start);

            let starts: Vec<RawStamp> = if e.recurrence_id.is_some() {
                vec![start.clone()]
            } else {
                self.series(e, start, to_utc)
            };

            for occurrence in starts {
                let original = self.instant(&occurrence);
                if e.recurrence_id.is_none()
                    && let Some(uid) = &e.uid
                    && overridden.contains(&(uid.clone(), original))
                {
                    continue;
                }
                let end = occurrence.with_naive(occurrence.naive() + length);
                let overlaps = match (&occurrence, &end) {
                    (RawStamp::Date(s), RawStamp::Date(e)) => {
                        *s < to_day && (*e > from_day || *s >= from_day)
                    }
                    _ => {
                        let (s, e) = (original, self.instant(&end));
                        s < to_utc && (e > from_utc || (e == s && s >= from_utc))
                    }
                };
                if overlaps {
                    let mut v = self.resolve_single(e);
                    v.start = Some(self.stamp(&occurrence));
                    v.end = Some(self.stamp(&end));
                    out.push(v);
                }
            }
        }
        out.sort_by_key(|v| match v.start {
            Some(Stamp::Date(d)) => (0, d.and_time(chrono::NaiveTime::MIN)),
            Some(Stamp::DateTime(t)) => (1, t.naive_utc()),
            None => (2, NaiveDateTime::MAX),
        });
        out
    }

    /// The start of every occurrence of a series that begins before `to`.
    fn series(&self, e: &RawEvent, start: &RawStamp, to_utc: NaiveDateTime) -> Vec<RawStamp> {
        let mut starts = vec![start.clone()];
        if let Some(text) = &e.rrule {
            let base_time = start.naive().time();
            let parsed = Rule::parse(text).and_then(|rule| {
                if rule.matches_time(base_time) {
                    Ok(rule)
                } else {
                    Err(Unsupported(
                        "its BYHOUR/BYMINUTE/BYSECOND names a time other than its start".into(),
                    ))
                }
            });
            match parsed {
                Ok(rule) => {
                    let base = start.naive();
                    // Generous by two days either side of the zone offsets.
                    let last = (to_utc + Duration::days(2)).date();
                    let until_local = rule.until.map(|u| match u {
                        Until::Date(d) => d
                            .and_hms_opt(23, 59, 59)
                            .unwrap_or(d.and_time(chrono::NaiveTime::MIN)),
                        Until::DateTime { at, utc: false } => at,
                        Until::DateTime { at, utc: true } => self.utc_to_wall(start, at),
                    });
                    starts = rule
                        .dates(base.date(), until_local.map(|u| u.date()), last)
                        .into_iter()
                        .map(|d| d.and_time(base.time()))
                        .filter(|t| until_local.is_none_or(|u| *t <= u))
                        .map(|t| start.with_naive(t))
                        .collect();
                }
                Err(why) => crate::log::warn(&format!(
                    "Recurring entry '{}': {why}; only its first occurrence is shown",
                    e.summary.as_deref().unwrap_or("(no title)")
                )),
            }
        }
        starts.extend(e.rdates.iter().cloned());

        let excluded: HashSet<NaiveDateTime> = e.exdates.iter().map(|x| self.instant(x)).collect();
        let mut seen = HashSet::new();
        starts.retain(|s| {
            let at = self.instant(s);
            !excluded.contains(&at) && seen.insert(at)
        });
        starts
    }

    /// How long one occurrence lasts, measured in the series' own wall clock.
    fn length(&self, e: &RawEvent, start: &RawStamp) -> Duration {
        if let Some(end) = &e.end {
            let d = end.naive() - start.naive();
            if d >= Duration::zero() {
                return d;
            }
        }
        if let Some(d) = e.duration.filter(|d| *d >= Duration::zero()) {
            return d;
        }
        match start {
            // RFC 5545: a date with no end is one day; a time with no end is
            // an instant.
            RawStamp::Date(_) => Duration::days(1),
            _ => Duration::zero(),
        }
    }

    /// Reads one event as written, with its zone applied.
    fn resolve_single(&self, e: &RawEvent) -> VEvent {
        let start = e.start.as_ref().map(|s| self.stamp(s));
        let end = match (&e.end, &e.start, e.duration) {
            (Some(end), _, _) => Some(self.stamp(end)),
            (None, Some(s), Some(d)) => Some(self.stamp(&s.with_naive(s.naive() + d))),
            _ => None,
        };
        let url = e.url.clone().or_else(|| {
            e.location
                .as_deref()
                .and_then(find_join_link)
                .or_else(|| e.description.as_deref().and_then(find_join_link))
        });
        VEvent {
            uid: e.uid.clone(),
            summary: e.summary.clone(),
            location: e.location.clone(),
            description: e.description.clone(),
            status: e.status.clone(),
            start,
            end,
            url,
        }
    }

    fn stamp(&self, raw: &RawStamp) -> Stamp {
        match raw {
            RawStamp::Date(d) => Stamp::Date(*d),
            other => Stamp::DateTime(Utc.from_utc_datetime(&self.instant(other))),
        }
    }

    /// A raw stamp as a UTC instant. Dates are taken at midnight UTC, which
    /// only ever serves as a key for matching exceptions against a series.
    fn instant(&self, raw: &RawStamp) -> NaiveDateTime {
        match raw {
            RawStamp::Date(d) => d.and_time(chrono::NaiveTime::MIN),
            RawStamp::Utc(t) => *t,
            RawStamp::Zoned(t, Some(zone)) => match self.zone(zone) {
                Some(z) => z.to_utc(*t),
                None => floating(*t),
            },
            RawStamp::Zoned(t, None) => floating(*t),
        }
    }

    /// A UTC instant on the wall clock a series is written in.
    fn utc_to_wall(&self, series: &RawStamp, utc: NaiveDateTime) -> NaiveDateTime {
        match series {
            RawStamp::Utc(_) => utc,
            RawStamp::Date(_) => Utc
                .from_utc_datetime(&utc)
                .with_timezone(&Local)
                .naive_local(),
            RawStamp::Zoned(_, zone) => match zone.as_deref().and_then(|z| self.zone(z)) {
                Some(z) => utc + Duration::seconds(i64::from(z.offset_at_utc(utc))),
                None => Utc
                    .from_utc_datetime(&utc)
                    .with_timezone(&Local)
                    .naive_local(),
            },
        }
    }

    fn zone(&self, id: &str) -> Option<&Zone> {
        if let Some(z) = self.zones.get(id) {
            return Some(z);
        }
        static UTC: Zone = Zone {
            transitions: Vec::new(),
            initial: 0,
            // Empty because a static cannot allocate; `to_utc` then falls
            // back to the offset in force, which is 0 here too.
            offsets: Vec::new(),
        };
        if matches!(
            id.to_ascii_uppercase().as_str(),
            "UTC" | "GMT" | "Z" | "ETC/UTC" | "ETC/GMT" | "UNIVERSAL" | "ZULU"
        ) {
            return Some(&UTC);
        }
        warn_unknown_zone(id);
        None
    }
}

/// A wall-clock time with no zone, read on this machine's clock.
fn floating(t: NaiveDateTime) -> NaiveDateTime {
    Local
        .from_local_datetime(&t)
        .earliest()
        .map(|dt| dt.naive_utc())
        // The hour the clocks skip: an hour later is what the wall shows.
        .or_else(|| {
            Local
                .from_local_datetime(&(t + Duration::hours(1)))
                .earliest()
                .map(|dt| dt.naive_utc())
        })
        .unwrap_or(t)
}

/// Once per zone name per run: a feed that uses one undefined zone uses it in
/// every entry, and one line says everything the log needs.
fn warn_unknown_zone(id: &str) {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|p| p.into_inner());
    if !seen.iter().any(|s| s == id) {
        seen.push(id.to_string());
        crate::log::warn(&format!(
            "Time zone '{id}' is used but not defined in the calendar; reading its times as local"
        ));
    }
}

/// The first online meeting link in a text, if any.
///
/// Outlook puts the Teams link in the description, Zoom in the location; a
/// feed has no field of its own for either.
pub fn find_join_link(text: &str) -> Option<String> {
    const HOSTS: [&str; 8] = [
        "teams.microsoft.com",
        "teams.live.com",
        "zoom.us",
        "meet.google.com",
        "webex.com",
        "whereby.com",
        "meet.jit.si",
        "gotomeeting.com",
    ];
    text.split(|c: char| {
        c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']')
    })
    .filter(|w| w.starts_with("https://"))
    .map(|w| w.trim_end_matches(['.', ',', ';']))
    .find(|w| {
        let host = w["https://".len()..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        HOSTS.iter().any(|h| {
            host.eq_ignore_ascii_case(h) || host.to_ascii_lowercase().ends_with(&format!(".{h}"))
        })
    })
    .map(str::to_owned)
}

/// `DTSTART`, `DTEND`, `RECURRENCE-ID`: a date, a UTC time, or a wall-clock
/// time in the zone its `TZID` names.
fn parse_raw(params: &[(String, String)], value: &str) -> Option<RawStamp> {
    let value = value.trim();
    let is_date = params
        .iter()
        .any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE"));
    if is_date || value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()
            .map(RawStamp::Date);
    }
    if let Some(body) = value.strip_suffix(['Z', 'z']) {
        return parse_naive(body).map(RawStamp::Utc);
    }
    let zone = params
        .iter()
        .find(|(k, _)| k == "TZID")
        .map(|(_, v)| v.clone());
    parse_naive(value).map(|t| RawStamp::Zoned(t, zone))
}

/// `EXDATE` and `RDATE` may list several values, comma separated.
fn parse_list(params: &[(String, String)], value: &str) -> Vec<RawStamp> {
    // An RDATE may also be a PERIOD (`start/end`); its start is what counts.
    value
        .split(',')
        .filter_map(|v| parse_raw(params, v.split('/').next().unwrap_or(v)))
        .collect()
}

fn parse_naive(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    let value = value.strip_suffix(['Z', 'z']).unwrap_or(value);
    NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()
}

/// `+0100`, `-0530`, `+013045`.
fn parse_offset(value: &str) -> Option<i32> {
    let value = value.trim();
    let (sign, digits) = match value.as_bytes().first()? {
        b'+' => (1, &value[1..]),
        b'-' => (-1, &value[1..]),
        _ => (1, value),
    };
    if !(digits.len() == 4 || digits.len() == 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let s: i32 = digits
        .get(4..6)
        .map(|s| s.parse().unwrap_or(0))
        .unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

/// `P1D`, `PT1H30M`, `P2W`, `-PT15M`.
fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (sign, rest) = match value.as_bytes().first()? {
        b'-' => (-1, &value[1..]),
        b'+' => (1, &value[1..]),
        _ => (1, value),
    };
    let rest = rest.strip_prefix(['P', 'p'])?;
    let mut total = Duration::zero();
    let mut number = String::new();
    let mut in_time = false;
    for c in rest.chars() {
        match c.to_ascii_uppercase() {
            'T' => in_time = true,
            d if d.is_ascii_digit() => number.push(d),
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                total += match (unit, in_time) {
                    ('W', false) => Duration::weeks(n),
                    ('D', false) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    number.is_empty().then_some(total * sign)
}

/// `#RRGGBB`, `#RRGGBBAA` (Apple's), or a CSS colour name for RFC 7986 —
/// of which only the plain hex form is worth reading.
pub fn parse_color(value: &str) -> Option<u32> {
    let hex = value.trim().strip_prefix('#')?;
    let hex = hex.get(..6)?;
    u32::from_str_radix(hex, 16).ok()
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

    // ---- Feeds: zones and recurrence ----

    const BERLIN: &str = "BEGIN:VTIMEZONE\r\n\
TZID:Europe/Berlin\r\n\
BEGIN:DAYLIGHT\r\n\
TZOFFSETFROM:+0100\r\n\
TZOFFSETTO:+0200\r\n\
DTSTART:19700329T020000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\n\
END:DAYLIGHT\r\n\
BEGIN:STANDARD\r\n\
TZOFFSETFROM:+0200\r\n\
TZOFFSETTO:+0100\r\n\
DTSTART:19701025T030000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\n\
END:STANDARD\r\n\
END:VTIMEZONE\r\n";

    fn feed(events: &str) -> Calendar {
        parse_calendar(&format!(
            "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Team\r\nX-APPLE-CALENDAR-COLOR:#FF2968FF\r\n{BERLIN}{events}END:VCALENDAR\r\n"
        ))
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    /// A window wide enough that the machine's own zone cannot shift an
    /// occurrence out of it.
    fn window(y: i32, m: u32, from: u32, to: u32) -> (DateTime<Local>, DateTime<Local>) {
        (
            utc(y, m, from, 0, 0).with_timezone(&Local),
            utc(y, m, to, 0, 0).with_timezone(&Local),
        )
    }

    fn starts(events: &[VEvent]) -> Vec<DateTime<Utc>> {
        events
            .iter()
            .filter_map(|e| match e.start {
                Some(Stamp::DateTime(t)) => Some(t),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_feed_names_and_colours_itself() {
        let cal = feed("");
        assert_eq!(cal.name.as_deref(), Some("Team"));
        assert_eq!(cal.color, Some(0xFF2968));
    }

    #[test]
    fn a_zone_definition_is_applied_on_both_sides_of_daylight_saving() {
        let cal = feed(
            "BEGIN:VEVENT\r\nUID:w\r\nSUMMARY:Jour fixe\r\n\
DTSTART;TZID=Europe/Berlin:20261020T090000\r\nDTEND;TZID=Europe/Berlin:20261020T100000\r\n\
RRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\n",
        );
        let (from, to) = window(2026, 10, 19, 29);
        // 09:00 in Berlin is 07:00 UTC in summer time and 08:00 after the
        // clocks went back on 25 October: the meeting stays at nine.
        assert_eq!(
            starts(&cal.occurrences(from, to)),
            [utc(2026, 10, 20, 7, 0), utc(2026, 10, 27, 8, 0)]
        );
    }

    #[test]
    fn the_hour_skipped_in_spring_is_read_with_the_offset_before_it() {
        let cal = feed("");
        let zone = cal.zone("Europe/Berlin").unwrap();
        let skipped = NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert_eq!(zone.to_utc(skipped), utc(2026, 3, 29, 1, 30).naive_utc());
        // The hour that happens twice in autumn is its first, summer, instance.
        let twice = NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert_eq!(zone.to_utc(twice), utc(2026, 10, 25, 0, 30).naive_utc());
    }

    #[test]
    fn exceptions_moves_and_cancellations_are_honoured() {
        let cal = feed(
            "BEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Standup\r\n\
DTSTART;TZID=Europe/Berlin:20261102T091500\r\nDURATION:PT15M\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\r\n\
EXDATE;TZID=Europe/Berlin:20261103T091500\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Standup (moved)\r\n\
RECURRENCE-ID;TZID=Europe/Berlin:20261104T091500\r\n\
DTSTART;TZID=Europe/Berlin:20261104T113000\r\nDTEND;TZID=Europe/Berlin:20261104T114500\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:s\r\nSTATUS:CANCELLED\r\n\
RECURRENCE-ID;TZID=Europe/Berlin:20261105T091500\r\n\
DTSTART;TZID=Europe/Berlin:20261105T091500\r\nEND:VEVENT\r\n",
        );
        let (from, to) = window(2026, 11, 2, 7);
        let got = cal.occurrences(from, to);
        let summary: Vec<(String, DateTime<Utc>)> = got
            .iter()
            .map(|e| {
                (
                    e.summary.clone().unwrap_or_default(),
                    starts(std::slice::from_ref(e))[0],
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("Standup".to_string(), utc(2026, 11, 2, 8, 15)),
                ("Standup (moved)".to_string(), utc(2026, 11, 4, 10, 30)),
                ("Standup".to_string(), utc(2026, 11, 6, 8, 15)),
            ]
        );
        // DURATION gives the end when there is no DTEND.
        assert_eq!(got[0].end, Some(Stamp::DateTime(utc(2026, 11, 2, 8, 30))));
    }

    #[test]
    fn a_timed_until_ends_the_series_at_that_moment() {
        // After the RFC's example: until 7 October 00:00 UTC, so the
        // Tuesday that day at 13:00 UTC is already past the end.
        let cal = parse_calendar(
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:19971002T130000Z\r\n\
RRULE:FREQ=WEEKLY;UNTIL=19971007T000000Z;BYDAY=TU,TH\r\nEND:VEVENT\r\n",
        );
        let got = cal.occurrences(
            utc(1997, 10, 1, 0, 0).with_timezone(&Local),
            utc(1997, 10, 31, 0, 0).with_timezone(&Local),
        );
        assert_eq!(starts(&got), [utc(1997, 10, 2, 13, 0)]);
    }

    #[test]
    fn a_yearly_all_day_entry_lands_on_its_day() {
        let cal = parse_calendar(
            "BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Birthday\r\nDTSTART;VALUE=DATE:19900815\r\n\
RRULE:FREQ=YEARLY\r\nEND:VEVENT\r\n",
        );
        let day = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        let from = crate::model::local_day_start(day);
        let got = cal.occurrences(from, from + Duration::days(2));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].start, Some(Stamp::Date(day)));
        assert_eq!(
            day_span(got[0].start, got[0].end),
            Some((day, day + Duration::days(1)))
        );

        // And not in a window that ends the day before.
        let before = crate::model::local_day_start(day - Duration::days(2));
        assert!(
            cal.occurrences(before, before + Duration::days(2))
                .is_empty()
        );
    }

    #[test]
    fn an_alarm_does_not_overwrite_its_event() {
        let body = "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Review\r\nDESCRIPTION:Agenda attached\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
END:VEVENT\r\n";
        let events = parse_events(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].description.as_deref(), Some("Agenda attached"));
    }

    #[test]
    fn a_join_link_is_found_in_the_description() {
        let body = "BEGIN:VEVENT\r\nUID:x\r\nDESCRIPTION:Join: <https://teams.microsoft.com/l/meetup-join/abc>\\nThanks\r\nEND:VEVENT\r\n";
        assert_eq!(
            parse_events(body)[0].url.as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/abc")
        );
        assert_eq!(find_join_link("see https://example.org/zoom.us/x"), None);
        assert_eq!(
            find_join_link("Room 4 / https://acme.zoom.us/j/123."),
            Some("https://acme.zoom.us/j/123".into())
        );
    }

    #[test]
    fn durations_and_offsets_are_read() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_duration("1H"), None);
        assert_eq!(parse_offset("+0530"), Some(19800));
        assert_eq!(parse_offset("-0800"), Some(-28800));
        assert_eq!(parse_offset("0100x"), None);
    }

    #[test]
    fn utc_named_zones_need_no_definition() {
        let cal = parse_calendar(
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=UTC:20261102T120000\r\nEND:VEVENT\r\n",
        );
        let (from, to) = window(2026, 11, 1, 4);
        assert_eq!(
            starts(&cal.occurrences(from, to)),
            [utc(2026, 11, 2, 12, 0)]
        );
    }
}
