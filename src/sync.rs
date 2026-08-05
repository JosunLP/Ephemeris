//! Hintergrund-Sync in einem eigenen Thread.
//!
//! Der UI-Thread besitzt die gesamte Zeitplanung (WM_TIMER) und schickt
//! Befehle herein; dieser Thread fuehrt sie aus, schreibt das Ergebnis in den
//! gemeinsamen Zustand und weckt das Fenster per `PostMessage`. Damit
//! blockiert kein Netzwerkaufruf jemals das Zeichnen.

use crate::config::{self, Config};
use crate::google::calendar::CalendarRef;
use crate::google::tasks::TaskListRef;
use crate::google::{self, Error, auth::Auth};
use crate::log;
use crate::model::{
    Agenda, Event, Task, filter_tasks_for_today, local_day_start, sort_events, sort_tasks,
};
use chrono::{Duration as ChronoDuration, Local};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Kalender- und Aufgabenlistenverzeichnis aendern sich hoechstens ein paarmal
/// im Jahr. Sie bei jedem Lauf neu zu holen kostet zwei von rund zehn
/// Anfragen pro Sync, ohne je etwas Neues zu liefern.
const META_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Wird nach jedem abgeschlossenen Befehl an das Fenster gepostet.
pub const WM_APP_SYNC_DONE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
/// Signalisiert nur "Statuszeile hat sich geaendert" (z. B. Sync gestartet).
pub const WM_APP_STATUS: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;

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
}

/// Sperrt den geteilten Zustand und ueberlebt eine Vergiftung.
///
/// `Mutex::lock` liefert `Err`, sobald irgendein Thread waehrend des Haltens
/// gepanickt ist — ab dann wuerde jedes `.unwrap()` die naechste Panik
/// ausloesen und aus einem lokalen Fehler einen Totalausfall machen. Der
/// Inhalt ist hier reine Anzeigedaten; im schlimmsten Fall ist ein Feld
/// halb geschrieben, was der naechste Abgleich ohnehin korrigiert.
pub fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Zwischengespeichertes Kalender-/Listenverzeichnis.
struct Meta {
    calendars: Vec<CalendarRef>,
    lists: Vec<TaskListRef>,
    at: Instant,
}

