// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Google Calendar and Google Tasks behind the common provider interface.
//!
//! The request handling itself lives in [`crate::google`]; this module only
//! adapts it to [`CalendarProvider`] and translates the error type, so that
//! the sync thread deals with exactly one shape of back end.

use super::{CalendarProvider, CalendarRef, Error, Result, TaskListRef};
use crate::google::{self, auth::Auth, calendar, tasks};
use crate::model::{Event, Task};
use chrono::{DateTime, Local, NaiveDate};

pub struct GoogleProvider {
    account_id: String,
    display_name: String,
    auth: Auth,
}

impl GoogleProvider {
    pub fn new(account_id: &str, display_name: &str) -> Result<Self> {
        Ok(Self {
            account_id: account_id.to_string(),
            display_name: display_name.to_string(),
            auth: Auth::load().map_err(convert)?,
        })
    }
}

/// Google's own error type mapped onto the shared one.
fn convert(e: google::Error) -> Error {
    match e {
        google::Error::NeedsSetup(m) => Error::NeedsSetup(m),
        google::Error::NeedsLogin(m) => Error::NeedsLogin(m),
        google::Error::Other(m) => Error::Other(m),
    }
}

impl CalendarProvider for GoogleProvider {
    fn account_id(&self) -> &str {
        &self.account_id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn ensure_authorized(&mut self) -> Result<()> {
        if !self.auth.has_refresh_token() {
            self.auth.interactive_login().map_err(convert)?;
        }
        Ok(())
    }

    fn forget(&mut self) {
        self.auth.forget();
    }

    fn calendars(&mut self) -> Result<Vec<CalendarRef>> {
        Ok(calendar::list_calendars(&mut self.auth)
            .map_err(convert)?
            .into_iter()
            .map(|c| CalendarRef {
                id: c.id,
                name: c.name,
                color: c.color,
            })
            .collect())
    }

    fn events(
        &mut self,
        cal: &CalendarRef,
        from: DateTime<Local>,
        to: DateTime<Local>,
        hide_declined: bool,
    ) -> Result<Vec<Event>> {
        let native = calendar::CalendarRef {
            id: cal.id.clone(),
            name: cal.name.clone(),
            color: cal.color,
        };
        calendar::list_events(&mut self.auth, &native, from, to, hide_declined).map_err(convert)
    }

    fn task_lists(&mut self) -> Result<Vec<TaskListRef>> {
        Ok(tasks::list_tasklists(&mut self.auth)
            .map_err(convert)?
            .into_iter()
            .map(|l| TaskListRef {
                id: l.id,
                name: l.name,
            })
            .collect())
    }

    fn tasks(
        &mut self,
        list: &TaskListRef,
        today: NaiveDate,
        include_undated: bool,
    ) -> Result<Vec<Task>> {
        let native = tasks::TaskListRef {
            id: list.id.clone(),
            name: list.name.clone(),
        };
        tasks::list_tasks(&mut self.auth, &native, today, include_undated).map_err(convert)
    }

    fn complete_task(&mut self, list_id: &str, task_id: &str) -> Result<()> {
        tasks::complete_task(&mut self.auth, list_id, task_id).map_err(convert)
    }
}
