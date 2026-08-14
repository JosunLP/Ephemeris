// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Calendar and task back ends.
//!
//! Every service the widget can talk to implements [`CalendarProvider`]. The
//! sync thread never knows which service it is looking at — it asks each
//! configured account for calendars, events, task lists and tasks, then merges
//! the results.
//!
//! Two properties matter more than the individual APIs:
//!
//! * **Accounts are queried in parallel.** A single account needs roughly ten
//!   round trips; doing three accounts one after another would triple the time
//!   the widget spends showing stale data. [`fetch_all`] fans out over a
//!   scoped thread per account, which keeps the "no async runtime" property of
//!   the rest of the program.
//! * **One broken account must not blank the widget.** Failures are collected
//!   per account and reported alongside whatever the healthy accounts
//!   returned. An expired Microsoft token should never hide today's Google
//!   meetings.

pub mod caldav;
pub mod google;
pub mod graph;
pub mod ical;
pub mod oauth;

use crate::model::{Event, Task};
use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// One agent for the whole process, shared by every provider.
///
/// Keeping the TLS session and connection alive matters: a sync run makes
/// roughly ten requests per account, and paying a full handshake for each of
/// them would dominate the time.
pub fn http() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            // We want to read the provider's error body, not just a bare
            // status code.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("TPMPlaner/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent()
    })
}

/// Extracts the service's own message from an error body.
///
/// Google nests it under `error.message`, Microsoft under `error.message` too,
/// and OAuth endpoints use `error_description`. Without this the status bar
/// would only ever say "HTTP 403".
pub fn api_error(status: u16, body: &str) -> Error {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let root = v.get("error").unwrap_or(&v);
            root.get("message")
                .or_else(|| root.get("error_description"))
                .or_else(|| v.get("error_description"))
                .and_then(|m| m.as_str().map(str::to_owned))
                .or_else(|| root.as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| body.chars().take(200).collect());

    match status {
        401 => Error::NeedsLogin(detail),
        403 if detail.contains("has not been used") || detail.contains("is disabled") => {
            Error::NeedsSetup(detail)
        }
        _ => Error::Other(format!("HTTP {status}: {detail}")),
    }
}

/// A calendar offered by a provider.
#[derive(Debug, Clone)]
pub struct CalendarRef {
    pub id: String,
    pub name: String,
    /// `0xRRGGBB`, taken from the service so the widget matches the web UI.
    pub color: u32,
}

/// A task list offered by a provider.
#[derive(Debug, Clone)]
pub struct TaskListRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum Error {
    /// One-time setup missing (credentials file, tenant, server URL).
    NeedsSetup(String),
    /// Sign-in required or expired.
    NeedsLogin(String),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NeedsSetup(m) | Error::NeedsLogin(m) | Error::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Which service an account talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Google,
    /// Any standards compliant CalDAV server: iCloud, Nextcloud, Fastmail,
    /// Synology, mailbox.org.
    Caldav,
    /// Microsoft Graph — covers Outlook calendars and Microsoft To Do.
    /// Teams meetings are ordinary Outlook events carrying a join link, so
    /// they need no separate back end.
    Microsoft,
}

/// One configured account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    pub kind: Kind,
    /// Stable identifier, used for the token file name and log messages.
    /// Free text so several accounts of the same kind can coexist
    /// ("work", "private").
    pub id: String,
    /// Shown in the widget when more than one account is configured.
    #[serde(default)]
    pub label: String,
    /// Off without removing the entry.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl AccountConfig {
    pub fn display(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// Everything the sync thread needs from a back end.
///
/// Deliberately blocking: the whole program runs without an async runtime, and
/// these calls only ever happen on the sync thread.
pub trait CalendarProvider: Send {
    /// Stable account identifier, matching [`AccountConfig::id`].
    fn account_id(&self) -> &str;

    /// Human readable account name for the UI.
    fn display_name(&self) -> &str;

    /// Sign in if no stored credential is available. Called before the first
    /// request of a sync run.
    fn ensure_authorized(&mut self) -> Result<()>;

    /// Discard the stored credential so the next run signs in again.
    fn forget(&mut self);

    fn calendars(&mut self) -> Result<Vec<CalendarRef>>;

    fn events(
        &mut self,
        calendar: &CalendarRef,
        from: DateTime<Local>,
        to: DateTime<Local>,
        hide_declined: bool,
    ) -> Result<Vec<Event>>;

    /// Not every service has tasks; the default is "none".
    fn task_lists(&mut self) -> Result<Vec<TaskListRef>> {
        Ok(Vec::new())
    }

    fn tasks(
        &mut self,
        _list: &TaskListRef,
        _today: NaiveDate,
        _include_undated: bool,
    ) -> Result<Vec<Task>> {
        Ok(Vec::new())
    }

    fn complete_task(&mut self, _list_id: &str, _task_id: &str) -> Result<()> {
        Err(Error::Other(
            "This account does not support completing tasks".into(),
        ))
    }
}

/// What one account contributed to a sync run.
#[derive(Debug, Default)]
pub struct AccountData {
    pub account_id: String,
    pub display_name: String,
    pub events: Vec<Event>,
    pub tasks: Vec<Task>,
    pub calendars: Vec<CalendarRef>,
    pub task_lists: Vec<TaskListRef>,
    /// `None` when the account succeeded.
    pub error: Option<Error>,
}

impl AccountData {
    fn failed(id: &str, name: &str, error: Error) -> Self {
        Self {
            account_id: id.to_string(),
            display_name: name.to_string(),
            error: Some(error),
            ..Default::default()
        }
    }
}

/// What a single account should fetch.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub from: DateTime<Local>,
    pub to: DateTime<Local>,
    pub today: NaiveDate,
    pub hide_declined: bool,
    pub include_undated_tasks: bool,
    /// Empty means "every calendar the account offers". Entries are matched
    /// against `CalendarRef::id`.
    pub calendar_ids: Vec<String>,
    pub tasklist_ids: Vec<String>,
    /// Previously discovered calendars and task lists, keyed by account id.
    ///
    /// Calendar and task list directories change a handful of times a year.
    /// Re-discovering them on every run costs two of roughly ten requests per
    /// account without ever returning anything new, so the sync thread hands
    /// back what it already knows until the cache expires.
    pub cached_directory: HashMap<String, Directory>,
}

