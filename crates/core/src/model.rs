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
    /// Which account this came from, the same stamp [`Task::account_id`]
    /// carries. A title identifies an entry within one account and nowhere
    /// else, so [`link_task_time_blocks`] needs it.
    #[serde(default)]
    pub account_id: String,
    /// The task this entry is the time block of, if one could be identified.
    ///
    /// Derived locally; see [`link_task_time_blocks`] for why no provider
    /// hands it over.
    #[serde(default)]
    pub task_id: Option<String>,
    /// The list [`Event::task_id`] was read from. Set with it and empty
    /// without it.
    ///
    /// Together with [`Event::account_id`] it completes the [`TaskKey`] the
    /// link stands for: an id on its own belongs to whichever provider issued
    /// it, and a second list may be using the same one. A cache written before
    /// this existed loads without it, and the entry simply goes unmatched
    /// until the next sync links it again.
    #[serde(default)]
    pub task_list_id: Option<String>,
}

impl Event {
    /// The task this entry is the time block of, as an identity rather than a
    /// bare id. `None` when nothing was linked, and when a cache written
    /// before [`Event::task_list_id`] existed left half a link behind.
    pub fn task_key(&self) -> Option<TaskKey> {
        let id = self.task_id.as_deref()?;
        let list = self.task_list_id.as_deref()?;
        Some(TaskKey::new(&self.account_id, list, id))
    }

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

/// Which task a hit region belongs to.
///
/// A position in [`Agenda::tasks`] would be smaller still and is what this
/// replaces, because a position is not an identity: the rectangles a click is
/// tested against were measured while painting, and the sync thread can put a
/// different list in place before the click arrives. `get(idx)` then answers
/// perfectly happily — with another task, which the widget would tick off on
/// the user's behalf.
///
/// A hash rather than the id itself so that [`crate::layout::Hit`] stays
/// `Copy`: every frame compares one against what is hovered, and a `String` in
/// there would put an allocation and a lifetime through all of that for no
/// gain. The account and the list are hashed alongside the task id because
/// each provider hands out ids in its own namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskKey(u64);

impl TaskKey {
    /// The identity of the task these three ids name.
    ///
    /// For the completion path, which carries the ids around rather than the
    /// task itself and has to arrive at the same key [`Task::key`] produces.
    pub fn new(account_id: &str, tasklist_id: &str, task_id: &str) -> TaskKey {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        account_id.hash(&mut hasher);
        tasklist_id.hash(&mut hasher);
        task_id.hash(&mut hasher);
        TaskKey(hasher.finish())
    }
}

impl Task {
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        matches!(self.due, Some(d) if d < today)
    }

    /// This task's identity, for a hit region to carry.
    pub fn key(&self) -> TaskKey {
        TaskKey::new(&self.account_id, &self.tasklist_id, &self.id)
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

impl Agenda {
    /// Is this calendar entry the time block of a task that has just been
    /// ticked off?
    ///
    /// The tick is acknowledged on the task row at once and only sent when the
    /// undo window closes, so for those few seconds the entry under *Schedule*
    /// has to say the same thing the entry under *Tasks* does. Afterwards
    /// [`Agenda::complete_task`] takes both rows away together.
    pub fn is_completing(&self, event: &Event) -> bool {
        let Some(key) = event.task_key() else {
            return false;
        };
        self.tasks.iter().any(|t| t.completing && t.key() == key)
    }

    /// The task a hit region names, or `None` when it is no longer on the list
    /// — a sync that arrived between the frame and the click, or the same task
    /// ticked off twice.
    pub fn task_mut(&mut self, key: TaskKey) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.key() == key)
    }

    /// Takes a completed task off the display, together with the calendar
    /// entries that were its time block.
    ///
    /// Returns those entries, which is what the sync thread needs to keep the
    /// provider from putting them straight back on the next run.
    ///
    /// Taken by [`TaskKey`], because a bare task id is not an identity: each
    /// provider hands out ids in its own namespace, and a second account — or
    /// a second list within one account — may well be using the same one for a
    /// different task. Removing by id alone takes those away too.
    ///
    /// The entries are matched on the same key, which is what
    /// [`link_task_time_blocks`] left on them.
    pub fn complete_task(&mut self, key: TaskKey) -> Vec<Event> {
        self.tasks.retain(|t| t.key() != key);
        let is_block = |e: &Event| e.task_key() == Some(key);
        let removed: Vec<Event> = self
            .events
            .iter()
            .chain(self.tomorrow.iter())
            .filter(|e| is_block(e))
            .cloned()
            .collect();
        self.events.retain(|e| !is_block(e));
        self.tomorrow.retain(|e| !is_block(e));
        removed
    }
}

