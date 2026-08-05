//! Domaenenmodell des Widgets: das, was tatsaechlich gezeichnet wird.
//!
//! Bewusst entkoppelt von den Google-JSON-Strukturen, damit der Renderer
//! nichts ueber die API weiss und die Filter-/Sortierlogik testbar bleibt.

use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

/// Ein Kalendereintrag des heutigen Tages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub title: String,
    /// `None` bei Ganztagesterminen.
    pub start: Option<DateTime<Local>>,
    pub end: Option<DateTime<Local>>,
    pub all_day: bool,
    pub location: Option<String>,
    /// Link in den Google-Kalender, wird beim Klick geoeffnet.
    pub html_link: Option<String>,
    /// Farbe des Quellkalenders als 0xRRGGBB.
    pub color: u32,
    pub calendar_name: String,
}

impl Event {
    /// Laeuft der Termin gerade?
    pub fn is_now(&self, now: DateTime<Local>) -> bool {
        match (self.start, self.end) {
            (Some(s), Some(e)) => s <= now && now < e,
            (Some(s), None) => s <= now && now < s + chrono::Duration::hours(1),
            _ => false,
        }
    }

    /// Ist der Termin bereits vorbei? Ganztagestermine nie.
    pub fn is_past(&self, now: DateTime<Local>) -> bool {
        if self.all_day {
            return false;
        }
        match self.end.or(self.start) {
            Some(e) => e <= now,
            None => false,
        }
    }

    /// Sortierschluessel: Ganztagestermine zuerst, danach nach Startzeit.
    fn sort_key(&self) -> (u8, i64) {
        if self.all_day {
            (0, 0)
        } else {
            (1, self.start.map(|s| s.timestamp()).unwrap_or(i64::MAX))
        }
    }
}

/// Eine Aufgabe aus Google Tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Fuer das Abhaken via API benoetigt.
    pub id: String,
    pub tasklist_id: String,
    pub title: String,
    /// Reines Kalenderdatum. Siehe [`parse_task_due`] fuer den Grund.
    pub due: Option<NaiveDate>,
    pub notes: Option<String>,
    /// Verschachtelungstiefe (Unteraufgaben werden eingerueckt).
    pub depth: u8,
    pub tasklist_name: String,
    /// Lokal gesetzt, solange das Abhaken noch zum Server unterwegs ist.
    /// Rein transient, gehoert nicht in den Cache.
    #[serde(skip)]
    pub completing: bool,
}

impl Task {
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        matches!(self.due, Some(d) if d < today)
    }
}

/// Der komplette Anzeigezustand eines Sync-Laufs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Agenda {
    pub day: Option<NaiveDate>,
    pub events: Vec<Event>,
    /// Termine von morgen. Ab dem spaeten Nachmittag ist die Liste fuer heute
    /// leer und das Widget waere sonst eine leere Flaeche — dabei ist genau
    /// dann die Frage "was kommt morgen als Erstes?" die interessante.
    pub tomorrow: Vec<Event>,
    pub tasks: Vec<Task>,
    pub fetched_at: Option<DateTime<Local>>,
    /// Fehlertext des letzten Versuchs; die alten Daten bleiben sichtbar.
    /// Wird nicht gecacht — beim Start gilt zunaechst "unbekannt".
    #[serde(skip)]
    pub last_error: Option<String>,
}

/// Google Tasks liefert `due` als RFC-3339-Zeitstempel, aber die Uhrzeit ist
/// **bedeutungslos** — die API normalisiert jedes Faelligkeitsdatum auf
/// `YYYY-MM-DDT00:00:00.000Z`. Wer den Wert als echten UTC-Zeitpunkt parst und
/// in die lokale Zone konvertiert, landet in jeder Zone oestlich von UTC einen
/// Tag zu frueh (in UTC-Zonen einen Tag zu spaet).
///
/// Deshalb: ausschliesslich die ersten zehn Zeichen auswerten, niemals
/// zeitzonenkonvertieren.
pub fn parse_task_due(raw: &str) -> Option<NaiveDate> {
    let date_part = raw.get(..10)?;
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
}

/// Behaelt Aufgaben, die **heute oder frueher** faellig sind.
///
/// Aufgaben ohne Faelligkeitsdatum fliegen raus (Konfiguration: `show_undated`),
/// ebenso alles, was erst in Zukunft faellig wird — das war die urspruengliche
/// Beschwerde: Google Tasks liefert per Default die komplette Liste.
pub fn filter_tasks_for_today(tasks: Vec<Task>, today: NaiveDate, show_undated: bool) -> Vec<Task> {
    tasks
        .into_iter()
        .filter(|t| match t.due {
            Some(d) => d <= today,
            None => show_undated,
        })
        .collect()
}

