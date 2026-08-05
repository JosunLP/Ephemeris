// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Hintergrund-Sync in einem eigenen Thread.
//!
//! Der UI-Thread besitzt die gesamte Zeitplanung (WM_TIMER) und schickt
//! Befehle herein; dieser Thread fuehrt sie aus, schreibt das Ergebnis in den
//! gemeinsamen Zustand und weckt das Fenster per `PostMessage`. Damit
//! blockiert kein Netzwerkaufruf jemals das Zeichnen.

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

/// Kalender- und Aufgabenlistenverzeichnis aendern sich hoechstens ein paarmal
/// im Jahr. Sie bei jedem Lauf neu zu holen kostet zwei von rund zehn
/// Anfragen pro Sync, ohne je etwas Neues zu liefern.
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
    /// Einmalige Einrichtung noetig (client_secret.json).
    NeedsSetup(String),
    /// Anmeldung noetig oder abgelaufen.
    NeedsLogin(String),
    Error(String),
}

/// Von UI- und Sync-Thread geteilter Zustand.
pub struct Shared {
    pub agenda: Agenda,
    pub status: Status,
    pub config: Config,
    /// Syntaxfehler in `config.json`, falls vorhanden.
    pub config_error: Option<String>,
    /// Verfuegbare Kalender als `(id, Name)` — Grundlage fuer die Auswahl im
    /// Kontextmenue. Vorher musste man die IDs von Hand in die JSON eintragen.
    pub calendars: Vec<(String, String)>,
    pub tasklists: Vec<(String, String)>,
    /// A newer release, once the daily check has found one.
    pub update: Option<crate::update::Available>,
}

/// Sperrt den geteilten Zustand und ueberlebt eine Vergiftung.
///
/// `Mutex::lock` liefert `Err`, sobald irgendein Thread waehrend des Haltens
/// gepanickt ist — ab dann wuerde jedes `.unwrap()` die naechste Panik
/// ausloesen und aus einem lokalen Fehler einen Totalausfall machen. Der
/// Inhalt ist hier reine Anzeigedaten; im schlimmsten Fall ist ein Feld
/// halb geschrieben, was der naechste Abgleich ohnehin korrigiert.
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
    /// Regulaerer Abgleich (Timer, Aufwachen, manueller Klick).
    Sync,
    /// Complete a task; hidden optimistically before the call goes out.
    CompleteTask {
        /// Which account owns it. Empty means "the only one", which is what a
        /// single-account setup produces.
        account_id: String,
        tasklist_id: String,
        task_id: String,
    },
    /// Zugang verwerfen und interaktiv neu anmelden.
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

/// Reicht das Kalender-/Listenverzeichnis an die Oberflaeche weiter.
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

/// Fasst aufgestaute Befehle zusammen.
///
/// Waehrend eines langen Laufs — die Browser-Anmeldung darf fuenf Minuten
/// dauern — sammeln sich weitere Anforderungen im Kanal. Zehnmal
/// hintereinander abzugleichen liefert zehnmal dasselbe Ergebnis und kostet
/// nur API-Kontingent, deshalb bleibt von mehreren `Sync` genau einer uebrig.
/// Erledigungen und Neuanmeldungen sind dagegen jede fuer sich bedeutsam und
/// bleiben vollstaendig erhalten.
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
                // Der volle Text passt nicht in die Statuszeile — ins
                // Protokoll gehoert er trotzdem.
                log::error(&format!("Sync fehlgeschlagen: {e}"));
                // Bewusst *keine* Daten verwerfen: ein WLAN-Aussetzer soll das
                // Widget nicht leerraeumen, nur die Statuszeile faerben.
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
// Damit beim Start sofort etwas dasteht statt einer leeren Flaeche, bis der
// erste Netzabruf durch ist.

pub fn read_cache() -> Option<Agenda> {
    let raw = std::fs::read_to_string(config::cache_path()).ok()?;
    let agenda: Agenda = serde_json::from_str(&raw).ok()?;
    // Ein Stand von gestern ist wertlos und waere sogar irrefuehrend.
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
