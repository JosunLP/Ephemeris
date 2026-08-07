// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Background synchronisation on a thread of its own.
//!
//! The user interface owns all the scheduling and sends commands in; this
//! thread carries them out, writes the result into the shared state and wakes
//! the interface through a [`Waker`]. No network call ever blocks drawing.

use crate::config::{self, Config};
use crate::host::Waker;
use crate::log;
use crate::model::{
    Agenda, Event, Task, filter_tasks_for_today, local_day_start, sort_events, sort_tasks,
};
use crate::provider::{
    self, AccountConfig, CalendarProvider, CalendarRef, Error, FetchRequest, Kind, TaskListRef,
};
use chrono::{Duration as ChronoDuration, Local};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Calendar and task list directories change a handful of times a year.
/// Re-fetching them on every run costs two of roughly ten requests per sync
/// and never returns anything new.
const META_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Why the user interface is being woken.
///
/// The sync thread must not know what kind of window it is talking to; on
/// Windows this becomes a posted message, another toolkit would use its own
/// event loop proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// A command finished; the agenda or the status may have changed.
    Finished,
    /// Only the status line moved, for instance "syncing" turning on.
    Status,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Syncing,
    /// One-time setup missing, such as a credentials file.
    NeedsSetup(String),
    /// Sign-in required or expired.
    NeedsLogin(String),
    Error(String),
}

/// State shared between the interface and the sync thread.
pub struct Shared {
    pub agenda: Agenda,
    pub status: Status,
    pub config: Config,
    /// Syntax error in `config.json`, if there is one.
    pub config_error: Option<String>,
    /// Available calendars as `(id, name)`, the basis for picking them in the
    /// context menu. Before this, the ids had to be typed into the settings
    /// file by hand.
    pub calendars: Vec<(String, String)>,
    pub tasklists: Vec<(String, String)>,
    /// A newer release, once the daily check has found one.
    pub update: Option<crate::update::Available>,
}

/// Locks the shared state and survives poisoning.
///
/// `Mutex::lock` returns `Err` as soon as any thread panicked while holding
/// it; from then on every `.unwrap()` would trigger the next panic and turn a
/// local fault into a total failure. The content here is display data only —
/// at worst a field is half written, which the next sync corrects anyway.
pub fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Cached directory of calendars and task lists, per account.
struct Meta {
    by_account: HashMap<String, provider::Directory>,
    at: Instant,
}

impl Meta {
    fn fresh(&self) -> bool {
        self.at.elapsed() < META_TTL
    }

    /// Merged view for the context menu, in configured account order.
    fn merged(&self, order: &[String]) -> (Vec<CalendarRef>, Vec<TaskListRef>) {
        let mut calendars = Vec::new();
        let mut lists = Vec::new();
        for id in order {
            if let Some(d) = self.by_account.get(id) {
                calendars.extend(d.calendars.iter().cloned());
                lists.extend(d.task_lists.iter().cloned());
            }
        }
        (calendars, lists)
    }
}

/// Builds the provider objects for the configured accounts.
///
/// A provider that cannot even be constructed — missing credentials file, for
/// instance — is not dropped silently: it becomes a provider that fails with
/// that very message, so the reason reaches the status bar instead of the
/// account merely disappearing.
fn build_providers(accounts: &[AccountConfig]) -> Vec<Box<dyn CalendarProvider>> {
    accounts
        .iter()
        .map(|account| -> Box<dyn CalendarProvider> {
            let built: provider::Result<Box<dyn CalendarProvider>> = match account.kind {
                Kind::Google => {
                    provider::google::GoogleProvider::new(&account.id, account.display())
                        .map(|p| Box::new(p) as Box<dyn CalendarProvider>)
                }
                Kind::Caldav => provider::caldav::CalDavProvider::new(
                    &account.id,
                    account.display(),
                    &config::data_dir().join(format!("caldav-{}.json", account.id)),
                    config::data_dir().join(format!("token-{}.bin", account.id)),
                )
                .map(|p| Box::new(p) as Box<dyn CalendarProvider>),
                Kind::Microsoft => provider::graph::GraphProvider::new(
                    &account.id,
                    account.display(),
                    &config::data_dir().join("microsoft_client.json"),
                    config::data_dir().join(format!("token-{}.bin", account.id)),
                )
                .map(|p| Box::new(p) as Box<dyn CalendarProvider>),
            };
            match built {
                Ok(p) => p,
                Err(e) => Box::new(BrokenProvider {
                    id: account.id.clone(),
                    name: account.display().to_string(),
                    error: e,
                }),
            }
        })
        .collect()
}

