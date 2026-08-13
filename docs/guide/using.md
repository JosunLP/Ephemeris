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
| Drag the panel | Move it, unless it is locked |
| Drag an edge or corner | Resize it, unless it is locked |
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
and records which one in the log. Set your own with `peek_hotkey`; write the
Command key as `Cmd`, `Win` or `Super`, whichever your keyboard says.

**On Wayland the compositor owns every keyboard shortcut** and will not let a
program claim one — deliberately, and it is an improvement on X11. So bind a
key to `tpmplaner --peek` instead, which tells the widget already running to
come forward:

```
# Sway, in ~/.config/sway/config
bindsym $mod+k exec tpmplaner --peek
```

It works on X11 too, if you would rather your desktop owned the shortcut.

## Conflicts

Overlapping meetings are counted in the section header and their times are
tinted. Back-to-back meetings — 10:00–11:00 followed by 11:00–12:00 — are not a
conflict and are not flagged.

## Locking it in place

The widget sits below everything and is dragged by its empty space, so reaching
past it for a file on the desktop moves it by accident. Right-click →
**Lock position and size** pins it: the drag and the resize grips stop
responding and the resize cursor stops appearing.

Everything else goes on working — scrolling, ticking tasks off, opening events,
the whole menu. Right-click → **Unlock position and size** to let it go again.
The setting is written to `config.json` as `"locked"`, so it survives a
restart, and *Reset position* is greyed out while it is on because moving the
widget is exactly what the lock forbids.

One thing the lock does not prevent: if the monitor it was on is unplugged, the
widget still comes back to the primary screen. A lock that could strand it
somewhere invisible would be a trap rather than a convenience.

## Copying the day

Right-click → **Copy agenda** puts the whole day on the clipboard as plain
text, ready for an email, a ticket, or a screen reader.

## Context menu

| Entry | What it does |
|---|---|
| Sync now | Fetch immediately |
| Start at login | Toggle starting with the session |
| Calendars / Task lists | Choose which sources to show |
| Copy agenda | The day as text on the clipboard |
| Lock / Unlock position and size | Pin the widget where it is, or let it go |
| Edit configuration | Open `config.json` |
| Reset position | Back to the top right corner. Greyed out while locked |
| Open log | For when something is wrong |
| Open data folder | Where credentials and settings live |
| Sign in to Google again | Discard credentials and re-authorise |
| Install update | Appears only when a newer version exists |

The same entries on all three systems, and the same effect — only the way the
menu is drawn differs. On Linux the widget draws its own, because X11 has none
and the widget carries no toolkit; submenus open in place with a row back to
the top rather than hovering beside it.

## When to expect a sync

Every 30 minutes by default, and additionally:

- when you click the refresh symbol
- on waking from standby
- at midnight, when the day rolls over
- after a change of system time or time zone

On failure the widget retries with a growing delay — one minute, two, four —
capped at the normal interval, so a long outage does not hammer the API. The
last known day stays on screen throughout; only the status line changes colour.
