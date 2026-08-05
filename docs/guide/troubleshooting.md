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

Fixed, but worth knowing what it was: task APIs return a due date as a
timestamp at midnight UTC, and the time is meaningless. Reading it as a real
instant and converting to local time lands a day early everywhere east of UTC.
The widget compares calendar dates and never converts.

If you still see a task on the wrong day, the log plus the raw value from the
provider will settle it — please open an issue.

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

[Open an issue](https://github.com/JosunLP/TPMPlaner/issues/new/choose) with
the relevant log lines. Redact anything private — the log contains no event
titles, but it does contain account labels and error messages from your
provider.
