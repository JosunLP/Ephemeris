# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.2] - 2026-08-07

### Fixed

- Google tasks due **today** never appeared, while overdue ones did — which
  turns the task section into a list of nothing but arrears, the opposite of
  what it is for. The request bounded `dueMax` at the end of today, so a task
  due today sat exactly on the bound while everything overdue sat safely below
  it. Google documents neither whether that bound is inclusive nor what it does
  with the time of day, and a bound on the very day being asked about has no
  margin for either answer. It now clears that day by a full day. Nothing extra
  reaches the screen: the server side filter was only ever there to keep the
  bulk of future tasks off the wire, and which tasks are shown has always been
  decided on the client.
- A due date that carries a time of day was filed a day early east of UTC. Such
  a value is a real instant, not a calendar day, and it was read as the first
  ten characters of the timestamp — so a task due half past midnight in Central
  Europe arrives as `22:30Z` the evening before and was shown as overdue since
  yesterday. Timestamps with a time of day are now converted into the local
  zone. A plain due date still is not: it always arrives as midnight, and
  converting that is what put tasks on the wrong day to begin with.

### Documentation

- Troubleshooting explains why a task created from Google Calendar can show up
  under *Schedule* rather than under *Tasks*. Such a task carries two
  independent dates — a time block, which is a genuine calendar entry, and a
  separate deadline, which is the due date the task list works from. Leaving
  the deadline empty leaves the task undated, and undated tasks are hidden
  unless `show_undated_tasks` says otherwise.

### Internal

- Both defects had been reasoned about in comments rather than tested, and the
  reasoning was wrong in each case — the `dueMax` bound was documented as
  working "whether Google treats the bound as inclusive or exclusive", which is
  precisely what it did not do. The bound and the parsing of a due date are now
  covered by tests, and the ones for the parsing take the time zone as a
  parameter so the cases east and west of UTC do not depend on where the test
  machine stands.
- The remaining German comments and the manifest are in English, so the whole
  source reads in one language.

## [1.0.1] - 2026-08-07

### Fixed

- The installer rejected an intact download as a checksum mismatch, which made
  the one-line install fail on every machine. GitHub serves release assets as
  `application/octet-stream`, and for a non-text content type PowerShell does
  not hand back the text of the file from `Invoke-WebRequest`: PowerShell 7
  returns a byte array and Windows PowerShell 5.1 returns an empty string, so
  the published checksum was never actually read. It is now fetched to a file
  and read from there. A checksum that cannot be read is also reported as
  such, rather than as a mismatched binary.
- Turning autostart on did nothing at all on a profile that had never had an
  autostart entry, and the installer aborted at the same step — after the
  binary was already in place. The `CurrentVersion\Run` key is not one Windows
  guarantees: it comes into being when something first registers for autostart,
  and writing a value into a key that is absent fails. The installer and the
  widget's own setting both create it now.
- The installer's closing hint printed as `connect a calendar â€" right-click`
  on stock Windows. The script carries no byte order mark, so Windows
  PowerShell 5.1 reads it as the system ANSI codepage, and the one-line install
  pipes it out of an HTTP response that declares no character set. Both scripts
  are plain ASCII now, and a check keeps them that way.

The 1.0.0 binaries are intact and their published checksums are correct and
unchanged: the checksum and encoding defects belonged to the installer that
verified them, not to what it installed. The 1.0.0 release has had its
`install.ps1` asset replaced with the fixed one, so pinning to that version
installs as well. The autostart defect is the exception — it is in the 1.0.0
binary too, and only 1.0.1 has it fixed.

### Internal

- The checksum and encoding defects were invisible to the test suite, because
  they only appear when the script is run against real assets over real HTTP.
  CI now installs, verifies and uninstalls for real on every change, under
  Windows PowerShell 5.1 as well as PowerShell 7, and the release workflow
  repeats it on both architectures against the assets actually attached to the
  release. The scripts are also linted and checked for non-ASCII characters.
- The autostart defect hid behind a machine that already had the key, which
  every developer machine and every CI runner does. The smoke test therefore
  takes the Run key away before it installs, and puts it back afterwards, so
  the installer has to create one. `set_autostart` is driven against a scratch
  key that is known to be absent, because emptying the real one would mean
  deleting whatever the machine starts at logon.

## [1.0.0] - 2026-08-07

First public release.

### Calendars and tasks

- **Google** Calendar and Google Tasks.
- **Microsoft Graph**: Outlook calendars, Microsoft To Do, and Teams meetings.
  A Teams meeting is an ordinary Outlook event carrying a join link, which the
  widget surfaces so a click joins the meeting instead of opening the entry.
- **CalDAV**: iCloud, Nextcloud, Fastmail, Synology, mailbox.org and anything
  else speaking the standard. Recurring events are expanded by the server.
- Several accounts at once, queried **in parallel**. One failing account never
  blanks the widget; its error is reported next to the healthy accounts' data.
- Tasks are filtered to those due today or earlier and sorted by due date.

### Interface

- Frameless glass panel with rounded corners, a soft shadow and per-pixel
  alpha, sitting below all normal windows but above the desktop.
- Day rail showing the shape of the day: one segment per event, in its calendar
  colour, with a marker for the current time.
- Hero card for the running or next event, with a countdown and progress.
- Now line between past and upcoming events.
- Conflict detection for overlapping events.
- Tomorrow preview once today is done.
- Tooltip for titles that had to be truncated.
- Undo window before a completed task is sent to the service.
- Drag to move, drag an edge to resize, mouse wheel to scroll.
- Global shortcut brings the widget to the front for a few seconds.

### Localisation and appearance

- Interface text in English, German, French, Spanish and Italian.
- Dates, times and reading direction come from the operating system, so they
  are correct in every locale — including 12- against 24-hour clocks and
  right-to-left layouts.
- Follows the system light and dark appearance, the accent colour, contrast
  themes, and the transparency and animation settings.

### Operations

- One-line installer and uninstaller, update check with an in-app prompt.
- File log with rotation; panics are recorded before the process ends.
- Settings are reloaded without a restart.

[1.0.2]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.2
[1.0.1]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.1
[1.0.0]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.0
