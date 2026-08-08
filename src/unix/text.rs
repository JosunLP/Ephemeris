// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! A text front end for macOS and Linux.
//!
//! Not the widget. The widget is a window that sits below every other window
//! and above the desktop, and that window has not been written for these
//! platforms yet — see `docs/development/porting.md`.
//!
//! What this is instead: the whole portable half, running. It loads the
//! settings, resolves the locale, drives the same sync thread the Windows
//! front end drives, and prints the agenda the renderer would have drawn. Two
//! reasons that is worth having rather than a stub that exits:
//!
//! * It makes the split verifiable. Continuous integration builds and runs
//!   this on Ubuntu and macOS, so "the core is portable" stops being a claim
//!   about the crate that compiles and becomes one about the program that
//!   runs.
//! * It gives whoever writes the real front end a working data path to render
//!   against, and something to compare against when the drawing is wrong.

use std::io::{self, Write};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tpmplaner_core::config::{self, Config};
use tpmplaner_core::host::Waker;
use tpmplaner_core::i18n::{self, Locale};
use tpmplaner_core::model::{Agenda, Event, Task};
use tpmplaner_core::sync::{self, Command, Shared, Status};
use tpmplaner_core::{demo, log};

/// How long to wait for the first sync before printing what is already known.
///
/// Generous: several accounts are queried in parallel and a slow CalDAV server
/// is not an error. The cached agenda is printed either way, so this bounds
/// how long the command takes, not whether it produces anything.
const SYNC_TIMEOUT: Duration = Duration::from_secs(90);

pub fn run() -> Result<(), String> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match print_everything(&mut out) {
        Ok(()) => Ok(()),
        // `tpmplaner | head` closes the pipe as soon as it has seen enough,
        // which is an ordinary thing to do and not a failure. Left alone it
        // would be a panic, and the panic hook would put it in the very log
        // users are asked to attach to bug reports.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn print_everything(out: &mut impl Write) -> io::Result<()> {
    let (cfg, config_error) = Config::load();
    let loc = Locale::resolve(&cfg.language);
    // The sync thread reaches for the global catalogue; it has no access to
    // this one.
    i18n::set_global(loc.cat);

    writeln!(out, "TPMPlaner {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(
        out,
        "The graphical widget is not ported to this platform yet. This is the \
         portable core: same settings, same calendars, same agenda, printed."
    )?;
    writeln!(out, "Settings: {}", config::config_path().display())?;
    if let Some(e) = &config_error {
        writeln!(out, "\n  ! {e}")?;
        log::warn(e);
    }
    writeln!(out)?;

    let agenda = if demo::enabled() {
        writeln!(out, "Demo mode — sample data, nothing is fetched.\n")?;
        demo::agenda()
    } else {
        fetch(out, cfg, config_error, &loc)?
    };
    print_agenda(out, &agenda, &loc)
}

/// Runs one sync through the same thread the Windows front end uses.
///
/// Deliberately not a shortcut past it: the schedule, the backoff, the
/// parallel fetch and the cache are the parts most worth exercising on a
/// platform nobody has run them on.
fn fetch(
    out: &mut impl Write,
    cfg: Config,
    config_error: Option<String>,
    loc: &Locale,
) -> io::Result<Agenda> {
    let shared = Arc::new(Mutex::new(Shared {
        // The last known state, so a failed sync still has something to show.
        agenda: sync::read_cache().unwrap_or_default(),
        status: Status::Syncing,
        config: cfg,
        config_error,
        calendars: Vec::new(),
        tasklists: Vec::new(),
        update: None,
    }));

    // `sync_channel(1)` rather than an unbounded one: the waker fires for
    // status changes as well as for finished work, and a notification already
    // waiting to be read is exactly as good as another one.
    let (tx, rx) = sync_channel::<()>(1);
    let handle = sync::spawn(shared.clone(), Arc::new(ChannelWaker(tx)));
    handle.send(Command::Sync);
    writeln!(out, "{}", loc.label(loc.cat.syncing))?;

    // Wait for the status to leave `Syncing` rather than for the first wake:
    // switching *to* `Syncing` is itself a wake.
    let deadline = Instant::now() + SYNC_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            writeln!(
                out,
                "  ! Timed out after {}s — showing what is cached.",
                SYNC_TIMEOUT.as_secs()
            )?;
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(()) => {}
            // Nothing to do: the deadline is re-checked at the top.
            Err(RecvTimeoutError::Timeout) => continue,
            // The worker dropped its waker, so it has stopped — panicked, most
            // likely, which is the case `sync::lock` recovers a poisoned mutex
            // for. A disconnected channel returns *immediately*, so treating
            // this as a timeout would spin a core flat out until the deadline
            // and then blame a timeout that never happened.
            Err(RecvTimeoutError::Disconnected) => {
                writeln!(
                    out,
                    "  ! Sync stopped unexpectedly — showing what is cached."
                )?;
                break;
            }
        }
        if !matches!(sync::lock(&shared).status, Status::Syncing) {
            break;
        }
    }

    let guard = sync::lock(&shared);
    if let Status::NeedsSetup(m) | Status::NeedsLogin(m) | Status::Error(m) = &guard.status {
        writeln!(out, "  ! {m}")?;
    }
    let agenda = guard.agenda.clone();
    drop(guard);

    handle.send(Command::Quit);
    writeln!(out)?;
    Ok(agenda)
}

