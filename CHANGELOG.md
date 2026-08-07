# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

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

The 1.0.0 binaries were never affected; only the installer that verified them
was. Their published checksums are correct and unchanged.

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

[1.0.1]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.1
[1.0.0]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.0