/// Marks the calendar entries that are a task's time block.
///
/// A task with a time block leads a double life. Creating a task from the
/// Google Calendar interface offers two dates: a *start date and duration*,
/// which is a genuine block on the calendar, and a *deadline*, which is the due
/// date the task list works from. Both are fetched, through two different APIs,
/// and nothing in either answer says they belong together — the calendar side
/// has no task id, and the task side has no event id. So the widget drew two
/// unrelated rows, and ticking one of them left the other exactly where it was.
///
/// Nothing can be done about the missing identifier: as of the 2026 revision of
/// the Calendar API, `eventType` has six values and none of them is a task, so
/// there is no field to read. What is left is the title, which both halves
/// carry unchanged because they *are* one item to the service that made them.
/// Matching on it is a guess, so the guess is made as narrow as it can be:
///
/// * within one account only — two accounts sharing a title mean nothing;
/// * on the whole normalised title, never a prefix;
/// * and never when two visible tasks carry the same title, because then there
///   is no telling which of them the block belongs to, and hiding a row for the
///   wrong reason is worse than leaving it.
///
/// Any link a previous pass left behind is dropped first, so a task that was
/// completed or renamed does not keep an entry tied to it.
pub fn link_task_time_blocks(events: &mut [Event], tasks: &[Task]) {
    use std::collections::HashMap;

    // `None` marks a title that more than one task claims.
    let mut by_title: HashMap<(&str, String), Option<&Task>> = HashMap::new();
    for task in tasks {
        let key = title_key(&task.title);
        if key.is_empty() {
            continue;
        }
        by_title
            .entry((task.account_id.as_str(), key))
            .and_modify(|slot| *slot = None)
            .or_insert(Some(task));
    }

    for event in events {
        let key = title_key(&event.title);
        let matched = match by_title.get(&(event.account_id.as_str(), key)) {
            Some(Some(task)) => Some(*task),
            _ => None,
        };
        event.task_id = matched.map(|t| t.id.clone());
        event.task_list_id = matched.map(|t| t.tasklist_id.clone());
    }
}

