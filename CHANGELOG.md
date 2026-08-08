# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The binary builds and runs on macOS and Linux. Not the widget — the window
  that sits below every other window and above the desktop has not been written
  for those platforms yet — but the whole portable half: settings, locale,
  accounts, the same sync thread the Windows front end drives, and the agenda
  printed instead of drawn. `TPMPLANER_DEMO=1` works there too.
- A `Host` for macOS and Linux. The data directory follows each platform's
  convention and is created readable by its owner alone, a browser is opened
  through `open` or `xdg-open`, and random bytes come from `/dev/urandom`.
  Credential storage is not implemented and says so plainly: the Keychain and
  the Secret Service are still to be written, and pretending to encrypt is
  worse than not encrypting.
- `Host::random_bytes` returns `Option`. It is the one method whose portable
  fallback was actively unsafe — it returned zeros, and a PKCE verifier of
  zeros is no verifier at all. There is no shorter buffer that fails closed
  either, because the callers encode whatever comes back, and an empty `state`
  would be compared against a callback's empty `state` and match. Refusing is
  the only safe answer, and refusing in the return type means the sign-in
  fails rather than the process: opening `/dev/urandom` has transient failure
  modes — `EMFILE`, `ENFILE` — that say nothing about the randomness, and with
  `panic = "abort"` a passing spike would have taken the whole widget down.
- `docs/development/porting.md`: what a front end has to provide, and the
  decisions taken before the code that depends on them. No cross-platform
  toolkit, and why. macOS first, with the API for each piece named. On Linux,
  X11 and `wlr-layer-shell` — and under GNOME, which implements neither, the
  widget will say that stacking below other windows is unavailable rather than
  quietly becoming an ordinary window that has stopped doing the one thing it
  is for.
- Fifteen more interface languages: Portuguese (European and Brazilian), Dutch,
  Swedish, Polish, Czech, Turkish, Russian, Ukrainian, Japanese, Simplified and
  Traditional Chinese, Korean, Arabic and Hebrew. Twenty catalogues in total.
  Dates, times and reading direction came from the operating system already, so
  an unlisted locale was never broken — but the labels a user actually reads
  were English, and a permanent strip of English in an otherwise localised
  desktop is exactly the friction this widget exists to avoid.
- Arabic and Hebrew are the first catalogues with `rtl` set, so the layout
  mirroring that `ar-SA` has always produced now agrees with the text inside
  it. The bidi isolation brackets that keep "32 min left" from rearranging
  itself are applied only while the labels are still Latin, which is what they
  were written for and what a Persian or Urdu locale still gets.
- Superseded ISO 639 codes are rewritten before the tag reaches the platform:
  `iw` becomes `he`, `in` becomes `id`, `ji` becomes `yi`. The catalogue could
  always read the old codes, but Windows cannot — it rejects `iw` outright —
  so reading direction, date formatting and DirectWrite's script shaping each
  fell back to a neutral default, and Hebrew came out laid left to right.
- Counted messages carry their language's plural rule. English gets by with a
  comparison against one; Russian needs three forms and has to look at the last
  two digits, Polish draws the same boundaries differently, Arabic has six
  categories and Hebrew a dual, while Chinese, Japanese, Korean and Turkish
  leave the noun alone entirely. The rule and its forms are one choice in the
  catalogue rather than two that can drift apart, and the forms for one and two
  may spell the numeral out — "تعارض واحد" reads better than "1 تعارض".
- `appearance` in `config.json`: customisation on top of what the system
  decides. Colours beyond the accent — panel, text, muted text, separator, and
  the semantic `now`, `overdue` and `conflict` — plus a font family, a size
  offset independent of `scale`, a weight for the section headers, a surface
  style (`aero`, `flat`, `borderless`), a layout density and per-calendar
  colour overrides. Everything defaults to `"system"`, and everything left
  there keeps being derived exactly as before. Following the system is the
  right default for a widget that should feel like part of it, but a wallpaper
  the panel disappears into, a glass surface next to an otherwise flat desktop
  or provider colours that are indistinguishable at 82% opacity are not
  answered by `"theme": "contrast"`, which is all or nothing.
- A whole set of choices can live in one shareable file: `"appearance":
  "midnight"` reads `midnight.theme.json` next to `config.json`, rather than
  growing another pile of top-level keys. A theme name is reduced to letters,
  digits, spaces, hyphens and underscores, so it names a file in the data
  directory and cannot become a path to one somewhere else.
- Only the colours a person reasons about are exposed; the shades between them
  are derived from those, so the set stays coherent. A custom panel colour
  keeps the gradient's shape by reproducing the built-in lightness spread
  around it, and if it crosses into the other appearance — a near-white panel
  while the dark theme is active — the text, separators and edges follow it. A
  white separator on light glass is invisible whatever the theme was called.

### Changed

- The front end is behind a platform boundary. `src/main.rs` calls four
  functions — `install_host`, `acquire_single_instance`, `run`, `fatal` — and a
  `#[cfg]` decides which module supplies them, so the Windows code moved to
  `src/win` and the new one lives in `src/unix`. Adding a platform is adding a
  directory rather than threading conditionals through the program. The Win32
  bindings became a Windows-only dependency at the same time, so they are not
  compiled at all elsewhere.
