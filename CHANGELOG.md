# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.0] - 2026-08-14

### Added

- **The widget runs on macOS and Linux.** A real window on both, with the
  behaviour the widget is defined by: below every normal window but above the
  desktop, no taskbar or window-switcher entry, never takes focus, and a global
  shortcut that brings it forward for a few seconds.

  On macOS that is an `NSWindow` at `kCGDesktopIconWindowLevel + 1` that
  refuses to become key, joins every Space, is skipped by the window switcher
  and has no Dock tile; Core Graphics draws it, Core Text sets it, and both go
  through the Objective-C runtime directly rather than a binding crate. The
  peek shortcut is `RegisterEventHotKey` and not an `NSEvent` global monitor —
  a monitor needs the accessibility permission, which means a system dialogue
  and a documented feature that does nothing until somebody grants it.

  On Linux it is an X11 window carrying `_NET_WM_STATE_BELOW`, `SKIP_TASKBAR`,
  `SKIP_PAGER` and `STICKY`, Motif hints for the missing frame, and
  `WM_HINTS.input = False` so a click on it never pulls focus out of what you
  are typing in. Cairo draws it and Pango sets it: Cairo's own text API is a
  documented "toy" interface with no shaping and no bidirectional reordering,
  and twenty catalogues with Arabic and Hebrew among them make that
  disqualifying. Xlib, Cairo and Pango are opened with `dlopen` rather than
  linked, so the tarball is one file with no development package to install and
  a machine with no display falls back to printing the agenda instead of
  refusing to start. The right-click menu is drawn by the widget, because X11
  has none and the widget carries no toolkit.

  **Wayland is native**, not XWayland. `wlr-layer-shell` is the only protocol
  that lets a client ask to sit above the wallpaper and below every ordinary
  window, and on the compositors that have it — Sway, Hyprland, river, Wayfire,
  KDE Plasma — the surface goes on the bottom layer and behaves exactly as the
  Win32 and AppKit windows do. Cairo draws into shared memory the compositor
  reads directly, two buffers deep, with no conversion step: Cairo's `ARGB32`
  and Wayland's `ARGB8888` are the same bytes.

  There is no code generator. `libwayland-client` exports the description of
  every core protocol object, so those are looked up like any other symbol;
  what is written by hand is only the layer shell and the four `xdg_shell`
  objects a popup menu needs. The two rules that makes safe — every request
  listed up to the last one used, because an opcode is a position, and a
  listener as long as its interface, because it is indexed by one — are written
  down where the tables are.

  **On GNOME the widget says what it cannot do.** Mutter has no layer shell and
  has said it will not, so there the surface is an ordinary `xdg_toplevel`: it
  sits among your windows rather than behind them, the compositor places it,
  and dragging is handed to the compositor with `xdg_toplevel.move` because
  Wayland gives no client the power to place its own window. All of that goes
  in the log at start-up. Quietly degrading was rejected in the porting notes
  and is still rejected.
- **The right-click menu can lock the widget's position and size.** It sits
  below everything and is dragged by its empty space, so reaching past it for a
  file on the desktop moves it by accident. *Lock position and size* pins it:
  the drag and the resize grips stop responding and the resize cursor stops
  appearing, while scrolling, ticking tasks off, opening events and the rest of
  the menu go on working. *Unlock position and size* lets it go again. Stored
  as `"locked"` in `config.json`, so it survives a restart, and *Reset
  position* greys out while it is on because moving the widget is exactly what
  the lock forbids. One thing it deliberately does not prevent: a widget left
  off every monitor still comes back, because a lock that could strand it
  invisibly would be a trap rather than a convenience.
- `tpmplaner --peek` brings the running widget forward, through the same socket
  that already makes it single-instance. This is how the peek shortcut works on
  Wayland at all: the compositor owns every keybinding and will not let a
  client grab one — deliberately, and an improvement on X11 — so a line like
  `bindsym $mod+k exec tpmplaner --peek` in the compositor's configuration is
  the shortcut. It works on X11 too, for anyone who would rather their desktop
  owned it.
