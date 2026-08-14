// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Microsoft Graph: Outlook calendars, Teams meetings and Microsoft To Do.
//!
//! There is no separate Teams calendar API. A Teams meeting is an ordinary
//! Outlook event that carries `isOnlineMeeting` and a join link, so supporting
//! Outlook supports Teams — the only extra work is surfacing that link, which
//! is what a desktop widget is actually wanted for during the day.
//!
//! Three details of this API differ from Google and are easy to get wrong:
//!
//! 1. **`calendarView`, not `events`.** `events` returns the recurrence rule
//!    for a recurring series; `calendarView` returns the concrete occurrence
//!    inside the requested window. This is the counterpart of Google's
//!    `singleEvents=true`.
//! 2. **Timestamps carry no offset.** Graph returns
//!    `{"dateTime": "2026-08-05T09:00:00.0000000", "timeZone": "UTC"}` — the
//!    string alone is ambiguous. The request therefore asks for UTC via the
//!    `Prefer` header and converts locally, instead of trusting a bare string.
//! 3. **To Do due dates are dates.** Like Google Tasks, `dueDateTime` is a
//!    calendar date dressed up as a timestamp; converting it across time zones
//!    moves it by a day.

use super::oauth::{ClientCredentials, Endpoints, Session, urlencode};
use super::{CalendarProvider, CalendarRef, Error, Result, TaskListRef, api_error, http};
use crate::model::{Event, Task, parse_task_due};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;
use std::path::PathBuf;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Public client: no secret. Microsoft rejects one for this application type.
const ENDPOINTS: Endpoints = Endpoints {
    auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
    token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
    // `offline_access` is what yields a refresh token at all.
    scopes: "offline_access Calendars.Read Tasks.ReadWrite",
    extra_auth_params: &[("prompt", "select_account")],
};

#[derive(Debug, Deserialize)]
struct ClientFile {
    client_id: String,
}

pub struct GraphProvider {
    account_id: String,
    display_name: String,
    session: Session,
}

impl GraphProvider {
    /// `client_file` holds the application (client) ID registered in Azure.
    pub fn new(
        account_id: &str,
        display_name: &str,
        client_file: &PathBuf,
        token_file: PathBuf,
    ) -> Result<Self> {
        let raw = std::fs::read_to_string(client_file).map_err(|_| {
            Error::NeedsSetup(format!(
                "microsoft_client.json is missing. Register an application in \
                 the Azure portal (public client, redirect URI \
                 http://localhost) and place the file here:\n{}",
                client_file.display()
            ))
        })?;
        let parsed: ClientFile = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|e| Error::NeedsSetup(format!("microsoft_client.json: {e}")))?;

        Ok(Self {
            account_id: account_id.to_string(),
            display_name: display_name.to_string(),
            session: Session::new(
                ENDPOINTS,
                ClientCredentials {
                    client_id: parsed.client_id,
                    client_secret: None,
                },
                token_file,
                format!("microsoft/{account_id}"),
                display_name,
            ),
        })
    }

    fn get(&mut self, url: &str) -> Result<String> {
        let token = self.session.access_token()?;
        let response = http()
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            // Without this the timestamps come back in the mailbox time zone
            // and the string carries no offset to disambiguate them.
            .header("Prefer", "outlook.timezone=\"UTC\"")
            .call()
            .map_err(|e| Error::Other(format!("Network error: {e}")))?;
        read_body(response)
    }

    fn patch(&mut self, url: &str, body: &str) -> Result<String> {
        let token = self.session.access_token()?;
        let response = http()
            .patch(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| Error::Other(format!("Network error: {e}")))?;
        read_body(response)
    }

    /// Walks `@odata.nextLink` until the collection is exhausted.
    fn get_all<T: for<'de> Deserialize<'de>>(&mut self, first_url: String) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut next = Some(first_url);
        while let Some(url) = next {
            let body = self.get(&url)?;
            let page: Page<T> = serde_json::from_str(&body)
                .map_err(|e| Error::Other(format!("Unexpected response: {e}")))?;
            out.extend(page.value);
            next = page.next_link;
        }
        Ok(out)
    }
}

