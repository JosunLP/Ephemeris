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
        fetched_at: Some(now - Duration::minutes(4)),
        last_error: None,
    };

    // The same pass the sync thread makes, so preview mode shows the pairing
    // rather than a case that cannot occur.
    let tasks = agenda.tasks.clone();
    crate::model::link_task_time_blocks(&mut agenda.events, &tasks);
    crate::model::link_task_time_blocks(&mut agenda.tomorrow, &tasks);
    agenda
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
