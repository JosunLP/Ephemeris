# Porting to macOS and Linux

The portable core builds and passes its tests on Ubuntu, macOS and Windows, and
since the platform split the binary builds and runs on all three as well. What
runs on macOS and Linux is a text front end: the settings, the calendars, the
sync and the agenda, printed instead of drawn.

The host behind it is no longer half of one. Dates and times come from the
platform's own locale database, credentials go to the Keychain or the Secret
Service, and the layout arithmetic the second renderer will need has moved out
of the Windows front end into the core. **The window is what is missing** — and
with it the run loop, the `Waker` and the drawing.

This page is the decision record the port needs before code is written rather
than halfway through. Where a choice is still open it says so, and says what
would settle it.

## What a front end has to provide

`src/main.rs` knows nothing about any operating system. A platform module
supplies four functions — `install_host`, `acquire_single_instance`, `run`,
`fatal` — and everything else reaches the core through the three traits in
`crates/core/src/host.rs`:

| Trait | Methods | Windows | macOS | Linux |
|---|---|---|---|---|
| `Host` | `data_dir` | `%APPDATA%` | `~/Library/Application Support` ✅ | `$XDG_CONFIG_HOME` ✅ |
| | `open_url` | `ShellExecuteW` | `open` ✅ | `xdg-open` ✅ |
| | `protect` / `unprotect` | DPAPI | Keychain ✅ | Secret Service ✅ |
| | `random_bytes` | `BCryptGenRandom` | `/dev/urandom` ✅ | `/dev/urandom` ✅ |
| `LocaleBackend` | tag, direction, four formatters | NLS | `CFDateFormatter` ✅ | C library ✅ |
| `Waker` | `wake` | `PostMessageW` | run loop source ❌ | event loop proxy ❌ |

✅ done, ❌ still to write. The text front end has a `Waker` that drops a token
in a channel, which is enough to prove the trait is not Windows-shaped, but not
a run loop.

**Two of these arrived differently from how this page first predicted, and the
reasons are worth keeping.**

*Credentials are stored, not encrypted.* DPAPI encrypts a blob and hands it back
to be written to a file. The Keychain and the Secret Service do not do that at
all — they hold a secret under a name and give it back to the account that owns
it. So the secret goes into the keyring and what lands in `token.bin` is a
reference to it. That is a better shape than DPAPI's rather than a compromise:
the token never reaches the file, so a copied `token.bin` is worth nothing.
Where there is no keyring — a container, a headless session — the bytes go into
the file as they are and a warning says so once, which is the documented
fallback this page asked for. A keyring that is present and refuses is not that
case and does not take that route: a locked keychain, a cancelled passphrase
prompt or a daemon that is a second behind are all temporary, and a plain copy
on disk is not, so nothing is written and the token is stored on the next
refresh instead. Linux drives `secret-tool` rather than speaking
D-Bus, for reasons set out in `src/unix/secure.rs`; when the window arrives it
will need a D-Bus connection anyway for the portals, and that is the moment to
change it.

*macOS uses Core Foundation, not `NSDateFormatter`.* `CFDateFormatter` is what
`NSDateFormatter` is built on, and
`CFDateFormatterCreateDateFormatFromTemplate` is `dateFormatFromTemplate:`
under its C name — the same CLDR data through a plain C API, so no
message-send machinery and no extra crate.

*Linux uses the C library, and `icu4x` is still open.* `newlocale` and
`uselocale` give the thread its own locale, `nl_langinfo` the locale's own
patterns and `strftime` the rendering. The hour convention, the names, the
field order and the separators are all correct. The long date is the honest
gap: POSIX has no long-date pattern, so the short one is widened — the
locale's own order and punctuation are kept, so `%Y年%m月%d日` stays that
shape, but a locale whose long form differs by more than the month's width will
be close rather than exact. That is what the `icu4x` measurement below is for,
and it is now an improvement to weigh rather than a hole to fill.