fn read_body(response: ureq::http::Response<ureq::Body>) -> Result<String> {
    let status = response.status().as_u16();
    let text = response
        .into_body()
        .read_to_string()
        .map_err(|e| Error::Other(format!("Unreadable response: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(api_error(status, &text));
    }
    Ok(text)
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphCalendar {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    hex_color: Option<String>,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphEvent {
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    is_all_day: bool,
    #[serde(default)]
    is_cancelled: bool,
    #[serde(default)]
    start: Option<GraphTime>,
    #[serde(default)]
    end: Option<GraphTime>,
    #[serde(default)]
    location: Option<GraphLocation>,
    #[serde(default)]
    web_link: Option<String>,
    #[serde(default)]
    is_online_meeting: bool,
    #[serde(default)]
    online_meeting: Option<OnlineMeeting>,
    #[serde(default)]
    response_status: Option<ResponseStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphTime {
    #[serde(default)]
    date_time: Option<String>,
    #[serde(default)]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphLocation {
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnlineMeeting {
    #[serde(default)]
    join_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseStatus {
    #[serde(default)]
    response: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodoList {
    id: String,
    #[serde(default)]
    display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodoTask {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    due_date_time: Option<GraphTime>,
    #[serde(default)]
    body: Option<TodoBody>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodoBody {
    #[serde(default)]
    content: Option<String>,
}

impl CalendarProvider for GraphProvider {
    fn account_id(&self) -> &str {
        &self.account_id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn ensure_authorized(&mut self) -> Result<()> {
        if !self.session.has_refresh_token() {
            self.session.interactive_login()?;
        }
        Ok(())
    }

    fn forget(&mut self) {
        self.session.forget();
    }

    fn calendars(&mut self) -> Result<Vec<CalendarRef>> {
        let url = format!("{GRAPH}/me/calendars?$select=id,name,hexColor,color&$top=100");
        let raw: Vec<GraphCalendar> = self.get_all(url)?;
        Ok(raw
            .into_iter()
            .map(|c| CalendarRef {
                color: parse_hex(c.hex_color.as_deref())
                    .or_else(|| preset_color(c.color.as_deref()))
                    .unwrap_or(0x0F_6CBD),
                name: if c.name.is_empty() {
                    c.id.clone()
                } else {
                    c.name
                },
                id: c.id,
            })
            .collect())
    }

    fn events(
        &mut self,
        calendar: &CalendarRef,
        from: DateTime<Local>,
        to: DateTime<Local>,
        hide_declined: bool,
    ) -> Result<Vec<Event>> {
        let url = format!(
            "{GRAPH}/me/calendars/{}/calendarView\
             ?startDateTime={}&endDateTime={}&$top=250&$orderby=start/dateTime\
             &$select=subject,isAllDay,isCancelled,start,end,location,webLink,\
             isOnlineMeeting,onlineMeeting,responseStatus",
            urlencode(&calendar.id),
            urlencode(&from.with_timezone(&Utc).to_rfc3339()),
            urlencode(&to.with_timezone(&Utc).to_rfc3339()),
        );
        let raw: Vec<GraphEvent> = self.get_all(url)?;

        Ok(raw
            .into_iter()
            .filter(|e| !e.is_cancelled)
            .filter(|e| {
                !hide_declined
                    || e.response_status
                        .as_ref()
                        .and_then(|r| r.response.as_deref())
                        != Some("declined")
            })
            .map(|e| {
                let join_url = e
                    .online_meeting
                    .as_ref()
                    .and_then(|m| m.join_url.clone())
                    .filter(|_| e.is_online_meeting);
                Event {
                    title: e
                        .subject
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "(no subject)".into()),
                    start: if e.is_all_day {
                        None
                    } else {
                        convert(e.start.as_ref())
                    },
                    end: if e.is_all_day {
                        None
                    } else {
                        convert(e.end.as_ref())
                    },
                    all_day: e.is_all_day,
                    location: e
                        .location
                        .and_then(|l| l.display_name)
                        .filter(|s| !s.trim().is_empty()),
                    html_link: e.web_link,
                    join_url,
                    color: calendar.color,
                    calendar_name: calendar.name.clone(),
                    calendar_id: calendar.id.clone(),
                    account_id: String::new(),
                    task_id: None,
                    task_list_id: None,
                }
            })
            .collect())
    }

    fn task_lists(&mut self) -> Result<Vec<TaskListRef>> {
        let url = format!("{GRAPH}/me/todo/lists?$top=100");
        let raw: Vec<TodoList> = self.get_all(url)?;
        Ok(raw
            .into_iter()
            .map(|l| TaskListRef {
                name: if l.display_name.is_empty() {
                    "Tasks".into()
                } else {
                    l.display_name
                },
                id: l.id,
            })
            .collect())
    }

    fn tasks(
        &mut self,
        list: &TaskListRef,
        _today: NaiveDate,
        _include_undated: bool,
    ) -> Result<Vec<Task>> {
        // Graph cannot filter by due date server side the way Google Tasks
        // can, so everything open is fetched and `model::filter_tasks_for_today`
        // does the selection — the same filter every provider goes through.
        let url = format!(
            "{GRAPH}/me/todo/lists/{}/tasks?$top=100&$filter=status%20ne%20%27completed%27",
            urlencode(&list.id),
        );
        let raw: Vec<TodoTask> = self.get_all(url)?;

        Ok(raw
            .into_iter()
            .filter(|t| t.status.as_deref() != Some("completed"))
            .map(|t| Task {
                id: t.id,
                tasklist_id: list.id.clone(),
                title: if t.title.trim().is_empty() {
                    "(no title)".into()
                } else {
                    t.title
                },
                // Date only, exactly as in Google Tasks: taking the time part
                // seriously would shift the date by a day.
                due: t
                    .due_date_time
                    .and_then(|d| d.date_time)
                    .as_deref()
                    .and_then(parse_task_due),
                notes: t
                    .body
                    .and_then(|b| b.content)
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty()),
                // Microsoft To Do has checklist items, but no nested tasks.
                depth: 0,
                tasklist_name: list.name.clone(),
                account_id: String::new(),
                completing: false,
            })
            .collect())
    }

    fn complete_task(&mut self, list_id: &str, task_id: &str) -> Result<()> {
        let url = format!(
            "{GRAPH}/me/todo/lists/{}/tasks/{}",
            urlencode(list_id),
            urlencode(task_id)
        );
        self.patch(&url, r#"{"status":"completed"}"#)?;
        Ok(())
    }
}

/// Graph timestamps carry the zone in a sibling field, not in the string.
///
/// The request asks for UTC, so that is the expected case; anything else is
/// still accepted as UTC rather than silently mis-parsed, because guessing a
/// named Windows time zone here would be worse than being explicit.
fn convert(t: Option<&GraphTime>) -> Option<DateTime<Local>> {
    let t = t?;
    let raw = t.date_time.as_deref()?;
    // The request asks for UTC. If the service ignored that — an unusual
    // mailbox setting, an older tenant — say so instead of silently treating
    // another zone's wall clock as UTC and being hours off.
    if let Some(zone) = t.time_zone.as_deref()
        && !zone.eq_ignore_ascii_case("UTC")
    {
        crate::log::warn(&format!(
            "Graph returned time zone '{zone}' despite the UTC preference;              times for this event may be off"
        ));
    }
    // Graph uses seven fractional digits, which `parse_from_rfc3339` rejects
    // outright because there is no offset at all.
    let trimmed = raw.split('.').next().unwrap_or(raw);
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").ok()?;
    Some(Utc.from_utc_datetime(&naive).with_timezone(&Local))
}

fn parse_hex(s: Option<&str>) -> Option<u32> {
    let s = s?.trim().trim_start_matches('#');
    (s.len() == 6)
        .then(|| u32::from_str_radix(s, 16).ok())
        .flatten()
}

/// Outlook's named calendar colours, as shown in the web interface.
fn preset_color(name: Option<&str>) -> Option<u32> {
    Some(match name?.trim() {
        "lightBlue" => 0x4F_9BE8,
        "lightGreen" => 0x5D_C08C,
        "lightOrange" => 0xE8_9B4F,
        "lightGray" => 0x9A_A0A6,
        "lightYellow" => 0xE5_C453,
        "lightTeal" => 0x4F_C4C4,
        "lightPink" => 0xE8_7FA8,
        "lightBrown" => 0xA6_7C52,
        "lightRed" => 0xE0_5C5C,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_timestamps_are_read_as_utc_and_shown_locally() {
        // Seven fractional digits and no offset — this is what Graph sends.
        let t = GraphTime {
            date_time: Some("2026-08-05T09:30:00.0000000".into()),
            time_zone: Some("UTC".into()),
        };
        let converted = convert(Some(&t)).expect("must parse");
        assert_eq!(
            converted.with_timezone(&Utc).format("%H:%M").to_string(),
            "09:30"
        );
    }

    #[test]
    fn a_malformed_timestamp_yields_none_instead_of_a_wrong_time() {
        let t = GraphTime {
            date_time: Some("not a timestamp".into()),
            time_zone: Some("UTC".into()),
        };
        assert!(convert(Some(&t)).is_none());
        assert!(convert(None).is_none());
    }

    #[test]
    fn todo_due_dates_keep_their_calendar_day() {
        // The same trap as Google Tasks: naive time zone conversion would
        // move this to the 4th in any zone east of UTC.
        let due = parse_task_due("2026-08-05T00:00:00.0000000").expect("must parse");
        assert_eq!(due, NaiveDate::from_ymd_opt(2026, 8, 5).unwrap());
    }

    #[test]
    fn calendar_colours_prefer_the_exact_value() {
        assert_eq!(parse_hex(Some("#0F6CBD")), Some(0x0F_6CBD));
        assert_eq!(parse_hex(Some("0f6cbd")), Some(0x0F_6CBD));
        assert_eq!(parse_hex(Some("")), None);
        assert_eq!(preset_color(Some("lightGreen")), Some(0x5D_C08C));
        assert_eq!(preset_color(Some("auto")), None);
    }

    #[test]
    fn a_page_without_next_link_ends_the_walk() {
        let page: Page<TodoList> =
            serde_json::from_str(r#"{"value":[{"id":"1","displayName":"Tasks"}]}"#).unwrap();
        assert!(page.next_link.is_none());
        assert_eq!(page.value.len(), 1);

        let page: Page<TodoList> =
            serde_json::from_str(r#"{"value":[],"@odata.nextLink":"https://next"}"#).unwrap();
        assert_eq!(page.next_link.as_deref(), Some("https://next"));
    }

    #[test]
    fn teams_meetings_expose_their_join_link() {
        let raw = r#"{
            "subject": "Sprint Review",
            "isAllDay": false,
            "isOnlineMeeting": true,
            "onlineMeeting": { "joinUrl": "https://teams.microsoft.com/l/meetup-join/x" },
            "start": { "dateTime": "2026-08-05T09:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-08-05T10:00:00.0000000", "timeZone": "UTC" }
        }"#;
        let event: GraphEvent = serde_json::from_str(raw).unwrap();
        assert!(event.is_online_meeting);
        assert_eq!(
            event.online_meeting.and_then(|m| m.join_url).as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/x")
        );
    }

    #[test]
    fn an_ordinary_event_has_no_join_link() {
        let raw = r#"{"subject":"Lunch","isAllDay":false}"#;
        let event: GraphEvent = serde_json::from_str(raw).unwrap();
        assert!(!event.is_online_meeting);
        assert!(event.online_meeting.is_none());
    }
}
