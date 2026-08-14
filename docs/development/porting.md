# Porting to macOS and Linux

The port is done.

`ephemeris-core` holds the model, the calendar back ends, synchronisation,
localisation and the palette, and calls no operating system API. On top of it
sit four front ends: a Direct2D renderer in a Win32 window, a Core Graphics
renderer in an `NSWindow`, and a Cairo renderer in either a Wayland surface or
an X11 window. All four draw the same widget — below every normal window but
above the desktop, no taskbar entry, no window-switcher entry, never takes
focus, with a shortcut that brings it forward for a few seconds.

This page is the decision record. It says what was chosen, why, and what would
reopen each choice.

## What a front end has to provide

`src/main.rs` knows nothing about any operating system. A platform module
supplies four functions — `install_host`, `acquire_single_instance`, `run`,
`fatal` — and everything else reaches the core through the three traits in
`crates/core/src/host.rs`:

| Trait | Methods | Windows | macOS | Linux |
|---|---|---|---|---|
| `Host` | `data_dir` | `%APPDATA%` | `~/Library/Application Support` | `$XDG_CONFIG_HOME` |
| | `open_url` | `ShellExecuteW` | `open` | `xdg-open` |
| | `protect` / `unprotect` | DPAPI | Keychain | Secret Service |
| | `random_bytes` | `BCryptGenRandom` | `/dev/urandom` | `/dev/urandom` |
| `LocaleBackend` | tag, direction, four formatters | NLS | `CFDateFormatter` | C library |
| `Waker` | `wake` | `PostMessageW` | `performSelectorOnMainThread:` | a byte down a pipe |

Two things a front end does not decide for itself. `main` turns a `run` that
returned `Err` into a failure exit status — a widget that could not start must
not tell a shell or a service manager that all is well; `acquire_single_instance`
returning `false` is a success, because nothing went wrong. And whatever
`open_url` is given goes through `host::is_openable_url` first: an event's link
comes from the calendar server, and every platform's opener launches what is
registered for a scheme rather than merely browsing to it.

**Two of these arrived differently from how this page first predicted, and the
reasons are worth keeping.**

*Credentials are stored, not encrypted.* DPAPI encrypts a blob and hands it back
to be written to a file. The Keychain and the Secret Service do not do that at
all — they hold a secret under a name and give it back to the account that owns
it. So the secret goes into the keyring and what lands in `token.bin` is a
reference to it. That is a better shape than DPAPI's rather than a compromise:
the token never reaches the file, so a copied `token.bin` is worth nothing.
Where there is no keyring — a container, a headless session — the bytes go into
the file as they are and a warning says so once. A keyring that is present and
refuses is not that case and does not take that route: a locked keychain, a
cancelled passphrase prompt or a daemon that is a second behind are all
temporary, and a plain copy on disk is not, so nothing is written and the token
is stored on the next refresh instead.

*macOS uses Core Foundation, not `NSDateFormatter`.* `CFDateFormatter` is what
`NSDateFormatter` is built on, and `CFDateFormatterCreateDateFormatFromTemplate`
is `dateFormatFromTemplate:` under its C name — the same CLDR data through a
plain C API, so no message-send machinery and no extra crate.

*Linux uses the C library.* `newlocale` and `uselocale` give the thread its own
locale, `nl_langinfo` the locale's own patterns and `strftime` the rendering.
The hour convention, the names, the field order and the separators are all
correct. The long date is the honest gap: POSIX has no long-date pattern, so
the short one is widened — the locale's own order and punctuation are kept, so
`%Y年%m月%d日` stays that shape, but a locale whose long form differs by more
than the month's width will be close rather than exact.

**`icu4x` was measured, and the answer is "not for this".** The test the
earlier version of this page asked for: two programs built with this project's
own release profile, one that does nothing and one that formats a long date
with `icu` 2 and `DateTimeFormatter` at `YMD::long()`, with the locale chosen
at run time so the data cannot be sliced down to one language.

| | |
|---|---|
| Baseline | 289,856 bytes |
| With `icu4x` | 1,344,648 bytes |
| **Cost** | **1,054,792 bytes — 1.01 MB** |