- Continuous integration builds the whole workspace on Ubuntu and macOS, not
  only the core crate, and runs the resulting binary in demo mode in two
  languages. "The core is portable" was a claim about a crate that compiles; it
  is now a claim about a program that runs.
- Accessibility keeps winning. A contrast theme ignores the custom colours and
  the surface style, because its colours come from the system and its flatness
  is the point; typography and density still apply, since nothing about a
  larger font or more room works against contrast. Custom colours are corrected
  until they read against the surface they sit on — the same bargain the accent
  has always struck, moving the lightness and leaving the hue — so a settings
  file cannot produce invisible text. Every correction, malformed colour and
  missing theme file is written to the log, because otherwise it is
  indistinguishable from a setting that had no effect.

### Fixed

- The agenda cache is replaced by rename rather than truncated and rewritten in
  place, so nothing can read it half written. Two copies of the program can now
  overlap — the Unix single-instance check is still a stub, and a timer firing
  over a slow sync is enough — and the loser of that race used to leave a
  truncated file behind. It parsed as no cache at all, which is safe but throws
  away the day the cache exists to carry across a restart.
- Waiting for the first sync no longer spins a core if the sync thread stops.
  A dropped channel returns from `recv_timeout` immediately rather than
  blocking, so treating it like a timeout meant looping flat out for the full
  ninety seconds and then reporting a timeout that never happened. It now says
  the sync stopped and prints what is cached.
- Scratch files from the cache write are cleaned up whichever step failed, not
  only a failed rename, and any an earlier run left behind are swept at
  start-up. The name carries the writing process's id, so nothing later ever
  reused one: a full disk, or a process killed between the write and the
  rename, left one more file in the data directory on every restart.
- The browser opener is reaped. Rust installs no `SIGCHLD` handler, so dropping
  the `Child` detached the handle without collecting the process, and a front
  end that runs for days accumulated one zombie per opened URL.
- Text was laid out as though it were German whatever the language: the
  renderer passed a hard-coded `de-DE` to DirectWrite. That name is what
  selects between the Han glyph shapes a single code point has in Japanese,
  Simplified and Traditional Chinese, and it decides where a line may break, so
  the new CJK catalogues would have been drawn legibly and visibly wrong. The
  configured locale is used now, validated first so a typo in `config.json`
  cannot stop the widget from starting, and a change of language rebuilds the
  text formats the same way a change of scale does.
- With `"backdrop": "acrylic"` the panel was drawn inset by the drop shadow's
  margin inside a window that had deliberately not reserved one, losing 14
  device independent pixels on each side. The renderer derived its own metrics
  from the scale factor while the window derived its geometry from the
  backdrop as well, and the two disagreed in exactly that case. The renderer is
  handed the metrics the window used, and the shadow is skipped when there is
  no margin to draw it in — twelve rectangles that cannot grow outwards are not
  a soft edge but twelve coats of black over the panel.
- A named theme file is watched alongside `config.json`. Editing
  `midnight.theme.json` is the advertised way to use `"appearance": "midnight"`,
  and only the settings file was checked for changes — so the edit did nothing
  until `config.json` happened to be written for some unrelated reason.
- A colour no longer rebuilds the renderer. Only the half of a customisation
  that is fixed at construction — the font family, weight and size offset, the
  density and the surface style — needs the Direct3D and Direct2D pipeline torn
  down and the window resized. Colours are uploaded per frame, so nudging
  `colors.now` in a theme file repaints instead of flickering the whole panel on
  every save.
- A contrast theme ignores the surface style in the window geometry, not only in
  the palette. `"surface": "borderless"` dropped the shadow margin from the
  window while the palette — which returns before it reaches the surface under
  contrast — still drew the shadow and the border into it.
- The note explaining that a panel colour crossed into the other appearance
  named the wrong theme. It read the theme after switching to it, so a near-white
  panel under the dark theme reported itself "lighter than the light theme
  expects", which is the opposite of what happened.
- Corrections are logged when the system contrast or theme changes, instead of
  being suppressed as repetition. They are not repetition: whether custom
  colours are ignored at all depends on contrast, and what has to be corrected
  to stay readable depends on the background — so the note saying the custom
  colours are being ignored was the one going missing.

### Internal

- The consistency checks are driven by the list of shipped catalogues instead
  of a list written out in the test, so a language cannot be added without
  being checked. They walk every field through a destructuring that fails to
  compile if a field is added and not listed, and they enforce a width budget
  for the labels that sit in a fixed column — a translation that is correct but
  too long arrives on screen as an ellipsis. The budget counts East Asian and
  Hangul characters as two columns, because they are drawn that way.
- `CONTRIBUTING.md` says what a translation pull request is expected to
  contain: who checked the text, the plural variant the language actually uses,
  and strings that fit.
- The last German comments and assertion messages are gone — two comments in
  `src/win/window.rs`, an `unreachable!` in `crates/core/src/theme.rs` and the
  two test messages beside it. The 1.0.2 entry below already claimed the source
  reads in one language; it does now. The German words still in the source are
  content rather than prose about it: the `de` catalogue, the demo data, and
  `"dunkel"` accepted alongside `"dark"` in `config.json`, which is a setting
  users have already written and not ours to invalidate.

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

[Unreleased]: https://github.com/JosunLP/TPMPlaner/compare/v1.0.2...HEAD
[1.0.2]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.2
[1.0.1]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.1
[1.0.0]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.0