Two things a front end does not decide for itself. `main` turns a `run` that
returned `Err` into a failure exit status — a command is judged by it, and a
widget that could not start must not tell a shell or a service manager that all
is well; `acquire_single_instance` returning `false` is a success, because
nothing went wrong. And whatever `open_url` is given goes through
`host::is_openable_url` first: an event's link comes from the calendar server,
and every platform's opener launches what is registered for a scheme rather
than merely browsing to it.

Then a renderer for what `theme.rs` and the layout metrics describe, and a
window with the behaviour the widget is defined by: below every normal window
but above the desktop, no taskbar entry, no window-switcher entry, never steals
focus, and a global hotkey that brings it forward for a few seconds.

## Decided: no cross-platform toolkit

egui, iced, GTK and Tauri would each give one interface everywhere, and each was
considered and rejected for the same reason: the widget's defining behaviour is
precisely what they abstract away. Below other windows, no taskbar or
window-switcher entry, never takes focus, system backdrop, system accent and
contrast settings honoured at runtime — every one of those needs a platform
escape hatch in every toolkit, which means writing the platform code anyway and
carrying a toolkit on top of it.

They also work against two properties this project states as facts: a ~1.7 MB
single-file binary and under 4 MB resident at rest. The current split — portable
core, thin native shell — keeps both.

This is a decision, not a prohibition. What would reopen it: a toolkit that
exposes window level, focus policy and taskbar visibility as first-class
settings on all three platforms.

## macOS

The window semantics map almost one to one, which is why macOS goes first.

| | |
|---|---|
| Window | `NSWindow` at `kCGDesktopIconWindowLevel + 1`, `canBecomeKeyWindow = false`, collection behaviour `.stationary` + `.canJoinAllSpaces` + `.ignoresCycle`, `LSUIElement` in the bundle so there is no Dock tile |
| Rendering | Core Graphics / Core Animation. `NSVisualEffectView` for what `"backdrop": "acrylic"` maps to on Windows |
| Locale | **Done.** `CFDateFormatter` with `CFDateFormatterCreateDateFormatFromTemplate` — `dateFormatFromTemplate:` under its C name, which is the API `LocaleBackend` was designed around |
| Credentials | **Done.** Keychain: `SecItemAdd`, `SecItemCopyMatching`, `SecItemUpdate`, accessible after first unlock so a widget that starts at login can still sync |
| Appearance | `NSApp.effectiveAppearance`, `NSColor.controlAccentColor`, and `NSWorkspace`'s `accessibilityDisplayShouldReduceTransparency` / `ShouldReduceMotion` / `ShouldIncreaseContrast` for the three switches in `SystemVisuals` |
| Hotkey | `RegisterEventHotKey`, or an `NSEvent` global monitor if the accessibility permission is acceptable |
| Shipping | Signed and notarised `.app`, arm64 and x86_64 |

## Linux: the decision that has to be made first

There is no single answer, and pretending otherwise is how a port stalls.

**X11 works.** `_NET_WM_WINDOW_TYPE_DESKTOP` or `_NET_WM_STATE_BELOW`,
`_NET_WM_STATE_SKIP_TASKBAR` and `SKIP_PAGER`, plus refusing input focus. Every
piece of the widget's behaviour has a standard for it.

**Wayland has no general protocol for "below every window, above the
desktop".** `wlr-layer-shell` gives exactly the right semantics — a background
or bottom layer, no focus, no taskbar — and covers the wlroots compositors:
Sway, Hyprland, river, Wayfire, and the KDE Plasma compositor. GNOME's Mutter
does not implement it and has said it will not.

**The decision: X11 and `wlr-layer-shell`, and on GNOME the widget says what it
cannot do rather than pretending.** Under Mutter with no layer shell it opens as
an ordinary window, and the log and the status line say that stacking below
other windows is not available on this compositor. The alternatives were
weighed:

- *A GNOME Shell extension.* It would work, and it is a second codebase in a
  second language with its own review process and its own breakage every GNOME
  release. Not for the first version. A contributor who wants it is welcome to
  it as a separate component.
- *Silently degrading to a normal window.* Rejected. A widget that quietly stops
  doing the one thing it is for is worse than one that says so.
- *X11 only, and let GNOME users run XWayland.* Rejected as the default,
  although it does work: it trades a clear limitation for a confusing one.