It works, and correctly: `11. August 2026` under `de-DE`, `11 August 2026`
under `en-GB`, ordinal dot and all. But a megabyte is right on the line this
page drew, and it buys a better long date on *one* of the three platforms —
Windows has NLS and macOS has Core Foundation, and both are already exact. Sixty
per cent onto a ~1.7 MB binary, for one platform's month names, is not a trade
worth making.

What would change it: needing correct CLDR data for something the platforms
*cannot* do — a non-Gregorian calendar the C library has no locale for, say —
at which point the megabyte buys three platforms rather than one. The
measurement is cheap to repeat, and the numbers above say what to compare
against.

## How much is shared

The window and the renderer are the platform-specific half, and rather less of
them is platform-specific than it looks. Four layers, from the portable end:

| | |
|---|---|
| `ephemeris_core::layout` | Where everything goes. Row and hit rectangles, the day rail's span, the tick circle, the scroll thumb, tooltip placement, which rows are visible, what fades, where the now line goes. Arithmetic over the metrics and the model, with its own tests. |
| `ephemeris_core::menu` | What the right-click menu contains — which entries, which ticked, which greyed. Also tested. |
| `src/paint/widget.rs` | The widget's *drawing*, written once against the `Canvas` trait. **Every** front end. |
| `src/unix/app.rs` | The widget's *behaviour* — input, timers, commands — written once against the `Shell` trait, by the two front ends that were written together. |

`Canvas` is a rounded rectangle, a line, a circle, an arc, a gradient, a clip
and two kinds of text. Four implementations sit behind it — Direct2D, Core
Graphics, Cairo on an X11 window and Cairo on a Wayland buffer — and each is a
few hundred lines. There is one description of what the widget looks like, and
changing it changes all four.

`Shell` is the window's geometry, a cursor, three timers, a menu, the clipboard
and the system's appearance. Only macOS and Linux meet the shared code there:
`src/win/window.rs` predates the trait and owns its own Win32 message pump.
That is a smaller duplication than the drawing was — a message pump and a
`poll` loop have genuinely little in common, which is why `Waker` is a trait
and the loop is not — and the *decisions* a loop makes are already shared
through `layout` and `menu`.

`src/paint` sits beside the front ends rather than in `ephemeris-core`, and the
line is worth naming. The core answers *what to show* and *where it goes*, and
can be tested without a device. `paint` answers *what it looks like*, which
cannot be, and would drag a notion of drawing into a crate whose whole point is
not having one.

## Decided: no cross-platform toolkit

egui, iced, GTK and Tauri would each give one interface everywhere, and each was
considered and rejected for the same reason: the widget's defining behaviour is
precisely what they abstract away. Below other windows, no taskbar or
window-switcher entry, never takes focus, system backdrop, system accent and
contrast settings honoured at runtime — every one of those needs a platform
escape hatch in every toolkit, which means writing the platform code anyway and
carrying a toolkit on top of it.

They also work against two properties this project states as facts: a ~1.7 MB
single-file binary and under 4 MB resident at rest.

This is a decision, not a prohibition. What would reopen it: a toolkit that
exposes window level, focus policy and taskbar visibility as first-class
settings on all three platforms.

The same reasoning rejected `objc2` and `cocoa` on macOS: the surface actually
used is a few dozen selectors, and `src/unix/mac/objc.rs` declares them in 400
lines with the two ABI rules that matter written down at the top.

## macOS

