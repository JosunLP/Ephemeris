# Using the widget

## Reading the panel

| Area | What it tells you |
|---|---|
| **Header** | Weekday, date and the current time |
| **Day rail** | The whole day on one strip: a coloured segment per event in its calendar colour, the elapsed part tinted, and a marker for right now. Shows the shape of the day — clustered mornings, free afternoons — which a list cannot. |
| **Hero card** | The meeting running now, or the next one, with a countdown and a progress bar |
| **Schedule** | Today's events. Past ones dimmed, the running one highlighted, remaining time on the right |
| **Now line** | Separates what has passed from what is coming, labelled with the time |
| **Tomorrow** | The first two events of tomorrow, once today is done |
| **Tasks** | Due today or earlier, oldest first, overdue in a distinct colour |
| **Status line** | Last sync and next sync, or whatever needs your attention |

The day rail adapts: the default window is 06:00 to 22:00, but it stretches to
cover an early flight or a late event rather than squashing them against the
edge.

## Mouse

| Action | Result |
|---|---|
| Drag the panel | Move it |
| Drag an edge or corner | Resize it |
| Mouse wheel | Scroll the list |
| Click the circle before a task | Tick it off |
| Click a task row | Open it in the web interface |
| Click an event row | Open it in the calendar, or **join** if it is an online meeting |
| Hover a truncated title | See it in full |
| Right-click | Context menu |

## Ticking off a task

A click on the circle marks the task done immediately, but nothing is sent for
four seconds. During that window the row shows **Undo** with a countdown; click
anywhere on it to take it back, and nothing ever reaches the service.

A click on a 14-pixel circle happens by accident on a desktop, which is exactly
why the grace period exists. Set `undo_seconds` to `0` if you would rather send
straight away.

## Bringing it forward

The widget sits below every normal window, which is what keeps it out of the
way — and also means you cannot see it while working. Press **Ctrl+Alt+Shift+K**
and it comes to the front for five seconds, then sinks back on its own. No
click, no focus change.

If that combination is taken by something else, the widget falls back to another
and records which one in the log. Set your own with `peek_hotkey`.

## Conflicts

Overlapping meetings are counted in the section header and their times are
tinted. Back-to-back meetings — 10:00–11:00 followed by 11:00–12:00 — are not a
conflict and are not flagged.

## Copying the day

Right-click → **Copy agenda** puts the whole day on the clipboard as plain
text, ready for an email, a ticket, or a screen reader.

## Context menu

| Entry | What it does |
|---|---|
| Sync now | Fetch immediately |
| Start with Windows | Toggle autostart |
| Calendars / Task lists | Choose which sources to show |
| Copy agenda | The day as text on the clipboard |
| Edit configuration | Open `config.json` |
| Reset position | Back to the top right corner |
| Open log | For when something is wrong |
| Open data folder | Where credentials and settings live |
| Sign in to Google again | Discard credentials and re-authorise |
| Install update | Appears only when a newer version exists |

## When to expect a sync

Every 30 minutes by default, and additionally:

- when you click the refresh symbol
- on waking from standby
- at midnight, when the day rolls over
- after a change of system time or time zone

On failure the widget retries with a growing delay — one minute, two, four —
capped at the normal interval, so a long outage does not hammer the API. The
last known day stays on screen throughout; only the status line changes colour.