- Where there is no desktop — a container, a server over SSH, a
  continuous-integration runner — the agenda is printed rather than drawn, and
  the same binary does both. `TPMPLANER_TEXT=1` asks for the printed form on a
  machine that does have a display, which makes it a usable command in its own
  right.
- Starting with the session on macOS and Linux: a launch agent in
  `~/Library/LaunchAgents`, or a `.desktop` file in
  `$XDG_CONFIG_HOME/autostart`. Both are plain files in the user's own home, so
  nothing is written outside the profile and removing the file is all it takes
  to undo — the promise the Windows installer already makes.
- `tpmplaner_core::menu` decides what the right-click menu contains — which
  entries appear, which are ticked, which are greyed out — for all three front
  ends, with its own tests. A `HMENU`, an `NSMenu` and a panel drawn with Cairo
  differ in how a menu is *shown*, not in what it says, and a command added to
  the core now fails to compile in every front end until each says what it
  does.
- `tpmplaner_core::hotkey` reads `peek_hotkey`. The spelling is the same
  everywhere and only the key *number* is not, so the parsing is shared and
  each platform maps a parsed combination to its own table. `Cmd`, `Win` and
  `Super` are all accepted for the same key, so one settings file works on
  every machine.
- Release artefacts for macOS and Linux: a tarball per target with the binary,
  the licence and the readme, and a SHA-256 beside it. No installer one-liner
  on either — inventing one that writes outside the package manager's view
  would make the widget a worse citizen than having none. The macOS archive
  also carries a `TPMPlaner.app`; it is **not** notarised, which needs a paid
  developer account, so Gatekeeper refuses it on first launch until the user
  right-clicks and chooses Open.
- `icu4x` was measured against the C library's long-date gap on Linux, which
  the porting notes had left open with a threshold rather than an answer. It
  costs 1.01 MB — sixty per cent onto the binary — and buys a better long date
  on one platform of three, because Windows has NLS and macOS has Core
  Foundation and both are already exact. Not taken, and now recorded as a
  measurement rather than a suspicion.
- A Flatpak manifest, a desktop entry and AppStream metadata, in `packaging/`.
  The runtime already carries Xlib, Cairo, Pango and libwayland, so the build
  is one binary and nothing else. A Flatpak build has no network, so every
  crate is listed as a source with the checksum `Cargo.lock` already holds —
  generated by `packaging/linux/cargo-sources.py`, which needs nothing but that
  file, and checked in continuous integration so it cannot go quietly stale.
  What is left before Flathub is the submission itself.
- Continuous integration builds the release binary on all three systems and
  smoke-tests both Linux windows for real: the X11 one under `xvfb`, and the
  Wayland one under a headless Sway, which is the only way to exercise the
  layer shell and is the half no compiler can check. Each is stopped after a
  few seconds and judged by its log. A `dlopen` failure, a wrong Xlib
  signature, a missing Pango symbol, a mis-numbered Wayland opcode and a panic
  in the first frame are all invisible to a compiler and all fatal there. The
  X11 run deliberately gets a server with no 32-bit visual and no compositing
  manager — the harder of the two cases, where the widget has to fall back to
  the default visual and draw opaque.
- The widget's behaviour and its drawing are each written once for the two new
  front ends, behind two traits: `Shell` is everything the widget needs from a
  window system — geometry, a cursor, three timers, a menu, the clipboard, the
  appearance settings — and `Canvas` is everything a renderer has to be able to
  draw. Core Graphics and Cairo are a few hundred lines each behind them. The
  Direct2D renderer predates both and still draws the same picture from its own
  code; that is one description of the interface too many, and
  `docs/development/porting.md` records it as such rather than leaving it to be
  discovered.