| | |
|---|---|
| Window | `NSWindow` at `kCGDesktopIconWindowLevel + 1`, `canBecomeKeyWindow` returning NO, collection behaviour `.canJoinAllSpaces` + `.stationary` + `.ignoresCycle`, activation policy `.accessory` so there is no Dock tile |
| Rendering | Core Graphics into a flipped `NSView`, Core Text for the type, behind the shared `Canvas`. No `NSVisualEffectView`: the renderer draws its own glass, and a system backdrop behind a rounded panel is a square box around it — the same trap `"backdrop": "acrylic"` documents on Windows |
| Locale | `CFDateFormatter` with `CFDateFormatterCreateDateFormatFromTemplate` |
| Credentials | Keychain: `SecItemAdd`, `SecItemCopyMatching`, `SecItemUpdate`, accessible after first unlock so a widget that starts at login can still sync |
| Appearance | `NSApp.effectiveAppearance`, `NSColor.controlAccentColor`, and `NSWorkspace`'s three `accessibilityDisplay…` switches |
| Hotkey | `RegisterEventHotKey`. **Not** an `NSEvent` global monitor: that needs the accessibility permission, which means a system dialogue, a trip to System Settings, and a documented feature that does nothing until somebody comes back and grants it |
| Autostart | A launch agent in `~/Library/LaunchAgents`. `SMAppService` is the modern route and needs a signed bundle registering itself; a launch agent works for the bare binary the tarball ships |
| Single instance | `NSRunningApplication`, comparing executable paths — no lock file to leave behind |
| Shipping | A tarball per architecture. A signed and notarised `.app` is worth having and needs a paid developer account, so it is not a thing this repository can produce on its own |

The run loop is written out rather than `[NSApp run]`. `-run` returns only when
`-stop:` is sent, and `-stop:` takes effect only once the loop finishes the
event it happens to be in — so quitting from the menu would leave the widget
closed and the process alive until the next mouse movement arrived.

The widget draws in **points**, not pixels. A point is 1/72 inch where a Windows
device-independent pixel is 1/96, so the metrics tuned against Windows come out
about a third larger — which is the difference between the two systems' own
interface conventions. `fs_row` at 12.5 lands beside the 13-point system font
and `fs_title` at 17 beside the 17-point title. Retina never reaches this
program: AppKit scales the context.

## Linux: the decision that had to be made first

**X11 works.** `_NET_WM_STATE_BELOW`, `SKIP_TASKBAR`, `SKIP_PAGER`, `STICKY`,
Motif hints for the missing frame, and `WM_HINTS.input = False` so the widget
never takes the keyboard. Every piece of its behaviour has a standard for it.

`_NET_WM_WINDOW_TYPE_DESKTOP` was the other candidate and is wrong: it means
*is* the desktop, and window managers that take it seriously stack the widget
beneath the icons and stop sending it clicks.

**Wayland has no general protocol for "below every window, above the
desktop".** `wlr-layer-shell` gives exactly the right semantics and covers the
wlroots compositors — Sway, Hyprland, river, Wayfire — and the KDE Plasma
compositor. GNOME's Mutter does not implement it and has said it will not.

**The decision stands, and both halves are written: X11 and
`wlr-layer-shell`, and where neither is available the widget says what it
cannot do rather than pretending.**

A Wayland session takes the native back end, not XWayland — under XWayland the
compositor treats the widget as a foreign client and the one request that
matters is the one it is least likely to honour. On a wlroots or KDE
compositor the surface goes on the bottom layer and behaves exactly as the
Win32 and AppKit windows do. On Mutter, which has no layer shell, it falls back
to an `xdg_toplevel`, and `src/unix/linux/wayland/mod.rs::report_shape` writes
into the log what was found and what follows — so "the widget is not staying
behind my windows" has an answer in the file bug reports ask for.

Two things really are different under the toplevel fallback, and both are
Wayland's design rather than an omission. The widget cannot place itself, so
dragging is handed to the compositor with `xdg_toplevel.move`. And it cannot
raise itself, so a peek is the fade without the rise — which costs nothing,
because a toplevel was never behind anything to begin with.

The alternatives were weighed and remain rejected:

- *A GNOME Shell extension.* It would work, and it is a second codebase in a
  second language with its own review process and its own breakage every GNOME
  release. A contributor who wants it is welcome to it as a separate component.
- *Silently degrading to a normal window.* Rejected. A widget that quietly
  stops doing the one thing it is for is worse than one that says so.
- *X11 only, and let everyone run XWayland.* Rejected as the *stated* answer,
  although it is what happens today: it trades a clear limitation for a
  confusing one, which is why the log is explicit.

