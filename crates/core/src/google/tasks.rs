// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Google Tasks API v1.
//!
//! Two quirks of this API the widget has to allow for:
//!
//! 1. `due` is documented as a plain date, normalised server side to
//!    `T00:00:00.000Z` — but the app lets a task carry a time of day, and such
//!    a value comes back as a real instant. Which day that is depends on the
//!    zone; see [`crate::model::parse_task_due`].
//! 2. As soon as `dueMin` or `dueMax` is set, tasks **without** a due date
//!    vanish from the response entirely. Seeing them means querying unfiltered
//!    and sieving on the client.
//! 3. Neither bound documents whether it is inclusive, which is why
//!    [`due_bound`] does not put one on the day being asked about.

use super::auth::Auth;
use super::{Result, agent, api_error, urlencode};
use crate::model::{Task, parse_task_due};
use chrono::{Duration, NaiveDate};
use serde::Deserialize;
use std::collections::HashMap;

const BASE: &str = "https://tasks.googleapis.com/tasks/v1";

#[derive(Debug, Clone)]
pub struct TaskListRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct TaskListsResponse {
    #[serde(default)]
    items: Vec<TaskListEntry>,
}

#[derive(Debug, Deserialize)]
struct TaskListEntry {
    id: String,
    #[serde(default)]
    title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TasksResponse {
    #[serde(default)]
    items: Vec<TaskEntry>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskEntry {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    due: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    deleted: Option<bool>,
    #[serde(default)]
    hidden: Option<bool>,
}

pub fn list_tasklists(auth: &mut Auth) -> Result<Vec<TaskListRef>> {
    let url = format!("{BASE}/users/@me/lists?maxResults=100&fields=items(id,title)");
    let body = request(auth, "GET", &url, None)?;
    let parsed: TaskListsResponse = serde_json::from_str(&body)?;
    Ok(parsed
        .items
        .into_iter()
        .map(|l| TaskListRef {
            name: if l.title.is_empty() {
                "Aufgaben".into()
            } else {
                l.title
            },
            id: l.id,
        })
        .collect())
}

/// The open tasks of one list.
///
/// With `include_undated` off the server filters through `dueMax`, so the
/// "not due for three weeks" tasks never go over the wire at all. Otherwise
/// everything has to be fetched; see the module comment.
pub fn list_tasks(
    auth: &mut Auth,
    list: &TaskListRef,
    today: NaiveDate,
    include_undated: bool,
) -> Result<Vec<Task>> {
    let due_max = due_bound(today);

    let mut raw: Vec<TaskEntry> = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let mut url = format!(
            "{BASE}/lists/{}/tasks?showCompleted=false&showHidden=false&showDeleted=false\
             &maxResults=100&fields=nextPageToken,items(id,title,due,notes,status,parent,deleted,hidden)",
            urlencode(&list.id),
        );
        if !include_undated {
            url.push_str(&format!("&dueMax={}", urlencode(&due_max)));
        }
        if let Some(token) = &page_token {
            url.push_str(&format!("&pageToken={}", urlencode(token)));
        }

        let body = request(auth, "GET", &url, None)?;
        let parsed: TasksResponse = serde_json::from_str(&body)?;
        raw.extend(parsed.items);

        match parsed.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }

    // Work out the depth from the parent chain before filtering, or a
    // subtask loses its parent.
    let by_id: HashMap<&str, &TaskEntry> = raw.iter().map(|t| (t.id.as_str(), t)).collect();

    let tasks = raw
        .iter()
        .filter(|t| {
            !t.deleted.unwrap_or(false)
                && !t.hidden.unwrap_or(false)
                && t.status.as_deref() != Some("completed")
        })
        .map(|t| Task {
            id: t.id.clone(),
            tasklist_id: list.id.clone(),
            title: if t.title.trim().is_empty() {
                "(no title)".into()
            } else {
                t.title.clone()
            },
            due: t.due.as_deref().and_then(parse_task_due),
            notes: t.notes.clone().filter(|n| !n.trim().is_empty()),
            depth: depth_of(t, &by_id),
            tasklist_name: list.name.clone(),
            account_id: String::new(),
            completing: false,
        })
        .collect();

    Ok(tasks)
}

/// The `dueMax` sent to the server: the end of **tomorrow**, not of today.
///
/// A day of slack, deliberately. Google documents neither whether the bound is
/// inclusive nor what it does with the time of day, and a task due today sits
/// exactly on a bound of today — an off-by-one there drops precisely the tasks
/// due today while leaving the overdue ones in place, which is what was
/// reported. Due dates that carry a time carry a zone with them as well, and
/// local midnight is up to fourteen hours away from the UTC one, so the bound
/// has to clear that too.
///
/// This filter is an optimisation, nothing more: it keeps the "not due for
/// three weeks" bulk off the wire. Which tasks are actually shown is decided by
/// [`crate::model::filter_tasks_for_today`], so a day of extra data costs one
/// filter pass and cannot put a future task on screen.
fn due_bound(today: NaiveDate) -> String {
    format!(
        "{}T23:59:59.999Z",
        (today + Duration::days(1)).format("%Y-%m-%d")
    )
}

/// Nesting depth, guarded against cycles and capped at three levels.
fn depth_of(entry: &TaskEntry, by_id: &HashMap<&str, &TaskEntry>) -> u8 {
    let mut depth = 0u8;
    let mut cursor = entry.parent.as_deref();
    while let Some(parent_id) = cursor {
        depth += 1;
        if depth >= 3 {
            break;
        }
        cursor = by_id.get(parent_id).and_then(|p| p.parent.as_deref());
    }
    depth
}

/// Completes a task. Requires the full `tasks` scope.
pub fn complete_task(auth: &mut Auth, tasklist_id: &str, task_id: &str) -> Result<()> {
    let url = format!(
        "{BASE}/lists/{}/tasks/{}",
        urlencode(tasklist_id),
        urlencode(task_id)
    );
    request(auth, "PATCH", &url, Some(r#"{"status":"completed"}"#))?;
    Ok(())
}

fn request(auth: &mut Auth, method: &str, url: &str, body: Option<&str>) -> Result<String> {
    let token = auth.access_token()?;
    let bearer = format!("Bearer {token}");

    let resp = match (method, body) {
        ("PATCH", Some(payload)) => agent()
            .patch(url)
            .header("Authorization", bearer)
            .header("Content-Type", "application/json")
            .send(payload)?,
        _ => agent().get(url).header("Authorization", bearer).call()?,
    };

    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string()?;
    if !(200..300).contains(&status) {
        return Err(api_error(status, &text));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_due_bound_clears_today_by_a_full_day() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 7).unwrap();
        assert_eq!(due_bound(today), "2026-08-08T23:59:59.999Z");
    }

    #[test]
    fn the_due_bound_crosses_a_month_end() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        assert_eq!(due_bound(today), "2026-09-01T23:59:59.999Z");
    }
}