- A `Host` for macOS and Linux. The data directory follows each platform's
  convention and is created readable by its owner alone, a browser is opened
  through `open` or `xdg-open`, and random bytes come from `/dev/urandom`.
- Credentials go to the platform's keyring on macOS and Linux: the Keychain
  through `SecItemAdd`, the Secret Service through `secret-tool`. Neither
  encrypts a blob the way DPAPI does — they store a secret under a name — so
  the token goes into the keyring and what lands in `token.bin` is a reference
  to it. That is a better shape than the Windows one rather than a compromise:
  the token never reaches the file at all. Where there is no keyring, on a
  headless machine or in a container, the bytes go into the file as they were
  before and a warning says so once; pretending to encrypt would be worse than
  not encrypting, and so would refusing to start. A keyring that is there and
  says no is a different answer and gets a different one back: a locked
  keychain, a dismissed passphrase prompt, a daemon a login item started faster
  than are all true now and false in a minute, and the plain copy the fallback
  writes would not be. Nothing is written at all there, which costs one
  sign-in. A file written either way reads back either way.
- Dates and times on macOS and Linux come from the platform's own locale
  database. They used to read `4 August 2026` on a twenty-four-hour clock in
  every locale, which is right for nobody in particular and exactly the trap
  the design note in `i18n.rs` warns about. macOS asks Core Foundation, whose
  `CFDateFormatterCreateDateFormatFromTemplate` is `dateFormatFromTemplate:`
  under its C name; Linux asks the C library through `newlocale`/`uselocale`,
  so the locale belongs to the formatting thread rather than to the whole
  process. Whether a locale counts in twelve or twenty-four hours is read from
  its own pattern rather than guessed from the language — `en-GB` and `en-US`
  disagree about it. The long date on Linux is the honest gap: POSIX has no
  long-date pattern, so the short one is widened, keeping the locale's field
  order and separators.
- `tpmplaner_core::layout`: the arithmetic behind the drawing, moved out of the
  Windows renderer before a second one is written. Where a row starts, how wide
  the time column has to be, which rows are visible, what fades and by how
  much, where the now line goes, which rectangle a click landed in — the same
  answers on Direct2D, Core Graphics and Cairo, and written once so they cannot
  drift apart. Measuring text stays with the front end, since only it knows how
  wide a word is in its font.
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

- The autostart entry in the context menu is *Start at login* rather than
  *Start with Windows*, in all twenty languages. The same entry is a registry
  value on Windows, a login item on macOS and an XDG autostart file on Linux,
  and naming one of them was wrong on the other two.
- Today's agenda as text — the *Copy agenda* entry — is built in the core, so
  the three front ends copy the same day rather than three near-identical ones.
- `Frame`, `UndoView` and `FrameResult` moved to `tpmplaner_core::layout`
  beside the hit regions: every front end draws from exactly those and nothing
  else, and a field added for one of them would otherwise be a field the others
  silently do not draw.
- **There is one description of what the widget looks like.** The Direct2D
  renderer drew the same picture from its own code — two thousand lines that
  had to be kept in step with the shared drawing by hand, and which the porting
  notes recorded as one description of the interface too many. It now
  implements the same `Canvas` trait the other three do, and `src/paint` moved
  out from under the Unix front end to sit beside all four. `render.rs` went
  from 2083 lines to 550: the device chain, the fonts, and the three lines that
  begin a frame, hand it over and present it.

  The one visible change is the refresh symbol. It was `\u{E72C}` from Segoe
  Fluent Icons and is now the same drawn arc and arrowhead the other platforms
  use — which is what makes it the same widget rather than three that resemble
  each other.
- The right-click menu's rows, measurements and painting are shared by the two
  Linux back ends. Neither X11 nor Wayland has menus and the widget carries no
  toolkit, so it draws its own; what differs between them is the surface and
  the dismissal — an override-redirect window with a pointer grab, or an
  `xdg_popup`, which is the only object in Wayland that comes with a grab and
  a "the user clicked elsewhere" event.
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

