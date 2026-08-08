// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The widget's domain model: what actually gets drawn.
//!
//! Deliberately decoupled from any provider's JSON shapes, so the renderer
//! knows nothing about an API and the filtering and sorting stay testable.

use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

/// A calendar entry for today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub title: String,
    /// `None` for all-day events.
    pub start: Option<DateTime<Local>>,
    pub end: Option<DateTime<Local>>,
    pub all_day: bool,
    pub location: Option<String>,
    /// Link into the web calendar, opened on click.
    pub html_link: Option<String>,
    /// Join link of an online meeting (Teams, Meet, Zoom).
    ///
    /// For a meeting that is running, "join" is the action actually wanted,
    /// so a click on the row takes this link when there is one and falls back
    /// to `html_link` otherwise.
    #[serde(default)]
    pub join_url: Option<String>,
    /// Colour of the source calendar as `0xRRGGBB`.
    pub color: u32,
    pub calendar_name: String,
    /// Id of the source calendar, as the provider knows it.
    ///
    /// Not drawn — it is the key a per-calendar colour override is looked up
    /// by, alongside the name. Defaulted on read so a cache written before it
    /// existed still loads.
    #[serde(default)]
    pub calendar_id: String,
}

impl Event {
    /// Is the event running right now?
    pub fn is_now(&self, now: DateTime<Local>) -> bool {
        match (self.start, self.end) {
            (Some(s), Some(e)) => s <= now && now < e,
            (Some(s), None) => s <= now && now < s + chrono::Duration::hours(1),
            _ => false,
        }
    }

    /// Is the event already over? Never true for all-day events.
    pub fn is_past(&self, now: DateTime<Local>) -> bool {
        if self.all_day {
            return false;
        }
        match self.end.or(self.start) {
            Some(e) => e <= now,
            None => false,
        }
    }

    /// Sort key: all-day events first, then by start time.
    fn sort_key(&self) -> (u8, i64) {
        if self.all_day {
            (0, 0)
        } else {
            (1, self.start.map(|s| s.timestamp()).unwrap_or(i64::MAX))
        }
    }
}

/// A task from a task list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Needed to complete the task through the API.
    pub id: String,
    pub tasklist_id: String,
    pub title: String,
    /// A plain calendar date. See [`parse_task_due`] for why.
    pub due: Option<NaiveDate>,
    pub notes: Option<String>,
    /// Nesting depth; subtasks are indented by it.
    pub depth: u8,
    pub tasklist_name: String,
    /// Which account this came from. Needed to route the completion call back
    /// to the right provider when several accounts are configured.
    #[serde(default)]
    pub account_id: String,
    /// Set locally while the completion is still on its way to the server.
    /// Purely transient and has no business in the cache.
    #[serde(skip)]
    pub completing: bool,
}

impl Task {
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        matches!(self.due, Some(d) if d < today)
    }
}

/// The complete display state of one sync run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Agenda {
    pub day: Option<NaiveDate>,
    pub events: Vec<Event>,
    /// Tomorrow's events. From late afternoon on, today's list is empty and
    /// the widget would otherwise be a blank panel — which is exactly when
    /// "what is first tomorrow?" becomes the interesting question.
    pub tomorrow: Vec<Event>,
    pub tasks: Vec<Task>,
    pub fetched_at: Option<DateTime<Local>>,
    /// Error text of the last attempt; the previous data stays visible.
    /// Not cached — at start-up the state is simply "unknown".
    #[serde(skip)]
    pub last_error: Option<String>,
}

/// Google Tasks and Microsoft To Do both return `due` as an RFC 3339
/// timestamp, and for a plain due date the time of day is **meaningless**: the
/// API normalises it to `YYYY-MM-DDT00:00:00.000Z`. Parsing that as a real UTC
/// instant and converting it to the local zone lands a day early in every zone
/// east of UTC — "due today" silently becomes "due yesterday".
///
/// So midnight is read as a bare calendar day, from the first ten characters,
/// with no time zone conversion at all.
///
/// A timestamp that carries a **different** time of day is the opposite case:
/// the task has a time, the value is a genuine instant, and only the local zone
/// says which day it falls on. `2026-08-06T22:30:00Z` is half past midnight on
/// the 7th in Berlin, and taking the first ten characters would file it as
/// overdue since yesterday.
pub fn parse_task_due(raw: &str) -> Option<NaiveDate> {
    parse_task_due_in(raw, &Local)
}