/// Wakes this front end by dropping a token in a channel.
///
/// The Windows front end posts a window message here; the point of the trait
/// is that the sync thread cannot tell the difference.
struct ChannelWaker(SyncSender<()>);

impl Waker for ChannelWaker {
    fn wake(&self) {
        let _ = self.0.try_send(());
    }
}

fn print_agenda(out: &mut impl Write, agenda: &Agenda, loc: &Locale) -> io::Result<()> {
    let today = agenda
        .day
        .unwrap_or_else(|| chrono::Local::now().date_naive());
    writeln!(out, "{}, {}", loc.weekday(today), loc.date_line(today))?;
    writeln!(out)?;

    writeln!(out, "{}", loc.label(loc.cat.section_events))?;
    if agenda.events.is_empty() {
        writeln!(out, "  {}", loc.label(loc.cat.no_events))?;
    }
    for event in &agenda.events {
        writeln!(out, "  {}", event_line(event, loc))?;
    }

    if !agenda.tomorrow.is_empty() {
        writeln!(out)?;
        writeln!(out, "{}", loc.label(loc.cat.tomorrow))?;
        for event in &agenda.tomorrow {
            writeln!(out, "  {}", event_line(event, loc))?;
        }
    }

    writeln!(out)?;
    writeln!(out, "{}", loc.label(loc.cat.section_tasks))?;
    if agenda.tasks.is_empty() {
        writeln!(out, "  {}", loc.label(loc.cat.no_tasks))?;
    }
    for task in &agenda.tasks {
        writeln!(out, "  {}", task_line(task, today, loc))?;
    }

    writeln!(out)?;
    match agenda.fetched_at {
        // The second placeholder is the next scheduled sync, and this front
        // end has none: it runs once and exits.
        Some(at) => writeln!(out, "{}", loc.updated_next(&loc.time(at), "—"))?,
        None => writeln!(out, "{}", loc.label(loc.cat.not_synced))?,
    }
    if let Some(e) = &agenda.last_error {
        writeln!(out, "  ! {e}")?;
    }
    Ok(())
}

fn event_line(event: &Event, loc: &Locale) -> String {
    // The first column is as wide as the widest thing that goes in it, which
    // is the all-day label rather than any time.
    let when = match (event.all_day, event.start) {
        (true, _) | (_, None) => loc.label(loc.cat.all_day),
        (false, Some(start)) => loc.time(start),
    };
    let mut line = format!("{when:>9}  {}", event.title);
    if let Some(place) = &event.location {
        line.push_str(&format!("  ({place})"));
    }
    line
}

fn task_line(task: &Task, today: chrono::NaiveDate, loc: &Locale) -> String {
    let due = match task.due {
        // Overdue, which the widget marks with colour and this cannot.
        Some(d) if d < today => format!("{}!", loc.day_month(d)),
        Some(d) => loc.day_month(d),
        None => String::new(),
    };
    let indent = "  ".repeat(task.depth as usize);
    format!("{due:>9}  {indent}{}", task.title)
}