- Ticking a task off now clears its time block from *Schedule* as well. A task
  given a start time in Google Calendar arrives twice — as a calendar entry
  through one API and as a dated task through another — and neither answer says
  the two are one item: the entry carries no task id, the task carries no event
  id, and `eventType` has six values of which none is a task. So the tick
  reached the row it was on and nothing else, and the block sat there for the
  rest of the day contradicting it. The two halves are paired on the one thing
  they share, the title, within a single account and only when exactly one
  visible task claims it — two tasks of the same name pair with nothing, because
  hiding the wrong row is worse than leaving both. A paired entry fades with the
  task during the undo window, is dropped with it once the completion goes out,
  and is kept off the next sync in case the provider still reports it. That last
  part could not be confirmed against a live account; the log says whenever it
  fires, and if that line never appears it can go.
- The agenda cache is replaced by rename rather than truncated and rewritten in
  place, so nothing can read it half written. Two writes can overlap — the text
  front end takes no single-instance lock, since it prints once and exits, and
  within one copy a timer firing over a slow sync is enough — and the loser of
  that race used to leave a truncated file behind. It parsed as no cache at all, which is safe but throws
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
- "Copy agenda" could do nothing at all, silently. Three separate things were
  wrong with one call. The clipboard was opened with a null window handle,
  which `EmptyClipboard` is documented to turn into a null owner and that is
  documented to make `SetClipboardData` fail — the widget's own window owns it
  now. `OpenClipboard` does not wait its turn but fails outright, and another
  application holding the clipboard for a few milliseconds while it copies
  something is ordinary, so it is retried ten times over 180 ms. And all five
  Win32 calls folded into one `false` with nothing logged, which is why the
  cause could only be guessed at: each now says which step failed and what
  Windows called it. The round-trip test creates a real message-only window rather than
  passing null, so it exercises the path the widget takes instead of the one
  that was wrong.
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
- The process exits with a failure status when it could not start, instead of
  reporting success after printing the reason. It never mattered while the only
  front end put the message in a message box nobody's shell was reading; it
  matters now that one of them is a command, where `tpmplaner || …`, a systemd
  unit and the smoke test all decide by the status. Another copy already owning
  the desktop stays a success, deliberately: nothing went wrong, this copy just
  has nothing to do.
- Only web links are handed to the platform's opener, on every front end.
  `ShellExecuteW` with the `open` verb, `open` on macOS and `xdg-open` on Linux
  do not browse — they launch whatever is registered for the scheme or the file
  type — and one of the things opened is an event's `htmlLink`, which arrives
  in the calendar server's JSON. A shared calendar somebody else can write to
  was therefore enough to turn a click on an agenda row into a UNC path or a
  `file:` link being executed. The rule lives beside `Host::open_url` so both
  front ends answer alike, and opening a local file — the settings, the data
  folder, the log — is now a separate function that says so, rather than the
  same one with a path squeezed through it.
- A failed "Copy agenda" no longer leaks the memory it allocated for the text.
  The clipboard takes ownership of that block only once `SetClipboardData` has
  succeeded, and the two ways out before that returned without freeing it. Kept
  company by the retry message, which reported the budget the constants
  describe rather than the time that actually elapsed: ten attempts leave nine
  gaps, so the wait is 180 ms, and that line exists to be held against a
  timestamp in a bug report.
- Writing the agenda cache says why it failed. Every step discarded its error,
  and the only symptom — a blank panel for a moment at every start — points
  nowhere on its own. The rename that replaced the truncating write is the step
  most worth hearing about: replacing a file another process holds open is a
  sharing violation on Windows, and every copy opens this file at start-up.
- On macOS and Linux, the message printed when the widget cannot start no
  longer panics if stderr has gone away. `tpmplaner 2>&1 | head -1` closes it,
  and so does a supervisor, and the panic hook would then write a broken pipe
  into the log the user was about to attach — on the one path that only runs
  when something has already gone wrong.