## Linux: the rest

| | Choice | Why |
|---|---|---|
| Rendering | Cairo, on an Xlib surface or a Wayland shared-memory buffer | One renderer, two surfaces. X11 draws into the window and the server composites; Wayland draws into memory the compositor reads, two buffers deep so a frame can be drawn while it still holds the last one. `CAIRO_FORMAT_ARGB32` and `WL_SHM_FORMAT_ARGB8888` are the same bytes, so there is no conversion. A display with no 32-bit visual falls back to the default one and draws opaque rather than refusing to start |
| Text | **Pango**, not `cairo_show_text` | Cairo's own text API is documented as a "toy" interface: one font, no shaping, no bidirectional reordering, no fallback for a script the font does not cover. Twenty catalogues with Arabic and Hebrew among them make that disqualifying. Pango is on every desktop that has Cairo, because GTK needs both |
| Linking | `dlopen` at run time, not a build dependency | The same binary has to work on a desktop and over SSH. Linking Xlib would make it refuse to *start* in a container, on a continuous-integration runner and on a headless server, where what it should do is print the agenda. It also means the tarball is one file that runs on any distribution, and CI builds it with no `-dev` package installed |
| Locale | **The C library**; `icu4x` still to be measured | `nl_langinfo` gives the locale's own `T_FMT`, which settles 12- against 24-hour properly rather than guessing from the language. What it cannot give is a long date, because POSIX has no pattern for one |
| Credentials | `secret-tool` | The Secret Service is a D-Bus interface, and every client is either a C library to link or a dozen crates to carry. `secret-tool` is the reference client and ships wherever the service does. The headless fallback is in place and says it is not encrypting |
| Appearance | `org.freedesktop.appearance` through `gdbus` | One portal interface that covers GNOME and KDE alike, and answers inside a Flatpak too. Driven with `gdbus` for the `secret-tool` reason; `gdbus monitor` watches for changes rather than the widget polling. The portal carries no key for "reduce motion" or "high contrast", so those come from `gsettings` where it answers and default to the values that change nothing where it does not |
| Menu | Drawn by the widget | Neither X11 nor Wayland has menus, and with no toolkit there is nothing to ask. The rows, the measurements and the painting are shared (`drawn_menu.rs`); what differs is the surface and the dismissal. X11 uses an override-redirect window with a pointer grab; Wayland uses an `xdg_popup`, the only object in that protocol that comes with a grab and a "the user clicked elsewhere" event. Submenus open in place with a row back to the top, which avoids a hierarchy of grabs |
| Hotkey | `XGrabKey` on X11; the compositor's own binding on Wayland | Wayland gives no client the power to grab a key — deliberately, and it is an improvement. So `ephemeris --peek` tells the running copy to come forward through the socket that already makes it single-instance, and one line in the compositor's configuration is the shortcut. It works on X11 too |
| Autostart | A `.desktop` file in `$XDG_CONFIG_HOME/autostart` | Old, small, and honoured by GNOME, KDE, XFCE, LXQt and the tiling compositors' session managers alike |
| Single instance | An abstract socket | No path, so nothing is left in `/tmp` for the next start to trip over, and the kernel releases it however the process ends. Named after the display, so two sessions on one host are two desktops |
| Shipping | Tarball; distributions package from source | There is no equivalent of the `install.ps1` one-liner, and inventing one that writes outside the package manager's view would be a worse citizen than having none. A Flatpak is the obvious next packaging step |

## Wayland without a code generator

Every Wayland binding in existence is produced by `wayland-scanner` from the
protocol XML. This one is not, for the reason the project carries no toolkit: a
build step is a thing that breaks on somebody else's machine.

Most of the protocol did not have to be written out. `libwayland-client.so.0`
*exports* the interface description of every core object — `wl_surface`,
`wl_shm`, `wl_pointer` and the rest — so those are `dlsym`ed like any other
symbol and cannot be got wrong. What is written by hand in
`wayland/ffi.rs` is only what libwayland does not ship: `zwlr_layer_shell_v1`,
`zwlr_layer_surface_v1`, and the four `xdg_shell` objects a popup menu needs.

