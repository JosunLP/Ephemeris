// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Preview mode with sample data.
//!
//! Active when the `EPHEMERIS_DEMO` environment variable is set. Intended for
//! looking at and adjusting the layout — size, scaling, opacity — before any
//! calendar account is connected. Nothing is synchronised in this mode.

use crate::model::{Agenda, Event, Task};
use chrono::{Duration, Local};

pub fn enabled() -> bool {
    std::env::var_os("EPHEMERIS_DEMO").is_some()
}

pub fn agenda() -> Agenda {
    let now = Local::now();
    let today = now.date_naive();
    let at = |mins: i64| now + Duration::minutes(mins);

    let event = |title: &str, from: i64, to: i64, color: u32, loc: Option<&str>| Event {
        title: title.into(),
        start: Some(at(from)),
        end: Some(at(to)),
        all_day: false,
        days: None,
        location: loc.map(str::to_owned),
        html_link: None,
        join_url: None,
        color,
        calendar_name: "Beispiel".into(),
        calendar_id: "demo".into(),
        account_id: "demo".into(),
        task_id: None,
        task_list_id: None,
    };

    // Tomorrow at a fixed time, not relative to now: otherwise the entries
    // would land in today depending on when the widget was started.
    let tomorrow_at = |h: u32, m: u32| {
        crate::model::local_day_start(today + Duration::days(1))
            + Duration::hours(h as i64)
            + Duration::minutes(m as i64)
    };
    let next_day =
        |title: &str, from: (u32, u32), to: (u32, u32), color: u32, loc: Option<&str>| Event {
            title: title.into(),
            start: Some(tomorrow_at(from.0, from.1)),
            end: Some(tomorrow_at(to.0, to.1)),
            all_day: false,
            days: None,
            location: loc.map(str::to_owned),
            html_link: None,
            join_url: None,
            color,
            calendar_name: "Beispiel".into(),
            calendar_id: "demo".into(),
            account_id: "demo".into(),
            task_id: None,
            task_list_id: None,
        };

    let task = |title: &str, due_offset: i64, depth: u8| Task {
        id: title.into(),
        tasklist_id: "demo".into(),
        title: title.into(),
        due: Some(today - Duration::days(due_offset)),
        notes: None,
        depth,
        tasklist_name: "Beispiel".into(),
        account_id: "demo".into(),
        completing: false,
    };

    let mut agenda = Agenda {
        day: Some(today),
        events: vec![
            Event {
                title: "Betriebsversammlung".into(),
                start: None,
                end: None,
                all_day: true,
                days: None,
                location: None,
                html_link: None,
                join_url: None,
                color: 0x9B_8AFB,
                calendar_name: "Firma".into(),
                calendar_id: "demo-company".into(),
                account_id: "demo".into(),
                task_id: None,
                task_list_id: None,
            },
            // Already over, so drawn dimmed.
            event("Daily Standup", -260, -245, 0x4C_8DF6, None),
            // Deliberately too long for the row: shows truncation and the tooltip.
            event(
                "Abstimmung Rollout Zahlungsmodul mit Fachbereich und externem Dienstleister",
                -120,
                -75,
                0x4C_8DF6,
                None,
            ),
            // Running right now, so it lands in the hero card.
            event("Sprint Review", -12, 33, 0x33_B679, Some("Raum Nord")),
            event("1:1 mit Anna", 88, 118, 0xF6_BF26, None),
            // Overlaps the one-to-one, which shows the conflict detection.
            event("Deployment-Fenster Produktion", 100, 175, 0xE6_7C73, None),
            // A task with a time block: the same item as the task of the same
            // name below, so ticking that one off takes this row with it.
            event("Rechnung 2041 freigeben", 45, 75, 0x33_B679, None),
        ],
        tomorrow: vec![
            next_day(
                "Kick-off Projekt Nordwind",
                (9, 0),
                (10, 30),
                0x4C_8DF6,
                Some("Raum Süd"),
            ),
            next_day("Retrospektive", (16, 0), (17, 0), 0x33_B679, None),
        ],
        tasks: vec![
            task("Angebot Meyer GmbH nachfassen", 3, 0),
            task("Rechnung 2041 freigeben", 1, 0),
            task("Sprint-Review vorbereiten", 0, 0),
            task("Folien bis Kapitel 4 durchgehen", 0, 1),
            task("Backup-Protokoll verifizieren", 0, 0),
            task("Onboarding-Dokument aktualisieren", 0, 0),
        ],
        week: Vec::new(),
        fetched_at: Some(now - Duration::minutes(4)),
        last_error: None,
    };

    // The week view: a school timetable on a calendar of its own, which is the
    // case the view was asked for, with the day's own entries on top of it.
    // Both weeks the grid can show: on a weekend the five-day view has moved
    // on to the coming one, and an empty preview would show nothing at all.
    let this_week = crate::model::week_start(today, 7);
    let work_week = crate::model::week_start(today, 5);
    agenda.week = timetable(this_week);
    if work_week != this_week {
        agenda.week.extend(timetable(work_week));
    }
    agenda.week.extend(
        agenda
            .events
            .iter()
            .chain(agenda.tomorrow.iter())
            .filter(|e| !e.all_day)
            .cloned(),
    );
    crate::model::sort_events(&mut agenda.week);

    // The same pass the sync thread makes, so preview mode shows the pairing
    // rather than a case that cannot occur.
    let tasks = agenda.tasks.clone();
    crate::model::link_task_time_blocks(&mut agenda.events, &tasks);
    crate::model::link_task_time_blocks(&mut agenda.tomorrow, &tasks);
    agenda
}