/// The zone is a parameter only so both branches can be tested without
/// depending on where the machine running the tests happens to stand.
fn parse_task_due_in<Tz: chrono::TimeZone>(raw: &str, zone: &Tz) -> Option<NaiveDate> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw)
        && dt.time() != NaiveTime::MIN
    {
        return Some(dt.with_timezone(zone).date_naive());
    }
    let date_part = raw.get(..10)?;
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
}

/// Keeps the tasks due **today or earlier**.
///
/// Tasks without a due date drop out unless `show_undated` says otherwise, and
/// so does everything only due in the future — which was the original
/// complaint: the task APIs return the whole list by default.
pub fn filter_tasks_for_today(tasks: Vec<Task>, today: NaiveDate, show_undated: bool) -> Vec<Task> {
    tasks
        .into_iter()
        .filter(|t| match t.due {
            Some(d) => d <= today,
            None => show_undated,
        })
        .collect()
}

/// Sorted by due date, oldest first, undated last; ties broken alphabetically
/// so the order stays stable between two syncs instead of jumping around.
pub fn sort_tasks(tasks: &mut [Task]) {
    tasks.sort_by(|a, b| {
        let ka = a.due.map(|d| (0u8, d)).unwrap_or((1, NaiveDate::MAX));
        let kb = b.due.map(|d| (0u8, d)).unwrap_or((1, NaiveDate::MAX));
        ka.cmp(&kb)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

pub fn sort_events(events: &mut [Event]) {
    events.sort_by_key(|e| e.sort_key());
}

/// Flags events that overlap another one in time.
///
/// Double bookings are near impossible to spot while skimming a list — it
/// means comparing the end of one row against the start of another in your
/// head. The result therefore appears in the section header and tints the
/// affected times.
///
/// All-day events do not count: they overlap everything by definition and
/// would be worthless as a warning.
pub fn mark_overlaps(events: &[Event]) -> Vec<bool> {
    let mut flags = vec![false; events.len()];
    for i in 0..events.len() {
        for j in (i + 1)..events.len() {
            if overlaps(&events[i], &events[j]) {
                flags[i] = true;
                flags[j] = true;
            }
        }
    }
    flags
}

fn overlaps(a: &Event, b: &Event) -> bool {
    if a.all_day || b.all_day {
        return false;
    }
    let (Some(a_start), Some(b_start)) = (a.start, b.start) else {
        return false;
    };
    // Without an end time the one hour default applies, as in
    // `Event::is_now`.
    let a_end = a.end.unwrap_or(a_start + chrono::Duration::hours(1));
    let b_end = b.end.unwrap_or(b_start + chrono::Duration::hours(1));
    // Touching at the boundary is not an overlap: 09:00-10:00 and 10:00-11:00
    // are two consecutive meetings, not a conflict.
    a_start < b_end && b_start < a_end
}

/// Start of the local day as a `DateTime<Local>`.
///
/// On daylight saving transition days midnight can be ambiguous or simply not
/// exist; this takes the earliest valid instant, or falls back to 01:00,
/// rather than panicking.
pub fn local_day_start(day: NaiveDate) -> DateTime<Local> {
    use chrono::TimeZone;
    let midnight = day.and_time(NaiveTime::MIN);
    match Local.from_local_datetime(&midnight).earliest() {
        Some(dt) => dt,
        None => Local
            .from_local_datetime(&day.and_hms_opt(1, 0, 0).unwrap())
            .earliest()
            .unwrap_or_else(|| Local.from_utc_datetime(&midnight)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_date_is_not_timezone_shifted() {
        // Exactly the case that slips to the 3rd when parsed naively in UTC+2.
        let d = parse_task_due("2026-08-04T00:00:00.000Z").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 8, 4).unwrap());
    }

    #[test]
    fn due_date_accepts_plain_date() {
        let d = parse_task_due("2026-08-04").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 8, 4).unwrap());
    }

    #[test]
    fn midnight_keeps_its_calendar_day_in_every_zone() {
        // The plain due date, written the three ways the services use it. None
        // of them may move, wherever the reader stands.
        let berlin = chrono::FixedOffset::east_opt(2 * 3600).expect("valid offset");
        let chicago = chrono::FixedOffset::west_opt(5 * 3600).expect("valid offset");
        for raw in [
            "2026-08-04T00:00:00.000Z",
            "2026-08-04T00:00:00+02:00",
            "2026-08-04T00:00:00-05:00",
        ] {
            for zone in [berlin, chicago] {
                assert_eq!(
                    parse_task_due_in(raw, &zone).unwrap(),
                    NaiveDate::from_ymd_opt(2026, 8, 4).unwrap(),
                    "{raw} was shifted in {zone}"
                );
            }
        }
    }

    #[test]
    fn a_due_time_lands_on_the_local_day_not_the_utc_one() {
        let aug_7 = NaiveDate::from_ymd_opt(2026, 8, 7).unwrap();

        // Half past midnight on the 7th in Berlin. Reading the first ten
        // characters files it as due on the 6th — overdue since yesterday.
        let berlin = chrono::FixedOffset::east_opt(2 * 3600).expect("valid offset");
        assert_eq!(
            parse_task_due_in("2026-08-06T22:30:00.000Z", &berlin).unwrap(),
            aug_7
        );

        // And the mirror image west of UTC: late on the 7th in Chicago.
        let chicago = chrono::FixedOffset::west_opt(5 * 3600).expect("valid offset");
        assert_eq!(
            parse_task_due_in("2026-08-08T02:00:00.000Z", &chicago).unwrap(),
            aug_7
        );
    }

    fn task(title: &str, due: Option<&str>) -> Task {
        Task {
            id: title.into(),
            tasklist_id: "l".into(),
            title: title.into(),
            due: due.and_then(parse_task_due),
            notes: None,
            depth: 0,
            tasklist_name: "Liste".into(),
            account_id: String::new(),
            completing: false,
        }
    }

    #[test]
    fn future_tasks_are_dropped_overdue_are_kept() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let input = vec![
            task("in three weeks", Some("2026-08-25")),
            task("today", Some("2026-08-04")),
            task("overdue", Some("2026-07-30")),
            task("undated", None),
        ];
        let kept = filter_tasks_for_today(input, today, false);
        let titles: Vec<_> = kept.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["today", "overdue"]);
    }

    #[test]
    fn undated_tasks_can_be_opted_in_and_sort_last() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let input = vec![task("undated", None), task("today", Some("2026-08-04"))];
        let mut kept = filter_tasks_for_today(input, today, true);
        sort_tasks(&mut kept);
        let titles: Vec<_> = kept.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["today", "undated"]);
    }

    fn timed(title: &str, from: (u32, u32), to: (u32, u32)) -> Event {
        use chrono::TimeZone;
        let at = |h: u32, m: u32| Local.with_ymd_and_hms(2026, 8, 4, h, m, 0).unwrap();
        Event {
            title: title.into(),
            start: Some(at(from.0, from.1)),
            end: Some(at(to.0, to.1)),
            all_day: false,
            location: None,
            html_link: None,
            join_url: None,
            color: 0,
            calendar_name: "K".into(),
            calendar_id: "k".into(),
        }
    }

    #[test]
    fn back_to_back_events_are_not_a_conflict() {
        // 09:00-10:00 and 10:00-11:00 merely touch.
        let events = vec![timed("a", (9, 0), (10, 0)), timed("b", (10, 0), (11, 0))];
        assert_eq!(mark_overlaps(&events), vec![false, false]);
    }

    #[test]
    fn genuine_double_bookings_are_flagged() {
        let events = vec![
            timed("a", (9, 0), (10, 0)),
            timed("b", (9, 30), (10, 30)),
            timed("c", (14, 0), (15, 0)),
        ];
        assert_eq!(mark_overlaps(&events), vec![true, true, false]);
    }

    #[test]
    fn an_event_fully_inside_another_counts() {
        let events = vec![
            timed("long", (9, 0), (12, 0)),
            timed("short", (10, 0), (10, 15)),
        ];
        assert_eq!(mark_overlaps(&events), vec![true, true]);
    }

    #[test]
    fn all_day_events_never_conflict() {
        let mut all_day = timed("all day", (0, 0), (23, 59));
        all_day.all_day = true;
        all_day.start = None;
        all_day.end = None;
        let events = vec![all_day, timed("meeting", (9, 0), (10, 0))];
        assert_eq!(mark_overlaps(&events), vec![false, false]);
    }

    #[test]
    fn tasks_sort_oldest_due_first() {
        let mut t = vec![
            task("b today", Some("2026-08-04")),
            task("a today", Some("2026-08-04")),
            task("old", Some("2026-07-01")),
        ];
        sort_tasks(&mut t);
        let titles: Vec<_> = t.iter().map(|x| x.title.as_str()).collect();
        assert_eq!(titles, vec!["old", "a today", "b today"]);
    }
}