Two rules make that safe to read. A request's opcode is its *position* in its
interface, so every table lists every request up to the last one used, in
order, including the ones the widget never sends — a gap silently renumbers the
rest. And a listener is an array of function pointers indexed by event opcode,
so `wl_pointer`'s has eleven entries although the widget binds version one: an
array shorter than the interface is a call past its end if a compositor ever
sends a later event.

`wl_proxy_marshal_flags` is variadic and is called through a variadic function
pointer rather than a convenient fixed one. On x86-64 a variadic callee reads
`al` for the number of vector registers used, and a call made through a
non-variadic type never sets it.

## What is left

- **A notarised macOS `.app`**, which needs a paid developer account. The
  release builds an unsigned bundle, which Gatekeeper refuses on first launch
  until the user right-clicks and chooses Open.
- **Publishing the Flatpak.** The manifest is in `packaging/linux` and
  `cargo-sources.json` is generated and checked against the lock file. The
  icon is in place — `assets/logo.svg`, installed into
  `hicolor/scalable/apps` under the application ID, which is what
  `appstreamcli compose` looks for at the end of every build and used to stop
  on. The submission manifest is settled too: it is generated rather than
  kept, by `packaging/linux/flathub-manifest.py --tag vX.Y.Z`, which replaces
  the directory source with the tag and the commit it points at, checks the
  crate list against the lock file *at that revision*, and writes the pair to
  copy into the Flathub repository. Generating it from the pinned commit
  rather than from the working tree is the point: there is one manifest to
  edit, and nothing that can drift away from what was submitted.

  What Flathub still wants is at least one screenshot. That is a linter error
  rather than a build failure, so the build itself now completes, but the
  submission does not pass without it — and screenshots have to be mirrored to
  `dl.flathub.org` rather than served from anywhere else.
- **The Windows window on `Shell`**, so all four front ends share the
  behaviour as well as the drawing. Less pressing than the renderer was: a
  Win32 message pump and a `poll` loop really are different things, and what a
  loop *decides* is already shared.

## What testing can and cannot reach

Continuous integration builds the binary on all three systems, runs clippy with
warnings denied, runs the full test suite, and smoke-tests three front ends:
the printed agenda in two languages, the X11 window under `xvfb`, and the
Wayland surface under a headless Sway — which is the only way to exercise the
layer shell, since that is the half no compiler can check and no other
compositor available to a runner implements. Each is stopped after a few
seconds and judged by its log.

That is what stops the Linux front ends rotting. A `dlopen` failure, a wrong
Xlib signature, a missing Pango symbol, a mis-numbered Wayland opcode and a
panic in the first frame are all invisible to a compiler and all fatal there.
The X11 run deliberately uses a virtual server with no 32-bit visual and no
compositing manager — the *harder* of the two cases, where the widget has to
fall back to the default visual and draw opaque.

There is no equivalent for macOS. A window opened on a runner cannot be
observed, and the same was already true of Direct2D: a broken drawing call is a
compile error, not something a headless machine notices. What CI does prove
there is that it compiles, links against the frameworks, and passes every test
that does not need a screen.

`scripts/check-macos.sh` brings the compile half of that forward for anyone
working on the port without a Mac. `cargo check` and `cargo clippy` never link,
so the AppKit symbols do not have to exist — two edits to a scratch copy of the
tree (compile `src/unix/mac` unconditionally, and change `kind = "framework"`
to `kind = "dylib"`, which rustc rejects off Apple targets) are enough to make
a Linux host type-check the whole macOS front end under the same lints CI
denies warnings for. It runs three passes, because `send_rect` has an `aarch64`
branch an x86_64 build never looks at and three modules have macOS halves a
Linux host never compiles, and it injects a deliberate type error first and
refuses to report success unless the compiler catches it — a green harness that
checks nothing is the easy mistake here.

What it cannot prove is that the selectors exist, that the message signatures
match AppKit's, or that anything appears on screen. Those need a Mac.
