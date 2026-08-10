# Architecture

## The split

```text
crates/core/          tpmplaner-core   — portable, no operating system calls
  model.rs            events, tasks, filtering, sorting, conflict detection
  provider/           Google, Microsoft Graph, CalDAV, iCalendar
  sync.rs             schedule, backoff, parallel fetch, cache
  i18n.rs             catalogues, relative times, bidi handling
  layout.rs           where everything goes, and what a click landed on
  menu.rs             what the right-click menu contains
  theme.rs            palette, metrics, appearance customisation
  hotkey.rs           reading "Ctrl+Alt+Shift+K"
  host.rs             the traits the platform must supply
  update.rs           release check

src/                  tpmplaner       — the front ends
  main.rs             platform-neutral: picks a front end, and nothing else
  win/                #[cfg(windows)]
    render.rs         Direct2D / DirectWrite / DirectComposition
    window.rs         Win32 window, input, timers, menu
    host_impl.rs      Host, LocaleBackend and Waker for Windows
    platform.rs       registry, monitors, clipboard, shortcuts
    secure.rs         DPAPI
  unix/               #[cfg(unix)] — macOS and Linux
    host.rs           Host: XDG paths, xdg-open/open, /dev/urandom
    locale.rs         CFDateFormatter, or the C library's locale database
    secure.rs         Keychain, or the Secret Service
    autostart.rs      a launch agent, or an XDG autostart entry
    app.rs            the widget's behaviour, behind the Shell trait
    paint.rs          the widget's drawing, behind the Canvas trait
    canvas.rs         what a renderer has to be able to draw
    text.rs           prints the agenda where there is no desktop
    mac/              #[cfg(target_os = "macos")]
      window.rs       NSWindow, the run loop, the shell
      canvas.rs       Core Graphics and Core Text
      objc.rs         the slice of the Objective-C runtime that is used
      menu.rs         NSMenu · visuals.rs appearance · hotkey.rs Carbon
    linux/            #[cfg(target_os = "linux")]
      window.rs       X11 window, the poll loop, the shell
      canvas.rs       Cairo and Pango
      ffi.rs          Xlib, Cairo and Pango, opened with dlopen
      menu.rs         the menu, drawn · visuals.rs the XDG portal
```

The core has **no dependency on the `windows` crate** and calls no platform
API. Continuous integration builds and tests it on Ubuntu, macOS and Windows,
so that boundary is enforced rather than merely intended.

`main.rs` sees one thing from a front end: `install_host`,
`acquire_single_instance`, `run` and `fatal`. Which module supplies them is a
`#[cfg]` and nothing else, so adding a platform is adding a directory rather
than threading conditionals through the program.

**Two traits sit between the front ends and the shared code.** `Canvas` is what
a renderer has to be able to draw — a rounded rectangle, a line, a circle, an
arc, a gradient, a clip and two kinds of text. `Shell` is what the widget needs
from a window system — geometry, a cursor, three timers, a menu, the clipboard
and the appearance settings. Behind them, `paint.rs` and `app.rs` are written
once and serve macOS and Linux alike; the Direct2D renderer predates both and
still draws the same picture from its own code, which
[Porting](/development/porting) records as the one description of the interface
too many.

## The three traits

Everything the core needs from an operating system goes through
`core/src/host.rs`:

| Trait | Supplies |
|---|---|
| `Host` | data directory, opening a browser, encrypting a secret, random bytes |
| `LocaleBackend` | date and time formatting, reading direction |
| `Waker` | how background work reaches the interface |

`LocaleBackend` is separate from `Host` on purpose: the quality of the answers
differs enormously. Windows has NLS, macOS has Core Foundation's CLDR data, and
Linux has the C library's locale database — where the long date is an
approximation, because POSIX has no pattern for one. A portable fallback can
only approximate all of it. Splitting them lets a front end supply a good
implementation of one and take the default for the other.

Portable fallbacks exist for all of them, so tests and headless tools work
without installing anything. What the fallback cannot do — real keychain
storage, cryptographic randomness — is stated plainly in the code rather than
faked.

## No async runtime

Everything is blocking, on two threads:

- The **interface thread** draws and owns all scheduling.
- The **sync thread** does nothing but network work.

Accounts are fetched in parallel through `std::thread::scope`, which lets each
worker borrow its own provider mutably — which is what token refresh inside
needs. No lock, no ordering between accounts; results are reassembled in
configured order so the display stays stable.

This keeps the idle footprint at roughly 4 MB. An async runtime would add a
thread pool that sleeps 99.99 % of the time.

## Drawing only when something changes

There is no render loop. The widget draws on new data, on the minute, on hover
— and at 60 Hz **only while an animation is running**, after which the timer is
switched off entirely. Measured idle cost is around 0.03 seconds of CPU across
40 seconds, which is the one-per-minute clock redraw.

Three timers, each alive only as long as needed: the minute tick, the animation
tick, and the undo countdown.

## Failure isolation

Every provider call is wrapped per account. A failing account contributes an
error instead of data; the healthy ones are unaffected. Sign-in problems
outrank network errors when choosing what to report, because one needs the user
to act and the other resolves itself.

Panics are written to the log before the process aborts — without that, the
widget would simply vanish from the desktop with no trace. Mutex poisoning is
tolerated: the shared state is display data, and refusing to recover would turn
a local fault into a total failure.

## The traps worth knowing

These are the places where the obvious implementation is wrong.

**A plain task due date is a date, not an instant.** Every task API returns
`2026-08-05T00:00:00.000Z` for it. Parsing that as UTC and converting to local
time moves it a day in most of the world, so `model::parse_task_due` reads ten
characters and does not convert.

**A due date with a time of day is the opposite.** Google Tasks lets a task
carry a time, and then the value is a real instant: `2026-08-06T22:30:00.000Z`
is half past midnight on the 7th in Berlin. Reading ten characters files it as
overdue since yesterday. The two cases are told apart by the time of day —
midnight is a calendar day, anything else is an instant and is converted.

**A server side due-date bound must not sit on the day you are asking about.**
`dueMax` at the end of today put every task due today exactly on the bound, and
Google documents neither whether the bound is inclusive nor what it does with
the time of day. The result was a task list showing nothing but overdue
entries. `google::tasks::due_bound` clears the day by a full day and lets
`model::filter_tasks_for_today` decide the real cutoff — the server filter
exists to save bandwidth, not to be exact.

**Recurring events must be expanded by the server.** Google needs
`singleEvents=true`, Graph needs `calendarView` rather than `events`, CalDAV
needs `<C:expand>`. Without them you get the recurrence rule instead of today's
occurrence.

**Graph timestamps carry no offset.** The zone sits in a sibling field. The
request asks for UTC through a `Prefer` header and checks the reply.

**A system backdrop fills the whole window.** Including the transparent margin
reserved for the shadow, and with square corners — which is exactly the grey
box around a rounded panel.

**Bidi reorders Latin text in a mirrored layout.** "32 min left" renders as
"min left 32" under an Arabic locale. Fixed with `U+202A`/`U+202C`; the newer
isolates are not honoured by DirectWrite.

**Column widths cannot be fixed.** What fits "gestern" truncates "yesterday".
Widths are measured from the actual content each frame.

## Testing

70 tests, the bulk in the core, covering the things that break silently: date
parsing across zones, overlap detection at boundaries, accent colour readability,
bidi handling, CalDAV and Graph response shapes, hotkey parsing, version
ordering.

The renderer and window are exercised by compiling — a broken Direct2D call is
a compile error, and a headless runner could not observe the rest anyway.