## Linux: the rest

| | Choice | Why |
|---|---|---|
| Rendering | Cairo for the X11 back end, and measure before committing the Wayland one | The release profile is tuned for ~1.7 MB. wgpu or Skia would serve both back ends from one renderer and cost several megabytes; that trade is worth making only if one renderer really does serve both, which is exactly what a measurement would show |
| Locale | **The C library for now**; `icu4x` still to be measured | `nl_langinfo` gives the locale's own `T_FMT`, which settles 12- against 24-hour properly rather than guessing from the language — the specific thing `i18n.rs` warns about — and `strftime` renders it. What it cannot give is a long date, because POSIX has no pattern for one. **The measurement stands**: if `icu4x` with only the datetime component costs under a megabyte it is worth the swap, and if it costs five it is not |
| Credentials | **Done, through `secret-tool`**; D-Bus directly when the window lands | The Secret Service is a D-Bus interface, and every client is either a C library to link or a dozen crates to carry — against a stated ~1.7 MB binary. `secret-tool` is the reference client, ships wherever the service does and covers GNOME Keyring and KWallet alike. The window will need a D-Bus connection anyway for the appearance and global-shortcut portals; that is when to speak the protocol and leave this as the fallback. The headless fallback is in place and says it is not encrypting |
| Appearance | `org.freedesktop.appearance` `color-scheme` and `accent-color` through the XDG settings portal | One interface that now covers both GNOME and KDE. `SystemVisuals` needs `high_contrast` and `animations` too; the portal does not carry those yet, so read them from the desktop's own settings and default to "on" |
| Hotkey | The global-shortcuts portal on Wayland, `XGrabKey` on X11 | The portal prompts the user once, which is the correct behaviour for a global hotkey and is not something to work around |
| Shipping | Tarball plus Flatpak; distributions package from source | There is no equivalent of the `install.ps1` one-liner, and inventing one that writes outside the package manager's view would be a worse citizen than having none |

## Order of work

1. **The platform split.** Done. `src/win` is behind `#[cfg(windows)]`, `src/unix`
   holds the host and the text front end, and `src/main.rs` sees only the front
   end contract. CI builds and smoke-tests the binary on all three systems, so
   this cannot rot back.
2. **The host, and the layout arithmetic.** Done. `LocaleBackend` and the
   credential methods are real on both systems — they were first because they
   are self-contained, testable against known locales and needed by everything
   else — and the geometry the renderer used to own now lives in
   `tpmplaner_core::layout` with its own tests. That was the item under *What
   the split does not solve* which had to be settled before a second renderer
   rather than after it.
3. **macOS.** One compositor, one set of APIs, window semantics that map almost
   one to one. What is left is the window itself: `Waker`, the run loop, the
   `NSWindow` and a Core Graphics renderer against the layout module.
4. **Linux/X11**, then Wayland via `wlr-layer-shell`, with the GNOME limitation
   reported rather than hidden.
5. **Release artefacts** per platform, once there is something to ship.

`TPMPLANER_DEMO=1` renders the interface without an account and is the smoke
test target for each new front end. It is what CI runs on Ubuntu and macOS
today, against the text front end.

## What the split does not solve

- **`Waker` is a trait, but the event loop is not.** Each front end still owns
  its own loop. That is correct — a run loop source and a posted message have
  nothing in common — but it means `window.rs` is not a template for the next
  platform, only a worked example.
- **The renderer draws, it does not lay out.** Settled. `tpmplaner_core::layout`
  now holds what `render.rs` used to mix in with the drawing: the panel inset
  and the mirror axis, row and hit rectangles, the tick circle and its click
  target, the day rail's span, the scroll thumb, tooltip placement, and the
  rules deciding which rows are visible, what fades, where the now line goes
  and whether a calendar name has earned its column. The one thing that stayed
  platform-side is measuring text — only the front end knows how wide a word is
  in its font — so the column widths take a measured number and decide what to
  do with it.
- **Nothing tests the drawing.** A broken Direct2D call is a compile error, not
  something a headless runner observes. That will be equally true of Core
  Graphics and Cairo.