/// One account's calendars and task lists.
#[derive(Debug, Clone, Default)]
pub struct Directory {
    pub calendars: Vec<CalendarRef>,
    pub task_lists: Vec<TaskListRef>,
}

/// Queries every account at the same time.
///
/// `std::thread::scope` lets each worker borrow its own provider mutably for
/// the duration of the call, which is what the token refresh inside needs.
/// Nothing is shared between the workers, so there is no lock and no ordering
/// between accounts; the results are reassembled in the caller's order
/// afterwards so the display stays stable between runs.
pub fn fetch_all(
    providers: &mut [Box<dyn CalendarProvider>],
    request: &FetchRequest,
) -> Vec<AccountData> {
    if providers.is_empty() {
        return Vec::new();
    }

    std::thread::scope(|scope| {
        let handles: Vec<_> = providers
            .iter_mut()
            .map(|provider| scope.spawn(move || fetch_one(provider.as_mut(), request)))
            .collect();

        handles
            .into_iter()
            .map(|h| {
                // A panic inside one provider must not take the sync thread
                // with it; the panic hook has already written it to the log.
                h.join().unwrap_or_else(|_| {
                    AccountData::failed("?", "?", Error::Other("Provider panicked".into()))
                })
            })
            .collect()
    })
}

fn fetch_one(provider: &mut dyn CalendarProvider, request: &FetchRequest) -> AccountData {
    let id = provider.account_id().to_string();
    let name = provider.display_name().to_string();

    match fetch_inner(provider, request) {
        Ok(mut data) => {
            data.account_id = id;
            data.display_name = name;
            data
        }
        Err(e) => AccountData::failed(&id, &name, e),
    }
}

fn fetch_inner(provider: &mut dyn CalendarProvider, request: &FetchRequest) -> Result<AccountData> {
    provider.ensure_authorized()?;

    let cached = request.cached_directory.get(provider.account_id());
    let calendars = match cached {
        Some(d) => d.calendars.clone(),
        None => provider.calendars()?,
    };
    let selected: Vec<CalendarRef> = calendars
        .iter()
        .filter(|c| request.calendar_ids.is_empty() || request.calendar_ids.contains(&c.id))
        .cloned()
        .collect();

    let mut events = Vec::new();
    for calendar in &selected {
        events.extend(provider.events(
            calendar,
            request.from,
            request.to,
            request.hide_declined,
        )?);
    }

    let task_lists = match cached {
        Some(d) => d.task_lists.clone(),
        None => provider.task_lists()?,
    };
    let selected_lists: Vec<TaskListRef> = task_lists
        .iter()
        .filter(|l| request.tasklist_ids.is_empty() || request.tasklist_ids.contains(&l.id))
        .cloned()
        .collect();

    let mut tasks = Vec::new();
    for list in &selected_lists {
        tasks.extend(provider.tasks(list, request.today, request.include_undated_tasks)?);
    }

    Ok(AccountData {
        account_id: String::new(),
        display_name: String::new(),
        events,
        tasks,
        calendars,
        task_lists,
        error: None,
    })
}

/// Turns the per-account errors into one short line for the status bar.
///
/// The full text of each failure goes to the log; the footer has room for
/// roughly one sentence.
pub fn summarize_errors(results: &[AccountData]) -> Option<String> {
    let failed: Vec<&AccountData> = results.iter().filter(|r| r.error.is_some()).collect();
    match failed.as_slice() {
        [] => None,
        [only] => Some(format!(
            "{}: {}",
            only.display_name,
            only.error.as_ref().expect("filtered above")
        )),
        many => Some(format!("{} accounts failed", many.len())),
    }
}

