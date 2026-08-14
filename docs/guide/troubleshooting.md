# Troubleshooting

## Start with the log

Right-click the widget → **Open log**. It records every sync, every failure
with its full text, and every panic. The status line only has room for about
one sentence; the log has the rest.

```text
2026-08-06 09:12:04 INFO  Sync ok: 6 events today (+2 tomorrow), 6 tasks, 2/2 accounts, 412 ms
2026-08-06 09:12:04 WARN  Account 'Work': token expired
```

---

## "Setup required"

No credentials for at least one account. See
[connecting accounts](/guide/accounts). Clicking the status line opens the data
directory.

## It asks me to sign in every week (Google)

Your Google Cloud project is still in **Testing**. Google expires refresh
tokens after seven days in that state.

Set the publishing status to *In production* under **OAuth consent screen**.
The unverified-app warning that appears once afterwards is expected for a
personal project.

## "Invalid configuration"

`config.json` has a syntax error and the defaults are in use. Click the status
line to open the file; the log names the line. A trailing comma is the usual
cause.

## The shortcut does nothing

Another application or Windows itself has claimed it. `Win+Alt+K`, for example,
is the Windows 11 microphone mute.

The widget tries alternatives and logs which one it settled on:

```text
WARN  peek_hotkey 'Ctrl+Alt+K' is taken by another application; using Ctrl+Alt+Shift+K instead
```

Set your own with `peek_hotkey`.

## A grey box around the widget

Set `"backdrop": "none"` — this is the default for new installations. The
Windows system backdrop fills the entire window rectangle including the margin
reserved for the shadow, which shows up as a square box around the rounded
panel.

## The widget has disappeared

- **Behind other windows** — that is by design. Press the shortcut, or minimise
  everything.
- **After unplugging a monitor** — it recovers automatically on the next
  display change. If not, right-click → *Reset position*.
- **After an error** — check the log for a `PANIC` line. The widget records
  what went wrong before it exits.

Not running at all? Start it from the Start menu.

## Tasks appear on the wrong day

Fixed, but worth knowing what it was: task APIs return a plain due date as a
timestamp at midnight UTC, where the time is meaningless. Reading it as a real
instant and converting to local time lands a day early everywhere east of UTC.
Midnight is therefore read as the bare calendar day and never converted.

A due date that carries a *time* is the opposite case — that is a real instant,
and the local zone decides which day it falls on. The widget tells the two
apart by the time of day.

If you still see a task on the wrong day, the log plus the raw value from the
provider will settle it — please open an issue.

## A task shows up under Schedule instead of under Tasks

That is a Google task with a **time block**, and both halves of it are working
as intended.

Creating a task from Google Calendar, on the *Task* tab, offers two different
dates:

- **Start date and duration** — when you plan to work on it. That is a block on
  your calendar, so it arrives through the calendar API and the widget lists it
  under *Schedule*, at its time, like any other commitment.
- **Deadline** — the actual due date, a separate field further down the dialog.
  This is the one the tasks API reports as the due date, and it is what the
  *Tasks* section sorts and colours by.

Set a deadline and the task appears in both places: as a block in the timeline
and as a dated entry in the task list. Leave it empty and the task has no due
date at all, which means the *Tasks* section only shows it once you turn on
`show_undated_tasks` — see [configuration](/guide/configuration).

A task created in the Google Tasks app has no time block, only a due date, so
it appears under *Tasks* alone.

**Ticking it off clears both rows.** Neither API says the two halves belong
together — the calendar entry carries no task id and the task carries no event
id — so the widget pairs them by their title within one account. That is a
guess, and it is kept narrow: the whole title has to match, case and spacing
aside, and if two tasks on screen share a title neither is paired, because there
would be no telling which block belongs to which. An unpaired block stays where
it is and behaves like any other entry; you can still tick the task off, the
schedule row simply outlives it until the next sync.

## Times are hours off (Microsoft)

Graph returns timestamps without a UTC offset, with the zone in a separate
field. The widget asks for UTC and checks the reply. If the server ignored
that, the log says so:

```text
WARN  Graph returned time zone 'W. Europe Standard Time' despite the UTC preference
```

## Nothing syncs, but the network is fine

Behind a corporate proxy, the widget uses the Windows internet settings. If
those are not configured, nothing gets out. The log shows the connection error.

## High CPU usage

It should be near zero at rest. Sustained load means something is redrawing
continuously — please open an issue with the log; that is a bug.

Note that animation only runs while something is moving. If your system has
animations turned off, the widget skips them entirely.

## Still stuck

[Open an issue](https://github.com/JosunLP/Ephemeris/issues/new/choose) with
the relevant log lines. Redact anything private — the log contains no event
titles, but it does contain account labels and error messages from your
provider.