- The owner-only mode on the data directory is applied to the data directory
  and not to its parents. `DirBuilder` carries one mode and uses it for every
  level it creates, so on a fresh account `~/.config` — or a Mac somehow
  missing `~/Library/Application Support` — would have been made owner-only
  too, and those belong to the platform rather than to us.
- A click in a right-to-left layout lands on what was drawn under it. The
  rectangles a click is tested against are unmirrored, like all the layout
  arithmetic, while the drawing is folded across the panel's axis at the
  drawing primitives — and the two were never brought into the same
  coordinates. Rows span the full width and hid it; the two regions that do not
  showed it plainly. In Arabic or Hebrew the refresh button was drawn in the
  top left and did nothing, while clicking the empty top right corner
  refreshed, and a task's tick circle opened the task in the browser while the
  far end of its row ticked it off. The fold now happens where the click enters
  the hit test, in the core, so the second front end inherits the rule rather
  than the bug.

### Internal

- The second smoke test says what it observes. It pins a French locale and
  checks the output, and that selects the catalogue and nothing else: the Unix
  front end leaves the portable locale backend in force, whose formatters
  ignore the tag they are handed. The comment claimed the date format was being
  observed too, which would have read as a false reassurance to whoever writes
  the real backend and wonders why nothing caught their bug — it now points at
  the assertion that step is waiting to become. The check also looks for
  `TÂCHES`, which is French alone, beside an `AGENDA` that six catalogues
  share.
- The scratch directory in the cache sweep test carries the process id. It was
  a fixed path under the shared temp directory, so two overlapping test runs —
  two checkouts, an editor running tests while the terminal does — had one
  deleting the other's fixtures mid-assertion, and the failure looked like a
  bug in the code under test.
- The text front end's module documentation says its columns are not
  authoritative. The first column is padded by `char` count while a terminal
  counts columns, and the two part company in exactly the languages continuous
  integration was extended to cover. The point of that front end is to be
  something to compare a renderer against, so what it is not a reference for is
  worth stating.
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
- The source really does read in one language now, which the 1.0.2 entry below
  has claimed since it was written. Twenty-four places: the module header of
  `src/win/platform.rs` and six section banners in `src/win/window.rs`, four
  more comments across those two and `src/win/render.rs`, an `unreachable!` and
  three assertion messages, the log lines for copying the agenda and toggling
  autostart, and eleven test fixture titles. Searching for German words is what
  missed most of them twice; searching for *comments with no English function
  words in them* is what found them.
- Five error messages the Google provider shows the user were German while the
  CalDAV and Microsoft ones beside them were English — so since the twenty
  catalogues landed, a Korean or Arabic user hit German at exactly the moment
  something had gone wrong. They now read as their siblings already did
  ("Network error", "Unexpected response", "I/O error"), and an event or task
  with no title is "(no title)" rather than "(ohne Titel)", which is the
  wording `caldav.rs` was already using. Not translated: the detail beside them
  comes from Google in English, so a translated prefix on an English payload
  would be for show. Putting them in the catalogues is a separate change, and
  `CONTRIBUTING.md` is right that it needs a human per language.
- Still German on purpose, because it is content rather than prose about the
  code: the `de` catalogue, the demo data, `"dunkel"`/`"hell"` accepted beside
  `"dark"`/`"light"` in `config.json` and `"strg"`/`"umschalt"` beside
  `"ctrl"`/`"shift"` in `peek_hotkey` — settings users have already written,
  and not ours to invalidate for tidiness — and the umlauts in the two tests
  that exist to prove non-ASCII survives.

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

[Unreleased]: https://github.com/JosunLP/TPMPlaner/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.1.0
[1.0.2]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.2
[1.0.1]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.1
[1.0.0]: https://github.com/JosunLP/TPMPlaner/releases/tag/v1.0.0