/// A week of lessons from `monday` on, Monday to Friday, plus two entries from
/// other calendars: an all-day one across two days, and a meeting that
/// overlaps a lesson so the grid has to put them side by side.
fn timetable(monday: chrono::NaiveDate) -> Vec<Event> {
    use crate::model::local_day_start;
    let at = |day: i64, h: i64, m: i64| {
        local_day_start(monday + Duration::days(day)) + Duration::hours(h) + Duration::minutes(m)
    };
    let entry = |title: &str, room: Option<&str>, day: i64, from: (i64, i64), to: (i64, i64)| {
        let (calendar_name, calendar_id, color) = if room.is_some() {
            ("Stundenplan", "demo-timetable", 0x33_B679)
        } else {
            ("Schule", "demo-school", 0xE6_7C73)
        };
        Event {
            title: title.into(),
            start: Some(at(day, from.0, from.1)),
            end: Some(at(day, to.0, to.1)),
            all_day: false,
            days: None,
            location: room.map(str::to_owned),
            html_link: None,
            join_url: None,
            color,
            calendar_name: calendar_name.into(),
            calendar_id: calendar_id.into(),
            account_id: "demo".into(),
            task_id: None,
            task_list_id: None,
        }
    };

    // Periods of 45 minutes with the breaks a German school day has.
    const PERIODS: [((i64, i64), (i64, i64)); 6] = [
        ((8, 0), (8, 45)),
        ((8, 50), (9, 35)),
        ((9, 55), (10, 40)),
        ((10, 45), (11, 30)),
        ((11, 50), (12, 35)),
        ((12, 40), (13, 25)),
    ];
    const PLAN: [[Option<(&str, &str)>; 6]; 5] = [
        [
            Some(("Mathe 7b", "R 112")),
            Some(("Mathe 7b", "R 112")),
            Some(("Physik 9a", "PH 2")),
            None,
            Some(("Mathe 10c", "R 204")),
            None,
        ],
        [
            None,
            Some(("Physik 9a", "PH 2")),
            Some(("Mathe 10c", "R 204")),
            Some(("Mathe 10c", "R 204")),
            Some(("AG Robotik", "PH 1")),
            Some(("AG Robotik", "PH 1")),
        ],
        [
            Some(("Mathe 7b", "R 112")),
            Some(("Physik 11", "PH 2")),
            Some(("Physik 11", "PH 2")),
            Some(("Mathe 7b", "R 112")),
            None,
            None,
        ],
        [
            Some(("Mathe 10c", "R 204")),
            None,
            Some(("Physik 9a", "PH 2")),
            Some(("Mathe 7b", "R 112")),
            Some(("Physik 11", "PH 2")),
            None,
        ],
        [
            Some(("Physik 11", "PH 2")),
            Some(("Physik 11", "PH 2")),
            None,
            Some(("Mathe 10c", "R 204")),
            Some(("Mathe 7b", "R 112")),
            None,
        ],
    ];

    let mut week = Vec::new();
    for (day, lessons) in PLAN.iter().enumerate() {
        for (period, lesson) in lessons.iter().enumerate() {
            if let Some((title, room)) = lesson {
                let (from, to) = PERIODS[period];
                week.push(entry(title, Some(room), day as i64, from, to));
            }
        }
    }
    week.push(entry(
        "Fachkonferenz Mathematik",
        None,
        1,
        (14, 0),
        (15, 30),
    ));
    week.push(entry("Elterngespräch Müller", None, 1, (15, 0), (15, 45)));
    week.push(Event {
        all_day: true,
        start: None,
        end: None,
        days: Some((monday + Duration::days(2), monday + Duration::days(4))),
        ..entry("Klassenfahrt 10c", None, 2, (0, 0), (0, 0))
    });
    week
}

#[cfg(test)]
mod tests {
    /// The preview is where a task with a time block can actually be clicked,
    /// which is the only way to see the fade and the two rows leaving together
    /// without connecting an account. A rename of either half would quietly
    /// take that case away again.
    #[test]
    fn the_preview_contains_a_task_with_a_time_block() {
        let agenda = super::agenda();
        let paired: Vec<&str> = agenda
            .events
            .iter()
            .filter(|e| e.task_id.is_some())
            .map(|e| e.title.as_str())
            .collect();
        assert_eq!(paired, vec!["Rechnung 2041 freigeben"]);
    }
}
