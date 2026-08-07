# Porting to macOS and Linux

The portable core builds and passes its tests on Ubuntu, macOS and Windows, and
since the platform split the binary builds and runs on all three as well. What
runs on macOS and Linux is a text front end: the settings, the calendars, the
sync and the agenda, printed instead of drawn. The window is what is missing.

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
| | `protect` / `unprotect` | DPAPI | Keychain ❌ | Secret Service ❌ |
| | `random_bytes` | `BCryptGenRandom` | `/dev/urandom` ✅ | `/dev/urandom` ✅ |
| `LocaleBackend` | tag, direction, four formatters | NLS | `NSDateFormatter` ❌ | `icu4x` ❌ |
| `Waker` | `wake` | `PostMessageW` | run loop source ❌ | event loop proxy ❌ |

✅ done, ❌ still to write. The text front end has a `Waker` that drops a token
in a channel, which is enough to prove the trait is not Windows-shaped, but not
a run loop.

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
| Locale | `NSDateFormatter` with `dateFormatFromTemplate:` — the API `LocaleBackend` was designed around |
| Credentials | Keychain: `SecItemAdd`, `SecItemCopyMatching` |
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
| Locale | `icu4x`, if the binary can afford it | 12- against 24-hour is not reliably derivable from `LC_TIME` alone, which is the specific thing `i18n.rs` warns about. `nl_langinfo` and `strftime` through libc are smaller and get the common cases right. **Measure both**: if `icu4x` with only the datetime component costs under a megabyte it is the honest choice, and if it costs five it is not |
| Credentials | Secret Service over D-Bus (`libsecret` or a pure-Rust client) | With a documented fallback for headless setups that have no keyring daemon — and the fallback has to say it is not encrypting, the way `PortableHost` already does |
| Appearance | `org.freedesktop.appearance` `color-scheme` and `accent-color` through the XDG settings portal | One interface that now covers both GNOME and KDE. `SystemVisuals` needs `high_contrast` and `animations` too; the portal does not carry those yet, so read them from the desktop's own settings and default to "on" |
| Hotkey | The global-shortcuts portal on Wayland, `XGrabKey` on X11 | The portal prompts the user once, which is the correct behaviour for a global hotkey and is not something to work around |
| Shipping | Tarball plus Flatpak; distributions package from source | There is no equivalent of the `install.ps1` one-liner, and inventing one that writes outside the package manager's view would be a worse citizen than having none |

## Order of work

1. **The platform split.** Done. `src/win` is behind `#[cfg(windows)]`, `src/unix`
   holds the host and the text front end, and `src/main.rs` sees only the front
   end contract. CI builds and smoke-tests the binary on all three systems, so
   this cannot rot back.
2. **macOS.** One compositor, one set of APIs, window semantics that map almost
   one to one. `LocaleBackend` first — it is self-contained, testable against
   known locales and needed by everything else.
3. **Linux/X11**, then Wayland via `wlr-layer-shell`, with the GNOME limitation
   reported rather than hidden.
4. **Release artefacts** per platform, once there is something to ship.

`TPMPLANER_DEMO=1` renders the interface without an account and is the smoke
test target for each new front end. It is what CI runs on Ubuntu and macOS
today, against the text front end.

## What the split does not solve

- **`Waker` is a trait, but the event loop is not.** Each front end still owns
  its own loop. That is correct — a run loop source and a posted message have
  nothing in common — but it means `window.rs` is not a template for the next
  platform, only a worked example.
- **The renderer draws, it does not lay out.** `render.rs` mixes both. A second
  front end will want the layout arithmetic — where a row starts, how wide the
  time column is, which rectangle a click landed in — and that is portable code
  currently living on the Windows side. Extracting it is worth doing *before*
  the second renderer, not after, or the arithmetic will be written twice and
  drift.
- **Nothing tests the drawing.** A broken Direct2D call is a compile error, not
  something a headless runner observes. That will be equally true of Core
  Graphics and Cairo.
