//! Google Calendar API v3 — nur die Termine des laufenden Tages.

use super::auth::Auth;
use super::{Result, agent, api_error, urlencode};
use crate::model::Event;
use chrono::{DateTime, Local};
use serde::Deserialize;

const BASE: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Debug, Clone)]
pub struct CalendarRef {
    pub id: String,
    pub name: String,
    pub color: u32,
}

#[derive(Debug, Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarListEntry {
    id: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    background_color: Option<String>,
    /// Im Google-Web-UI abgewaehlte Kalender wollen wir auch hier nicht sehen.
    #[serde(default)]
    selected: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsResponse {
    #[serde(default)]
    items: Vec<EventEntry>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventEntry {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    html_link: Option<String>,
    #[serde(default)]
    start: Option<EventTime>,
    #[serde(default)]
    end: Option<EventTime>,
    #[serde(default)]
    attendees: Vec<Attendee>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventTime {
    /// Zeitgebundener Termin, RFC 3339 mit Offset.
    #[serde(default)]
    date_time: Option<String>,
    /// Ganztaegig: reines Datum, **keine** Zeitzonenkonvertierung.
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attendee {
    #[serde(rename = "self", default)]
    is_self: bool,
    #[serde(default)]
    response_status: Option<String>,
}

/// Alle lesbaren, im Web-UI aktivierten Kalender des Kontos.
pub fn list_calendars(auth: &mut Auth) -> Result<Vec<CalendarRef>> {
    let url = format!(
        "{BASE}/users/me/calendarList?minAccessRole=reader&maxResults=250\
         &fields=items(id,summary,backgroundColor,selected)"
    );
    let body = get(auth, &url)?;
    let parsed: CalendarListResponse = serde_json::from_str(&body)?;

    Ok(parsed
        .items
        .into_iter()
        .filter(|c| c.selected.unwrap_or(true))
        .map(|c| CalendarRef {
            color: parse_hex_color(c.background_color.as_deref()).unwrap_or(0x4C_8DF6),
            name: if c.summary.is_empty() {
                c.id.clone()
            } else {
                c.summary
            },
            id: c.id,
        })
        .collect())
}

/// Termine eines Kalenders im Fenster `[from, to)`.
///
/// `singleEvents=true` ist entscheidend: ohne das liefert die API bei
/// Serienterminen die Wiederholungsregel statt der konkreten Instanz von heute.
pub fn list_events(
    auth: &mut Auth,
    cal: &CalendarRef,
    from: DateTime<Local>,
    to: DateTime<Local>,
    hide_declined: bool,
) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let mut url = format!(
            "{BASE}/calendars/{}/events?singleEvents=true&orderBy=startTime\
             &timeMin={}&timeMax={}&maxResults=250\
             &fields=nextPageToken,items(summary,status,location,htmlLink,start,end,attendees(self,responseStatus))",
            urlencode(&cal.id),
            urlencode(&from.to_rfc3339()),
            urlencode(&to.to_rfc3339()),
        );
        if let Some(token) = &page_token {
            url.push_str(&format!("&pageToken={}", urlencode(token)));
        }

        let body = get(auth, &url)?;
        let parsed: EventsResponse = serde_json::from_str(&body)?;

        for item in parsed.items {
            if item.status.as_deref() == Some("cancelled") {
                continue;
            }
            if hide_declined
                && item
                    .attendees
                    .iter()
                    .any(|a| a.is_self && a.response_status.as_deref() == Some("declined"))
            {
                continue;
            }

            let (start, all_day) = convert_time(item.start.as_ref());
            let (end, _) = convert_time(item.end.as_ref());

            events.push(Event {
                title: item
                    .summary
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "(ohne Titel)".into()),
                start,
                end,
                all_day,
                location: item.location.filter(|s| !s.trim().is_empty()),
                html_link: item.html_link,
                color: cal.color,
                calendar_name: cal.name.clone(),
            });
        }

        match parsed.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }

    Ok(events)
}

/// `dateTime` -> lokale Zeit; `date` -> Ganztagestermin ohne Zeitpunkt.
fn convert_time(t: Option<&EventTime>) -> (Option<DateTime<Local>>, bool) {
    let Some(t) = t else {
        return (None, false);
    };
    if let Some(dt) = &t.date_time {
        let parsed = DateTime::parse_from_rfc3339(dt)
            .ok()
            .map(|d| d.with_timezone(&Local));
        (parsed, false)
    } else {
        (None, t.date.is_some())
    }
}

/// `"#a4bdfc"` -> `0x00A4BDFC`.
fn parse_hex_color(s: Option<&str>) -> Option<u32> {
    let s = s?.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok()
}

fn get(auth: &mut Auth, url: &str) -> Result<String> {
    let token = auth.access_token()?;
    let resp = agent()
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .call()?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string()?;
    if !(200..300).contains(&status) {
        return Err(api_error(status, &text));
    }
    Ok(text)
}