/// The most pressing error kind across all accounts.
///
/// Sign-in problems outrank plain network errors: they need the user to act,
/// while a network error resolves itself.
pub fn worst_error(results: &[AccountData]) -> Option<&Error> {
    let errors: Vec<&Error> = results.iter().filter_map(|r| r.error.as_ref()).collect();
    errors
        .iter()
        .find(|e| matches!(e, Error::NeedsSetup(_)))
        .or_else(|| errors.iter().find(|e| matches!(e, Error::NeedsLogin(_))))
        .or_else(|| errors.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    struct Stub {
        id: &'static str,
        fail: Option<Error>,
        events: usize,
    }

    impl CalendarProvider for Stub {
        fn account_id(&self) -> &str {
            self.id
        }
        fn display_name(&self) -> &str {
            self.id
        }
        fn ensure_authorized(&mut self) -> Result<()> {
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
        fn forget(&mut self) {}
        fn calendars(&mut self) -> Result<Vec<CalendarRef>> {
            Ok(vec![CalendarRef {
                id: format!("{}-cal", self.id),
                name: self.id.into(),
                color: 0,
            }])
        }
        fn events(
            &mut self,
            _c: &CalendarRef,
            _f: DateTime<Local>,
            _t: DateTime<Local>,
            _d: bool,
        ) -> Result<Vec<Event>> {
            Ok((0..self.events)
                .map(|i| Event {
                    title: format!("{} {i}", self.id),
                    start: None,
                    end: None,
                    all_day: true,
                    location: None,
                    html_link: None,
                    join_url: None,
                    color: 0,
                    calendar_name: self.id.into(),
                    calendar_id: format!("{}-cal", self.id),
                    account_id: String::new(),
                    task_id: None,
                    task_list_id: None,
                })
                .collect())
        }
    }

    fn request() -> FetchRequest {
        let now = Local.with_ymd_and_hms(2026, 8, 5, 8, 0, 0).unwrap();
        FetchRequest {
            from: now,
            to: now,
            today: now.date_naive(),
            hide_declined: true,
            include_undated_tasks: false,
            calendar_ids: Vec::new(),
            tasklist_ids: Vec::new(),
            cached_directory: HashMap::new(),
        }
    }

    #[test]
    fn a_failing_account_does_not_hide_the_healthy_ones() {
        let mut providers: Vec<Box<dyn CalendarProvider>> = vec![
            Box::new(Stub {
                id: "google",
                fail: None,
                events: 3,
            }),
            Box::new(Stub {
                id: "work",
                fail: Some(Error::NeedsLogin("token expired".into())),
                events: 99,
            }),
        ];
        let results = fetch_all(&mut providers, &request());

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].events.len(), 3, "healthy account must survive");
        assert!(results[0].error.is_none());
        assert!(results[1].events.is_empty());
        assert!(results[1].error.is_some());
    }

    #[test]
    fn results_keep_the_configured_order() {
        // Threads finish in whatever order they like; the display must not.
        let mut providers: Vec<Box<dyn CalendarProvider>> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|id| {
                Box::new(Stub {
                    id: Box::leak(id.to_string().into_boxed_str()),
                    fail: None,
                    events: 1,
                }) as Box<dyn CalendarProvider>
            })
            .collect();
        let results = fetch_all(&mut providers, &request());
        let ids: Vec<&str> = results.iter().map(|r| r.account_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn sign_in_problems_outrank_network_errors() {
        let results = vec![
            AccountData::failed("a", "A", Error::Other("connection reset".into())),
            AccountData::failed("b", "B", Error::NeedsLogin("expired".into())),
        ];
        assert!(matches!(worst_error(&results), Some(Error::NeedsLogin(_))));
    }

    #[test]
    fn setup_problems_outrank_sign_in_problems() {
        let results = vec![
            AccountData::failed("a", "A", Error::NeedsLogin("expired".into())),
            AccountData::failed("b", "B", Error::NeedsSetup("no credentials".into())),
        ];
        assert!(matches!(worst_error(&results), Some(Error::NeedsSetup(_))));
    }

    #[test]
    fn one_failure_is_named_several_are_counted() {
        let one = vec![AccountData::failed(
            "a",
            "Work",
            Error::NeedsLogin("expired".into()),
        )];
        assert_eq!(summarize_errors(&one).unwrap(), "Work: expired");

        let many = vec![
            AccountData::failed("a", "Work", Error::Other("x".into())),
            AccountData::failed("b", "Home", Error::Other("y".into())),
        ];
        assert_eq!(summarize_errors(&many).unwrap(), "2 accounts failed");
    }

    #[test]
    fn no_accounts_is_not_an_error() {
        let mut none: Vec<Box<dyn CalendarProvider>> = Vec::new();
        assert!(fetch_all(&mut none, &request()).is_empty());
        assert!(summarize_errors(&[]).is_none());
    }
}
