//! Vorschaumodus mit Beispieldaten.
//!
//! Aktiv, wenn die Umgebungsvariable `TPMPLANER_DEMO` gesetzt ist. Gedacht
//! zum Ansehen und Einrichten des Layouts (Groesse, Skalierung, Deckkraft),
//! bevor der Google-Zugang steht. Es wird dabei nichts synchronisiert.

use crate::model::{Agenda, Event, Task};
use chrono::{Duration, Local};

pub fn enabled() -> bool {
    std::env::var_os("TPMPLANER_DEMO").is_some()
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
        color,
        calendar_name: "Beispiel".into(),
    };

    // Morgen zu einer festen Uhrzeit — nicht relativ zu jetzt, sonst landen
    // die Eintraege je nach Startzeit des Widgets noch im heutigen Tag.
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
            color,
            calendar_name: "Beispiel".into(),
        };

    let task = |title: &str, due_offset: i64, depth: u8| Task {
        id: title.into(),
        tasklist_id: "demo".into(),
        title: title.into(),
        due: Some(today - Duration::days(due_offset)),
        notes: None,
        depth,
        tasklist_name: "Beispiel".into(),
        completing: false,
    };

    Agenda {
        day: Some(today),
        events: vec![
            Event {
                title: "Betriebsversammlung".into(),
                start: None,
                end: None,
                all_day: true,
                location: None,
                html_link: None,
                color: 0x9B_8AFB,
                calendar_name: "Firma".into(),
            },
            // Bereits vorbei — wird gedimmt gezeichnet.
            event("Daily Standup", -260, -245, 0x4C_8DF6, None),
            // Bewusst zu lang fuer die Zeile: zeigt Kuerzung und Tooltip.
            event(
                "Abstimmung Rollout Zahlungsmodul mit Fachbereich und externem Dienstleister",
                -120,
                -75,
                0x4C_8DF6,
                None,
            ),
            // Laeuft gerade — landet in der Hero-Karte.
            event("Sprint Review", -12, 33, 0x33_B679, Some("Raum Nord")),
            event("1:1 mit Anna", 88, 118, 0xF6_BF26, None),
            // Ueberschneidung mit dem 1:1 — zeigt die Konflikterkennung.
            event("Deployment-Fenster Produktion", 100, 175, 0xE6_7C73, None),
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
    }
}