/// Stand-in for an account that could not be set up.
struct BrokenProvider {
    id: String,
    name: String,
    error: Error,
}

impl CalendarProvider for BrokenProvider {
    fn account_id(&self) -> &str {
        &self.id
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn ensure_authorized(&mut self) -> provider::Result<()> {
        Err(self.error.clone())
    }
    fn forget(&mut self) {}
    fn calendars(&mut self) -> provider::Result<Vec<CalendarRef>> {
        Err(self.error.clone())
    }
    fn events(
        &mut self,
        _c: &CalendarRef,
        _f: chrono::DateTime<Local>,
        _t: chrono::DateTime<Local>,
        _d: bool,
    ) -> provider::Result<Vec<crate::model::Event>> {
        Err(self.error.clone())
    }
}

pub enum Command {
    /// A normal sync: timer, waking from standby, or a manual click.
    Sync,
    /// Complete a task; hidden optimistically before the call goes out.
    CompleteTask {
        /// Which account owns it. Empty means "the only one", which is what a
        /// single-account setup produces.
        account_id: String,
        tasklist_id: String,
        task_id: String,
    },
    /// Discard the stored credential and sign in again interactively.
    Relogin,
    Quit,
}

#[derive(Clone)]
pub struct SyncHandle {
    tx: Sender<Command>,
}

impl SyncHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

/// Starts the sync thread.
///
/// `waker` is how finished work reaches the user interface. Nothing else in
/// this crate knows what a window is.
pub fn spawn(shared: Arc<Mutex<Shared>>, waker: Arc<dyn Waker>) -> SyncHandle {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("tpmplaner-sync".into())
        .spawn(move || worker(shared, waker, rx))
        .expect("could not start the sync thread");
    SyncHandle { tx }
}

fn worker(shared: Arc<Mutex<Shared>>, waker: Arc<dyn Waker>, rx: Receiver<Command>) {
    let mut providers = build_providers(&snapshot_config(&shared).effective_accounts());
    let mut meta: Option<Meta> = None;
    // Ride along with the sync run rather than opening a second connection.
    let mut last_update_check: Option<Instant> = None;

    while let Ok(first) = rx.recv() {
        for cmd in coalesce(first, &rx) {
            match cmd {
                Command::Quit => return,

                Command::Sync => {
                    set_status(&shared, &waker, Status::Syncing);
                    let cfg = snapshot_config(&shared);
                    // Accounts can be added or removed while running; rebuild
                    // when the configured set no longer matches.
                    let wanted = cfg.effective_accounts();
                    if !same_accounts(&providers, &wanted) {
                        log::info("Account list changed — rebuilding providers");
                        providers = build_providers(&wanted);
                        meta = None;
                    }
                    let result = run_sync(&mut providers, cfg, &mut meta);
                    publish_sources(&shared, &meta);
                    apply_sync_result(&shared, &waker, result);
                    check_for_update(&shared, &waker, &mut last_update_check);
                }

                Command::Relogin => {
                    log::info("Re-authorization requested");
                    set_status(&shared, &waker, Status::Syncing);
                    for p in providers.iter_mut() {
                        p.forget();
                    }
                    let cfg = snapshot_config(&shared);
                    providers = build_providers(&cfg.effective_accounts());
                    // The directory belonged to the previous sign-in.
                    meta = None;
                    let result = run_sync(&mut providers, cfg, &mut meta);
                    publish_sources(&shared, &meta);
                    apply_sync_result(&shared, &waker, result);
                }

                Command::CompleteTask {
                    account_id,
                    tasklist_id,
                    task_id,
                } => {
                    let outcome = match providers
                        .iter_mut()
                        .find(|p| account_id.is_empty() || p.account_id() == account_id)
                    {
                        Some(p) => p.complete_task(&tasklist_id, &task_id),
                        None => Err(Error::Other(format!("Unknown account '{account_id}'"))),
                    };

                    let mut guard = lock(&shared);
                    match outcome {
                        Ok(()) => {
                            // Remove for good; the next regular sync confirms it.
                            guard.agenda.tasks.retain(|t| t.id != task_id);
                            guard.status = Status::Idle;
                            write_cache(&guard.agenda);
                        }
                        Err(e) => {
                            log::error(&format!("Completing task failed: {e}"));
                            // Undo the optimistic hide.
                            if let Some(t) = guard.agenda.tasks.iter_mut().find(|t| t.id == task_id)
                            {
                                t.completing = false;
                            }
                            guard.status = status_for(&e);
                        }
                    }
                    drop(guard);
                    notify(&waker);
                }
            }
        }
    }
}

/// Looks for a newer release at most once a day.
fn check_for_update(
    shared: &Arc<Mutex<Shared>>,
    waker: &Arc<dyn Waker>,
    last: &mut Option<Instant>,
) {
    if last.is_some_and(|t| t.elapsed() < crate::update::CHECK_INTERVAL) {
        return;
    }
    *last = Some(Instant::now());

    if let Some(available) = crate::update::check() {
        log::info(&format!("Update available: {}", available.version));
        lock(shared).update = Some(available);
        notify(waker);
    }
}

/// Do the live providers still match the configured accounts?
fn same_accounts(providers: &[Box<dyn CalendarProvider>], wanted: &[AccountConfig]) -> bool {
    providers.len() == wanted.len()
        && providers
            .iter()
            .zip(wanted)
            .all(|(p, a)| p.account_id() == a.id)
}

/// Passes the calendar and task list directory on to the interface.
fn publish_sources(shared: &Arc<Mutex<Shared>>, meta: &Option<Meta>) {
    let Some(m) = meta else { return };
    let mut guard = lock(shared);
    let order: Vec<String> = guard
        .config
        .effective_accounts()
        .iter()
        .map(|a| a.id.clone())
        .collect();
    let (calendars, lists) = m.merged(&order);
    guard.calendars = calendars.into_iter().map(|c| (c.id, c.name)).collect();
    guard.tasklists = lists.into_iter().map(|l| (l.id, l.name)).collect();
}

/// Collapses queued commands.
///
/// During a long run — a browser sign-in may take five minutes — further
/// requests pile up in the channel. Syncing ten times in a row returns the
/// same result ten times and only spends API quota, so exactly one of several
/// `Sync` commands survives. Completions and re-authorisations each matter in
/// their own right and are kept in full.
fn coalesce(first: Command, rx: &Receiver<Command>) -> Vec<Command> {
    let mut batch = vec![first];
    while let Ok(next) = rx.try_recv() {
        batch.push(next);
    }
    if batch.len() == 1 {
        return batch;
    }

    let mut seen_sync = false;
    let kept: Vec<Command> = batch
        .into_iter()
        .filter(|c| {
            if matches!(c, Command::Sync) {
                let first_one = !seen_sync;
                seen_sync = true;
                return first_one;
            }
            true
        })
        .collect();
    kept
}

fn snapshot_config(shared: &Arc<Mutex<Shared>>) -> Config {
    lock(shared).config.clone()
}

fn run_sync(
    providers: &mut [Box<dyn CalendarProvider>],
    cfg: Config,
    meta: &mut Option<Meta>,
) -> std::result::Result<Agenda, Error> {
    let started = Instant::now();
    let today = Local::now().date_naive();
    let tomorrow = today + ChronoDuration::days(1);
    let day_start = local_day_start(today);
    let midnight = local_day_start(tomorrow);
    // Two days in one request: the same call yields the tomorrow preview for
    // free, a second one would be pure waste.
    let window_end = local_day_start(today + ChronoDuration::days(2));

    let request = FetchRequest {
        from: day_start,
        to: window_end,
        today,
        hide_declined: cfg.hide_declined,
        include_undated_tasks: cfg.show_undated_tasks,
        calendar_ids: cfg.calendar_ids.clone(),
        tasklist_ids: cfg.tasklist_ids.clone(),
        cached_directory: match meta.as_ref() {
            Some(m) if m.fresh() => m.by_account.clone(),
            _ => HashMap::new(),
        },
    };

    // All accounts at once. One slow mailbox must not delay the others.
    let results = provider::fetch_all(providers, &request);

    // Every account that answered contributes, whatever the others did.
    let mut events = Vec::new();
    let mut tasks: Vec<Task> = Vec::new();
    let mut directory: HashMap<String, provider::Directory> = HashMap::new();
    for account in &results {
        if let Some(e) = &account.error {
            log::warn(&format!("Account '{}': {e}", account.display_name));
            continue;
        }
        events.extend(account.events.iter().cloned());
        directory.insert(
            account.account_id.clone(),
            provider::Directory {
                calendars: account.calendars.clone(),
                task_lists: account.task_lists.clone(),
            },
        );
        for task in &account.tasks {
            let mut task = task.clone();
            // Stamp the origin so completing routes back to the right account.
            task.account_id = account.account_id.clone();
            tasks.push(task);
        }
    }

    let succeeded = results.iter().filter(|r| r.error.is_none()).count();
    if succeeded == 0 && !results.is_empty() {
        // Nothing came back at all — report the most actionable reason rather
        // than replacing the display with an empty day.
        return Err(provider::worst_error(&results)
            .cloned()
            .unwrap_or_else(|| Error::Other("No account returned data".into())));
    }

    // Keep the previous entry for an account that failed this round, so a
    // single hiccup does not force a full rediscovery next time.
    let reused = meta.as_ref().filter(|m| m.fresh());
    if let Some(old) = reused {
        for (id, dir) in &old.by_account {
            directory.entry(id.clone()).or_insert_with(|| dir.clone());
        }
    }
    let keep_timestamp = reused.map(|m| m.at);
    *meta = Some(Meta {
        by_account: directory,
        at: keep_timestamp.unwrap_or_else(Instant::now),
    });

    // Split by day. All-day events have no start time and stay with today.
    let (mut events, mut next_day): (Vec<Event>, Vec<Event>) = events
        .into_iter()
        .partition(|e| e.start.map(|s| s < midnight).unwrap_or(true));
    sort_events(&mut events);
    sort_events(&mut next_day);

    // Server side filtering only happened where the API supports it; this
    // drops everything still in the future for every provider alike.
    let mut tasks = filter_tasks_for_today(tasks, today, cfg.show_undated_tasks);
    sort_tasks(&mut tasks);

    log::info(&format!(
        "Sync ok: {} events today (+{} tomorrow), {} tasks, {}/{} accounts, {} ms",
        events.len(),
        next_day.len(),
        tasks.len(),
        succeeded,
        results.len(),
        started.elapsed().as_millis(),
    ));

    Ok(Agenda {
        day: Some(today),
        events,
        tomorrow: next_day,
        tasks,
        fetched_at: Some(Local::now()),
        // Partial failure is worth showing, but not worth blanking the widget.
        last_error: provider::summarize_errors(&results),
    })
}

fn apply_sync_result(
    shared: &Arc<Mutex<Shared>>,
    waker: &Arc<dyn Waker>,
    result: std::result::Result<Agenda, Error>,
) {
    {
        let mut guard = lock(shared);
        match result {
            Ok(agenda) => {
                write_cache(&agenda);
                guard.agenda = agenda;
                guard.status = Status::Idle;
            }
            Err(e) => {
                // The full text does not fit the status line, but it still
                // belongs in the log.
                log::error(&format!("Sync failed: {e}"));
                // Deliberately discard *no* data: a dropped wireless
                // connection should tint the status line, not empty the
                // widget.
                guard.status = status_for(&e);
                guard.agenda.last_error = Some(e.to_string());
            }
        }
    }
    notify(waker);
}

fn status_for(e: &Error) -> Status {
    match e {
        Error::NeedsSetup(m) => Status::NeedsSetup(m.clone()),
        Error::NeedsLogin(m) => Status::NeedsLogin(m.clone()),
        Error::Other(m) => Status::Error(m.clone()),
    }
}

fn set_status(shared: &Arc<Mutex<Shared>>, waker: &Arc<dyn Waker>, status: Status) {
    lock(shared).status = status;
    notify(waker);
}

fn notify(waker: &Arc<dyn Waker>) {
    waker.wake();
}

// --- Cache -----------------------------------------------------------------
//
// So something is on screen immediately at start-up rather than a blank
// panel, until the first network round trip completes.

pub fn read_cache() -> Option<Agenda> {
    let raw = std::fs::read_to_string(config::cache_path()).ok()?;
    let agenda: Agenda = serde_json::from_str(&raw).ok()?;
    // Yesterday's state is worthless and would in fact be misleading.
    if agenda.day? != Local::now().date_naive() {
        return None;
    }
    Some(agenda)
}

fn write_cache(agenda: &Agenda) {
    let path = config::cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(agenda) {
        let _ = std::fs::write(path, json);
    }
}