/// Sortierung: zuerst nach Faelligkeit (aelteste zuerst, undatiert ans Ende),
/// bei Gleichstand alphabetisch, damit die Reihenfolge zwischen zwei Syncs
/// stabil bleibt und nicht springt.
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

/// Markiert Termine, die sich zeitlich mit einem anderen ueberschneiden.
///
/// Doppelbuchungen sind beim Ueberfliegen einer Liste kaum zu erkennen — man
/// muesste Ende und Anfang zweier Zeilen im Kopf vergleichen. Das Ergebnis
/// steht deshalb in der Abschnittszeile und faerbt die betroffenen Uhrzeiten.
///
/// Ganztagestermine zaehlen nicht mit: sie ueberschneiden sich definitionsgemaess
/// mit allem und waeren als Warnung wertlos.
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
    // Ohne Endzeit gilt die Google-Vorgabe von einer Stunde, wie auch in
    // `Event::is_now`.
    let a_end = a.end.unwrap_or(a_start + chrono::Duration::hours(1));
    let b_end = b.end.unwrap_or(b_start + chrono::Duration::hours(1));
    // Beruehrung an der Grenze ist keine Ueberschneidung: 09:00–10:00 und
    // 10:00–11:00 sind zwei aufeinanderfolgende Termine, kein Konflikt.
    a_start < b_end && b_start < a_end
}

/// Beginn des lokalen Tages als `DateTime<Local>`.
///
/// An Tagen mit Zeitumstellung kann Mitternacht mehrdeutig oder nicht
/// existent sein; wir nehmen dann den fruehesten gueltigen Zeitpunkt bzw.
/// weichen auf 01:00 aus, statt zu panicken.
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
        // Genau der Fall, der bei naivem Parsen in UTC+2 auf den 3.8. rutscht.
        let d = parse_task_due("2026-08-04T00:00:00.000Z").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 8, 4).unwrap());
    }

    #[test]
    fn due_date_accepts_plain_date() {
        let d = parse_task_due("2026-08-04").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 8, 4).unwrap());
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
            completing: false,
        }
    }

    #[test]
    fn future_tasks_are_dropped_overdue_are_kept() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let input = vec![
            task("in drei Wochen", Some("2026-08-25")),
            task("heute", Some("2026-08-04")),
            task("ueberfaellig", Some("2026-07-30")),
            task("ohne Datum", None),
        ];
        let kept = filter_tasks_for_today(input, today, false);
        let titles: Vec<_> = kept.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["heute", "ueberfaellig"]);
    }

    #[test]
    fn undated_tasks_can_be_opted_in_and_sort_last() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let input = vec![task("ohne Datum", None), task("heute", Some("2026-08-04"))];
        let mut kept = filter_tasks_for_today(input, today, true);
        sort_tasks(&mut kept);
        let titles: Vec<_> = kept.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["heute", "ohne Datum"]);
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
            color: 0,
            calendar_name: "K".into(),
        }
    }

    #[test]
    fn back_to_back_events_are_not_a_conflict() {
        // 09:00-10:00 und 10:00-11:00 beruehren sich nur.
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
            timed("lang", (9, 0), (12, 0)),
            timed("kurz", (10, 0), (10, 15)),
        ];
        assert_eq!(mark_overlaps(&events), vec![true, true]);
    }

    #[test]
    fn all_day_events_never_conflict() {
        let mut all_day = timed("ganztags", (0, 0), (23, 59));
        all_day.all_day = true;
        all_day.start = None;
        all_day.end = None;
        let events = vec![all_day, timed("termin", (9, 0), (10, 0))];
        assert_eq!(mark_overlaps(&events), vec![false, false]);
    }

    #[test]
    fn tasks_sort_oldest_due_first() {
        let mut t = vec![
            task("b heute", Some("2026-08-04")),
            task("a heute", Some("2026-08-04")),
            task("alt", Some("2026-07-01")),
        ];
        sort_tasks(&mut t);
        let titles: Vec<_> = t.iter().map(|x| x.title.as_str()).collect();
        assert_eq!(titles, vec!["alt", "a heute", "b heute"]);
    }
}