/// The normalised form two titles are compared in.
///
/// Case and runs of whitespace differ between the calendar and the task copy of
/// the same item often enough — a wrapped title, a trailing space someone typed
/// — that comparing the raw strings would miss pairs that are obviously the
/// same. Nothing beyond that is folded away: punctuation and accents carry
/// meaning in a title.
pub fn title_key(title: &str) -> String {
    let mut key = String::with_capacity(title.len());
    for word in title.split_whitespace() {
        if !key.is_empty() {
            key.push(' ');
        }
        key.extend(word.chars().flat_map(char::to_lowercase));
    }
    key
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
///
/// The second half of the result is the number of clashing *pairs*, counted
/// here because this is the loop that already knows them. Dividing the number
/// of flagged events by two — which is what the badge used to do — under-counts
/// exactly the case most worth flagging: three meetings in the same slot are
/// three pairs, not one.
pub fn mark_overlaps(events: &[Event]) -> (Vec<bool>, usize) {
    let mut flags = vec![false; events.len()];
    let mut pairs = 0;
    for i in 0..events.len() {
        for j in (i + 1)..events.len() {
            if overlaps(&events[i], &events[j]) {
                flags[i] = true;
                flags[j] = true;
                pairs += 1;
            }
        }
    }
    (flags, pairs)
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

/// Today's agenda as plain text, for pasting into a message.
///
/// Here rather than in a front end because "copy agenda" is one of the menu
/// commands and all three offer it. What differs per platform is putting the
/// string on the clipboard, which is one call; what does not is the shape of
/// the text, and three copies of that would be three subtly different
/// agendas.
///
/// Deliberately plain: no colour, no box drawing and no alignment padding. It
/// is pasted into a chat window or an email, where a proportional font makes
/// columns meaningless — and where `終日` counting as two characters and four
/// columns would misalign them anyway.
pub fn agenda_as_text(agenda: &Agenda, loc: &crate::i18n::Locale) -> String {
    let c = loc.cat;
    let today = agenda.day.unwrap_or_else(|| Local::now().date_naive());
    let mut out = format!("{} — {}\n", loc.weekday(today), loc.date_line(today));

    out.push_str(&format!("\n{}\n", c.section_events));
    if agenda.events.is_empty() {
        out.push_str(&format!("  {}\n", c.no_events));
    }
    for ev in &agenda.events {
        let when = if ev.all_day {
            c.all_day.to_string()
        } else {
            match (ev.start, ev.end) {
                (Some(s), Some(e)) => format!("{}-{}", loc.time(s), loc.time(e)),
                (Some(s), None) => loc.time(s),
                _ => String::new(),
            }
        };
        match &ev.location {
            Some(place) => out.push_str(&format!("  {when}  {}  ({place})\n", ev.title)),
            None => out.push_str(&format!("  {when}  {}\n", ev.title)),
        }
    }

    out.push_str(&format!("\n{}\n", c.section_tasks));
    if agenda.tasks.is_empty() {
        out.push_str(&format!("  {}\n", c.no_tasks));
    }
    for task in &agenda.tasks {
        let due = match task.due {
            Some(d) if d == today => c.today.to_string(),
            Some(d) => loc.day_month(d),
            None => "-".into(),
        };
        // Subtasks indented, as they are on screen.
        let indent = "  ".repeat(task.depth as usize + 1);
        out.push_str(&format!("{indent}[ ] {due}  {}\n", task.title));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The copied text is the one thing a user takes out of the widget and
    /// into somewhere else, so an empty day has to read as an empty day rather
    /// than as a broken export.
    #[test]
    fn the_copied_agenda_names_both_sections_even_when_empty() {
        let loc = crate::i18n::Locale::resolve("en-GB");
        let text = agenda_as_text(&Agenda::default(), &loc);
        assert!(text.contains(loc.cat.section_events), "{text}");
        assert!(text.contains(loc.cat.section_tasks), "{text}");
        assert!(text.contains(loc.cat.no_events), "{text}");
        assert!(text.contains(loc.cat.no_tasks), "{text}");
        // No placeholder may survive into text somebody pastes.
        assert!(!text.contains("{}"), "{text}");
    }

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
            account_id: String::new(),
            task_id: None,
            task_list_id: None,
        }
    }

    #[test]
    fn back_to_back_events_are_not_a_conflict() {
        // 09:00-10:00 and 10:00-11:00 merely touch.
        let events = vec![timed("a", (9, 0), (10, 0)), timed("b", (10, 0), (11, 0))];
        assert_eq!(mark_overlaps(&events).0, vec![false, false]);
    }

    #[test]
    fn genuine_double_bookings_are_flagged() {
        let events = vec![
            timed("a", (9, 0), (10, 0)),
            timed("b", (9, 30), (10, 30)),
            timed("c", (14, 0), (15, 0)),
        ];
        assert_eq!(mark_overlaps(&events).0, vec![true, true, false]);
    }

    #[test]
    fn an_event_fully_inside_another_counts() {
        let events = vec![
            timed("long", (9, 0), (12, 0)),
            timed("short", (10, 0), (10, 15)),
        ];
        assert_eq!(mark_overlaps(&events).0, vec![true, true]);
    }

    #[test]
    fn all_day_events_never_conflict() {
        let mut all_day = timed("all day", (0, 0), (23, 59));
        all_day.all_day = true;
        all_day.start = None;
        all_day.end = None;
        let events = vec![all_day, timed("meeting", (9, 0), (10, 0))];
        assert_eq!(mark_overlaps(&events).0, vec![false, false]);
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

    /// The reported case: one item, two rows, and a tick that only reached one
    /// of them.
    fn on_account(id: &str, mut event: Event) -> Event {
        event.account_id = id.into();
        event
    }

    fn task_on(account: &str, title: &str) -> Task {
        let mut t = task(title, Some("2026-08-04"));
        t.account_id = account.into();
        t
    }

    #[test]
    fn a_time_block_is_tied_to_the_task_of_the_same_name() {
        let mut events = vec![
            on_account(
                "google",
                timed("Write the quarterly report", (9, 0), (11, 0)),
            ),
            on_account("google", timed("Dentist", (14, 0), (15, 0))),
        ];
        let tasks = vec![
            task_on("google", "Write the quarterly report"),
            task_on("google", "Order printer paper"),
        ];
        link_task_time_blocks(&mut events, &tasks);

        assert_eq!(
            events[0].task_id.as_deref(),
            Some("Write the quarterly report"),
            "the block and its task were not paired up"
        );
        assert_eq!(events[1].task_id, None, "the dentist is not a task");
    }

    #[test]
    fn case_and_spacing_do_not_break_the_pairing() {
        let mut events = vec![on_account(
            "google",
            timed("  Write   the Quarterly Report ", (9, 0), (11, 0)),
        )];
        let tasks = vec![task_on("google", "Write the quarterly report")];
        link_task_time_blocks(&mut events, &tasks);
        assert!(events[0].task_id.is_some());
    }

    #[test]
    fn a_title_shared_by_two_tasks_pairs_with_neither() {
        // Two "Follow up" tasks, one block. There is no telling which of them
        // it belongs to, and taking the row away for the wrong one is worse
        // than leaving it.
        let mut events = vec![on_account("google", timed("Follow up", (9, 0), (10, 0)))];
        let mut second = task_on("google", "Follow up");
        second.id = "other-id".into();
        let tasks = vec![task_on("google", "Follow up"), second];
        link_task_time_blocks(&mut events, &tasks);
        assert_eq!(events[0].task_id, None);
    }

    #[test]
    fn a_title_shared_across_accounts_is_not_a_pair() {
        // The work calendar's "Standup" has nothing to do with the private
        // account's task of the same name.
        let mut events = vec![on_account("work", timed("Standup", (9, 0), (9, 15)))];
        let tasks = vec![task_on("private", "Standup")];
        link_task_time_blocks(&mut events, &tasks);
        assert_eq!(events[0].task_id, None);
    }

    #[test]
    fn a_stale_link_is_dropped_rather_than_kept() {
        let mut events = vec![on_account(
            "google",
            timed("Renamed since", (9, 0), (10, 0)),
        )];
        events[0].task_id = Some("gone".into());
        events[0].task_list_id = Some("l".into());
        link_task_time_blocks(&mut events, &[]);
        assert_eq!(events[0].task_id, None);
        assert_eq!(events[0].task_list_id, None, "half a link is still a link");
    }

    #[test]
    fn ticking_the_task_off_marks_the_time_block_too() {
        let mut agenda = Agenda {
            events: vec![on_account("google", timed("Tax return", (9, 0), (11, 0)))],
            tomorrow: vec![on_account("google", timed("Tax return", (9, 0), (11, 0)))],
            tasks: vec![task_on("google", "Tax return")],
            ..Default::default()
        };
        link_task_time_blocks(&mut agenda.events, &agenda.tasks.clone());
        link_task_time_blocks(&mut agenda.tomorrow, &agenda.tasks.clone());

        assert!(
            !agenda.is_completing(&agenda.events[0]),
            "nothing has been ticked yet"
        );

        // The optimistic hide, before the call goes out.
        agenda.tasks[0].completing = true;
        assert!(agenda.is_completing(&agenda.events[0]));

        // And once the server confirms, both rows go.
        let removed = agenda.complete_task(agenda.tasks[0].key());
        let taken: Vec<(&str, &str)> = removed
            .iter()
            .map(|e| (e.account_id.as_str(), e.title.as_str()))
            .collect();
        assert_eq!(
            taken,
            vec![("google", "Tax return"), ("google", "Tax return")],
            "the caller needs the account, not only the title"
        );
        assert!(agenda.tasks.is_empty());
        assert!(agenda.events.is_empty(), "the schedule row stayed behind");
        assert!(agenda.tomorrow.is_empty());
    }

    /// The click that ticks a task off is tested against rectangles measured
    /// while painting, and the sync thread can put a different list in place in
    /// between. A row number would then name whatever moved into that place —
    /// so the hit region carries the task instead.
    #[test]
    fn a_ticked_task_is_found_by_its_own_identity_and_not_by_its_position() {
        // The list the row was painted from: the second task was clicked.
        let painted = [
            task_on("google", "Renew the passport"),
            task_on("google", "Book the flight"),
        ];
        let key = painted[1].key();

        // What the sync thread put there instead: the first task gone, so the
        // clicked row is now at index 0.
        let mut agenda = Agenda {
            tasks: vec![task_on("google", "Book the flight")],
            ..Default::default()
        };
        assert_eq!(
            agenda.task_mut(key).map(|t| t.title.clone()).as_deref(),
            Some("Book the flight")
        );

        // The same title on another account is a different task.
        let mut elsewhere = Agenda {
            tasks: vec![task_on("private", "Book the flight")],
            ..Default::default()
        };
        assert!(elsewhere.task_mut(key).is_none());

        // And a task that has left the list at all is nobody, rather than
        // whoever took its place.
        let mut gone = Agenda {
            tasks: vec![task_on("google", "Renew the passport")],
            ..Default::default()
        };
        assert!(gone.task_mut(key).is_none());
    }

    #[test]
    fn completing_a_task_without_a_block_leaves_the_schedule_alone() {
        let mut agenda = Agenda {
            events: vec![on_account("google", timed("Dentist", (14, 0), (15, 0)))],
            tasks: vec![task_on("google", "Order printer paper")],
            ..Default::default()
        };
        link_task_time_blocks(&mut agenda.events, &agenda.tasks.clone());
        assert!(agenda.complete_task(agenda.tasks[0].key()).is_empty());
        assert_eq!(agenda.events.len(), 1);
    }

    /// Ids are namespaced by whoever handed them out, so the same string turns
    /// up again on another account and in another list of the same account.
    /// Ticking one of them off must take that one away and nothing else.
    #[test]
    fn a_shared_task_id_elsewhere_survives_the_completion() {
        let same_id = |account: &str, list: &str, title: &str| {
            let mut t = task_on(account, title);
            t.id = "1".into();
            t.tasklist_id = list.into();
            t
        };
        let ticked = same_id("work", "inbox", "Send the invoice");
        let other_account = same_id("private", "inbox", "Water the plants");
        let other_list = same_id("work", "someday", "Learn to sail");

        let mut agenda = Agenda {
            events: vec![
                on_account("work", timed("Send the invoice", (9, 0), (10, 0))),
                on_account("private", timed("Water the plants", (9, 0), (10, 0))),
                on_account("work", timed("Learn to sail", (11, 0), (12, 0))),
            ],
            tomorrow: vec![on_account(
                "private",
                timed("Water the plants", (8, 0), (8, 30)),
            )],
            tasks: vec![ticked.clone(), other_account, other_list],
            ..Default::default()
        };
        // The links the sync run leaves behind: every entry points at "1",
        // which is three different tasks.
        link_task_time_blocks(&mut agenda.events, &agenda.tasks.clone());
        link_task_time_blocks(&mut agenda.tomorrow, &agenda.tasks.clone());
        assert!(agenda.events.iter().all(|e| e.task_id.is_some()));

        // The optimistic hide belongs to the ticked task alone as well.
        agenda.tasks[0].completing = true;
        assert!(agenda.is_completing(&agenda.events[0]));
        assert!(
            !agenda.is_completing(&agenda.events[1]),
            "the other account's entry is not the one being ticked off"
        );

        let removed = agenda.complete_task(ticked.key());
        let taken: Vec<&str> = removed.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(taken, vec!["Send the invoice"]);

        let left: Vec<&str> = agenda.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(left, vec!["Water the plants", "Learn to sail"]);
        let schedule: Vec<&str> = agenda.events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(schedule, vec!["Water the plants", "Learn to sail"]);
        assert_eq!(agenda.tomorrow.len(), 1, "tomorrow belongs to another task");
    }
}
