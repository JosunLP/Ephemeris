# Porting to macOS and Linux

The port is done, with one piece named at the end of this page still open.

`tpmplaner-core` holds the model, the calendar back ends, synchronisation,
localisation and the palette, and calls no operating system API. On top of it
sit three front ends: a Direct2D renderer in a Win32 window, a Core Graphics
renderer in an `NSWindow`, and a Cairo renderer in an X11 window. All three
draw the same widget — below every normal window but above the desktop, no
taskbar entry, no window-switcher entry, never takes focus, with a global
shortcut that brings it forward for a few seconds.

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

*Linux uses the C library, and `icu4x` is still open.* `newlocale` and
`uselocale` give the thread its own locale, `nl_langinfo` the locale's own
patterns and `strftime` the rendering. The hour convention, the names, the field
order and the separators are all correct. The long date is the honest gap: POSIX
has no long-date pattern, so the short one is widened — the locale's own order
and punctuation are kept, so `%Y年%m月%d日` stays that shape, but a locale whose
long form differs by more than the month's width will be close rather than
exact. **The measurement stands**: if `icu4x` with only the datetime component
costs under a megabyte it is worth the swap, and if it costs five it is not.

## How much is shared

The window and the renderer are the platform-specific half, and rather less of
them is platform-specific than it looks. Four layers, from the portable end:

| | |
|---|---|
| `tpmplaner_core::layout` | Where everything goes. Row and hit rectangles, the day rail's span, the tick circle, the scroll thumb, tooltip placement, which rows are visible, what fades, where the now line goes. Arithmetic over the metrics and the model, with its own tests. |
| `tpmplaner_core::menu` | What the right-click menu contains — which entries, which ticked, which greyed. Also tested. |
| `src/unix/paint.rs` | The widget's *drawing*, written once against the `Canvas` trait. |
| `src/unix/app.rs` | The widget's *behaviour* — input, timers, commands — written once against the `Shell` trait. |

`Canvas` and `Shell` are the two boundaries the macOS and Linux front ends meet
the shared code at. `Canvas` is a rounded rectangle, a line, a circle, an arc, a
gradient, a clip and two kinds of text; `Shell` is the window's geometry, a
cursor, three timers, a menu, the clipboard and the system's appearance. Each
back end is a few hundred lines behind those.

**The Windows renderer is not built this way, and that is one description of
the interface too many.** `src/win/render.rs` draws the same picture from its
own Direct2D code. It predates the split, it works, and rewriting a working
renderer against a trait designed after it is a change with real risk and no
user-visible result. What keeps the two from drifting is that everything
*decided* rather than drawn already lives in `layout` and `menu`, where a change
reaches all three at once. Migrating Direct2D onto `Canvas` is the natural next
piece of work; it is not this one.

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
| Rendering | Core Graphics into a flipped `NSView`, Core Text for the type. No `NSVisualEffectView`: the renderer draws its own glass, and a system backdrop behind a rounded panel is a square box around it — the same trap `"backdrop": "acrylic"` documents on Windows |
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

**The decision stands: X11 and `wlr-layer-shell`, and where neither is
available the widget says what it cannot do rather than pretending.** The X11
half is written. Under a Wayland session it runs through XWayland, and whether
`_NET_WM_STATE_BELOW` reaches the compositor is then the compositor's business
— several ignore it for X11 clients. `src/unix/linux/mod.rs::report_session`
writes what was detected and what follows from it into the log at every start,
so "the widget is not staying behind my windows" has an answer in the file bug
reports ask for. **The layer-shell back end is the one piece of the port still
outstanding**; see *What is left*, below.

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
| Rendering | Cairo on an Xlib surface | No intermediate image and no copy per frame; the X server composites. A 32-bit `TrueColor` visual gives per-pixel alpha, and a display with none falls back to the default visual and draws opaque rather than refusing to start |
| Text | **Pango**, not `cairo_show_text` | Cairo's own text API is documented as a "toy" interface: one font, no shaping, no bidirectional reordering, no fallback for a script the font does not cover. Twenty catalogues with Arabic and Hebrew among them make that disqualifying. Pango is on every desktop that has Cairo, because GTK needs both |
| Linking | `dlopen` at run time, not a build dependency | The same binary has to work on a desktop and over SSH. Linking Xlib would make it refuse to *start* in a container, on a continuous-integration runner and on a headless server, where what it should do is print the agenda. It also means the tarball is one file that runs on any distribution, and CI builds it with no `-dev` package installed |
| Locale | **The C library**; `icu4x` still to be measured | `nl_langinfo` gives the locale's own `T_FMT`, which settles 12- against 24-hour properly rather than guessing from the language. What it cannot give is a long date, because POSIX has no pattern for one |
| Credentials | `secret-tool` | The Secret Service is a D-Bus interface, and every client is either a C library to link or a dozen crates to carry. `secret-tool` is the reference client and ships wherever the service does. The headless fallback is in place and says it is not encrypting |
| Appearance | `org.freedesktop.appearance` through `gdbus` | One portal interface that covers GNOME and KDE alike, and answers inside a Flatpak too. Driven with `gdbus` for the `secret-tool` reason; `gdbus monitor` watches for changes rather than the widget polling. The portal carries no key for "reduce motion" or "high contrast", so those come from `gsettings` where it answers and default to the values that change nothing where it does not |
| Menu | Drawn by the widget | X11 has no menus. With no toolkit there is nothing to ask, so it is an override-redirect window drawn with the same Cairo and Pango — one row per entry from the same `tpmplaner_core::menu` list the other two use. Submenus open in place with a "back" row, which avoids a hierarchy of grabs and behaves correctly on every window manager |
| Hotkey | `XGrabKey` on the root window | Repeated for the lock and numeric-lock modifier combinations, or the shortcut stops working the moment Caps Lock is on |
| Autostart | A `.desktop` file in `$XDG_CONFIG_HOME/autostart` | Old, small, and honoured by GNOME, KDE, XFCE, LXQt and the tiling compositors' session managers alike |
| Single instance | An abstract socket | No path, so nothing is left in `/tmp` for the next start to trip over, and the kernel releases it however the process ends. Named after the display, so two sessions on one host are two desktops |
| Shipping | Tarball; distributions package from source | There is no equivalent of the `install.ps1` one-liner, and inventing one that writes outside the package manager's view would be a worse citizen than having none. A Flatpak is the obvious next packaging step |

## What is left

- **A native Wayland back end via `wlr-layer-shell`.** This is the outstanding
  item. It needs the protocol spoken directly — `wl_registry`, `wl_compositor`,
  `wl_shm`, `zwlr_layer_shell_v1` — against a `libwayland-client` opened the
  same way Xlib is, and a Cairo image surface behind a shared-memory buffer
  instead of the Xlib surface. Everything above `Canvas` and `Shell` already
  works and would not change. Until then the log says what XWayland cannot
  promise.
- **A notarised macOS `.app`**, which needs a paid developer account.
- **A Flatpak**, so GNOME and KDE users have something to install rather than a
  tarball to unpack.
- **`icu4x` measured** against the C library's long-date gap.
- **The Windows renderer on `Canvas`**, so there is one description of the
  interface rather than two.

## What testing can and cannot reach

Continuous integration builds the binary on all three systems, runs clippy with
warnings denied, runs the full test suite, and smoke-tests two front ends: the
printed agenda in two languages, and — on Linux, under `xvfb` — the real window,
which is stopped after a few seconds and judged by its log. That last one is
what stops the Linux front end rotting: a `dlopen` failure, a wrong Xlib
signature, a missing Pango symbol and a panic in the first frame are all
invisible to a compiler.

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