pub enum Command {
    /// Regulaerer Abgleich (Timer, Aufwachen, manueller Klick).
    Sync,
    /// Aufgabe abhaken; wird optimistisch sofort ausgeblendet.
    CompleteTask {
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

/// Startet den Sync-Thread. `hwnd` als `isize`, weil `HWND` nicht `Send` ist —
/// wir nutzen es ausschliesslich fuer `PostMessage`, was threadsicher ist.
pub fn spawn(shared: Arc<Mutex<Shared>>, hwnd: isize) -> SyncHandle {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("tpmplaner-sync".into())
        .spawn(move || worker(shared, hwnd, rx))
        .expect("Sync-Thread konnte nicht gestartet werden");
    SyncHandle { tx }
}

fn worker(shared: Arc<Mutex<Shared>>, hwnd: isize, rx: Receiver<Command>) {
    let mut auth: Option<Auth> = None;
    let mut meta: Option<Meta> = None;

    while let Ok(first) = rx.recv() {
        for cmd in coalesce(first, &rx) {
            match cmd {
                Command::Quit => return,

                Command::Sync => {
                    set_status(&shared, hwnd, Status::Syncing);
                    let result = ensure_auth(&mut auth)
                        .and_then(|a| run_sync(a, snapshot_config(&shared), &mut meta));
                    apply_sync_result(&shared, hwnd, result);
                }

                Command::Relogin => {
                    log::info("Neuanmeldung angefordert");
                    set_status(&shared, hwnd, Status::Syncing);
                    if let Some(a) = auth.as_mut() {
                        a.forget();
                    }
                    auth = None;
                    // Das Verzeichnis gehoert zum alten Konto.
                    meta = None;
                    let result = ensure_auth(&mut auth)
                        .and_then(|a| a.interactive_login().map(|_| a))
                        .and_then(|a| run_sync(a, snapshot_config(&shared), &mut meta));
                    apply_sync_result(&shared, hwnd, result);
                }

                Command::CompleteTask {
                    tasklist_id,
                    task_id,
                } => {
                    let outcome = ensure_auth(&mut auth)
                        .and_then(|a| google::tasks::complete_task(a, &tasklist_id, &task_id));

                    let mut guard = lock(&shared);
                    match outcome {
                        Ok(()) => {
                            // Endgueltig aus der Anzeige nehmen; der naechste
                            // regulaere Sync bestaetigt es ohnehin.
                            guard.agenda.tasks.retain(|t| t.id != task_id);
                            guard.status = Status::Idle;
                            write_cache(&guard.agenda);
                        }
                        Err(e) => {
                            log::error(&format!("Abhaken fehlgeschlagen: {e}"));
                            // Optimistisches Ausblenden zuruecknehmen.
                            if let Some(t) = guard.agenda.tasks.iter_mut().find(|t| t.id == task_id)
                            {
                                t.completing = false;
                            }
                            guard.status = status_for(&e);
                        }
                    }
                    drop(guard);
                    notify(hwnd, WM_APP_SYNC_DONE);
                }
            }
        }
    }
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

/// Beim ersten Bedarf laden. Fehlt `client_secret.json`, meldet das die
/// Statuszeile — ein erneuter Versuch beim naechsten Sync kostet nichts.
fn ensure_auth(slot: &mut Option<Auth>) -> google::Result<&mut Auth> {
    if slot.is_none() {
        *slot = Some(Auth::load()?);
    }
    Ok(slot.as_mut().unwrap())
}

fn snapshot_config(shared: &Arc<Mutex<Shared>>) -> Config {
    lock(&shared).config.clone()
}

fn run_sync(auth: &mut Auth, cfg: Config, meta: &mut Option<Meta>) -> google::Result<Agenda> {
    // Ohne gespeicherten Zugang zuerst einmalig den Browser-Flow durchlaufen.
    if !auth.has_refresh_token() {
        auth.interactive_login()?;
    }

    let started = Instant::now();
    let today = Local::now().date_naive();
    let tomorrow = today + ChronoDuration::days(1);
    let day_start = local_day_start(today);
    let midnight = local_day_start(tomorrow);
    // Zwei Tage in einem Abruf: derselbe Request liefert die Morgen-Vorschau
    // gratis mit, ein zweiter waere reine Verschwendung.
    let window_end = local_day_start(today + ChronoDuration::days(2));

    // Verzeichnis nur holen, wenn der Cache abgelaufen ist.
    let fresh = meta.as_ref().is_some_and(|m| m.at.elapsed() < META_TTL);
    if !fresh {
        *meta = Some(Meta {
            calendars: google::calendar::list_calendars(auth)?,
            lists: google::tasks::list_tasklists(auth)?,
            at: Instant::now(),
        });
    }
    let cached = meta.as_ref().expect("Verzeichnis wurde gerade gefuellt");

    // --- Kalender ---
    let calendars: Vec<_> = cached
        .calendars
        .iter()
        .filter(|c| cfg.calendar_ids.is_empty() || cfg.calendar_ids.contains(&c.id))
        .collect();

    let mut events = Vec::new();
    for cal in &calendars {
        events.extend(google::calendar::list_events(
            auth,
            cal,
            day_start,
            window_end,
            cfg.hide_declined,
        )?);
    }

    // Nach Tagen trennen. Ganztagestermine haben keine Startzeit; sie stammen
    // aus der Abfrage fuer heute, weil `list_events` sie nur im Fenster des
    // jeweiligen Tages liefert — sie bleiben daher bei den heutigen.
    let (mut events, mut next_day): (Vec<Event>, Vec<Event>) = events
        .into_iter()
        .partition(|e| e.start.map(|s| s < midnight).unwrap_or(true));
    sort_events(&mut events);
    sort_events(&mut next_day);

    // --- Aufgaben ---
    let lists: Vec<_> = cached
        .lists
        .iter()
        .filter(|l| cfg.tasklist_ids.is_empty() || cfg.tasklist_ids.contains(&l.id))
        .collect();

    let mut tasks: Vec<Task> = Vec::new();
    for list in &lists {
        tasks.extend(google::tasks::list_tasks(
            auth,
            list,
            today,
            cfg.show_undated_tasks,
        )?);
    }

    // Serverseitig gefiltert wurde nur bei `dueMax`; hier faellt in jedem Fall
    // alles Zukuenftige heraus.
    let mut tasks = filter_tasks_for_today(tasks, today, cfg.show_undated_tasks);
    sort_tasks(&mut tasks);

    log::info(&format!(
        "Sync ok: {} events today (+{} tomorrow) from {} calendar(s), {} tasks from {} list(s), {} ms{}",
        events.len(),
        next_day.len(),
        calendars.len(),
        tasks.len(),
        lists.len(),
        started.elapsed().as_millis(),
        if fresh {
            " (Verzeichnis aus Cache)"
        } else {
            ""
        },
    ));

    Ok(Agenda {
        day: Some(today),
        events,
        tomorrow: next_day,
        tasks,
        fetched_at: Some(Local::now()),
        last_error: None,
    })
}

fn apply_sync_result(shared: &Arc<Mutex<Shared>>, hwnd: isize, result: google::Result<Agenda>) {
    {
        let mut guard = lock(&shared);
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
    notify(hwnd, WM_APP_SYNC_DONE);
}

fn status_for(e: &Error) -> Status {
    match e {
        Error::NeedsSetup(m) => Status::NeedsSetup(m.clone()),
        Error::NeedsLogin(m) => Status::NeedsLogin(m.clone()),
        Error::Other(m) => Status::Error(m.clone()),
    }
}

fn set_status(shared: &Arc<Mutex<Shared>>, hwnd: isize, status: Status) {
    lock(&shared).status = status;
    notify(hwnd, WM_APP_STATUS);
}

fn notify(hwnd: isize, msg: u32) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
    }
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
