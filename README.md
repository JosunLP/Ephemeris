<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

<p align="center">
  <img src="assets/logo.png" alt="" width="128" height="128">
</p>

# Ephemeris

**Today's calendar events and due tasks on your desktop.** Google, Outlook,
Teams and CalDAV side by side, in one small window that stays out of the way.

[![CI](https://github.com/JosunLP/Ephemeris/actions/workflows/ci.yml/badge.svg)](https://github.com/JosunLP/Ephemeris/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Documentation](https://img.shields.io/badge/docs-josunlp.github.io-brightgreen)](https://josunlp.github.io/Ephemeris/)

📖 **[Documentation](https://josunlp.github.io/Ephemeris/)** ·
⬇️ **[Releases](https://github.com/JosunLP/Ephemeris/releases/latest)**

---

## Install

**Windows**

```powershell
irm https://github.com/JosunLP/Ephemeris/releases/latest/download/install.ps1 | iex
```

No administrator rights, nothing outside your user profile, and the SHA-256
checksum is verified before anything is written to disk.

**macOS and Linux** — a tarball, because neither has an equivalent of that
one-liner that would not fight the package manager:

```bash
# Pick the archive for your system from the releases page, then:
tar -xzf ephemeris-<target>.tar.gz
./ephemeris-<target>/ephemeris
```

Right-click the widget and choose *Start at login* to keep it there.

Then connect a calendar — see [connecting accounts](https://josunlp.github.io/Ephemeris/guide/accounts).

## What it does

- **Every calendar at once.** Google Calendar and Tasks, Microsoft Graph
  (Outlook, Teams and To Do) and any CalDAV server — iCloud, Nextcloud,
  Fastmail, Synology, mailbox.org. Several accounts queried in parallel; one
  failing never blanks the others.
- **Stays out of the way.** Below every normal window but above the desktop. No
  taskbar entry, no Alt-Tab entry, and clicking it never steals focus. One
  shortcut brings it forward for a few seconds.
- **Shows what a list hides.** A day rail with one coloured segment per event,
  a marker for right now, overlapping meetings flagged, and tomorrow's first
  entries once today is done.
- **Tasks that matter today.** Due today or earlier, oldest first, overdue
  marked. Tick one off from the widget, with a few seconds to change your mind.
- **Speaks your system's language.** Interface text in twenty catalogues,
  Arabic and Hebrew among them — and dates, times and reading direction come
  from the operating system, so a 12-hour clock, a Hijri calendar or a
  right-to-left layout are simply correct even in a language nobody has
  translated yet.
- **Fits your theme.** Light and dark, the accent colour, contrast themes, and
  the transparency and animation settings, all followed at runtime — and where
  following the system is not enough, colours, typography, surface style,
  density and per-calendar colours can be set outright, in `config.json` or in
  a theme file you can share.
- **Stays where you put it.** Right-click and lock it: no more nudging it out
  of place while reaching for something on the desktop. Everything else goes on
  working, and the same entry unlocks it.

## What it costs

| | |
|---|---|
| Binary | ~1.7 MB, single file, no runtime to install |
| Idle CPU | 0.03 s across 40 s, measured |
| Resident memory | under 4 MB at rest |

There is no render loop: the widget draws when something changes and then stops.

## Platform support

| | Status |
|---|---|
| **Windows 10/11** | Direct2D in a Win32 window |
| **macOS 11+** | Core Graphics in an `NSWindow`, credentials in the Keychain |
| **Linux, Wayland** | `wlr-layer-shell` on Sway, Hyprland, river, Wayfire and KDE Plasma. On GNOME, which has no layer shell, an ordinary window — and the widget says so in its log rather than pretending |
| **Linux, X11** | The EWMH hints that give the widget its behaviour, on any window manager |

Cairo and Pango draw both Linux back ends; only the surface differs. Anywhere
with no desktop — a container, a server over SSH, a CI runner — the agenda is
printed instead of drawn rather than the widget refusing to start, and
`EPHEMERIS_TEXT=1` asks for that on a machine that does have one.

On Wayland the compositor owns every keyboard shortcut, so bind one to
`ephemeris --peek` — `bindsym $mod+k exec ephemeris --peek` in Sway — and it
brings the running widget forward exactly as the built-in shortcut does
elsewhere.

`ephemeris-core` contains the model, the calendar back ends, synchronisation,
localisation and the palette, and calls no operating system API at all — CI
enforces that on Ubuntu, macOS and Windows. Above it, the widget's layout, its
behaviour and its drawing are each written once; only the window, the renderer
behind a small `Canvas` trait, and the event loop are per platform.
[The porting notes](https://josunlp.github.io/Ephemeris/development/porting)
are the decision record: what was chosen on each system, why, and what is left.

On Linux the desktop's libraries are opened at run time rather than linked, so
the tarball is a single file with no development package to install first.

## Build from source

```bash
git clone https://github.com/JosunLP/Ephemeris
cd Ephemeris
cargo test --workspace
cargo run --release
```

Rust 1.90 or newer, and MSVC build tools on Windows. Nothing else on macOS or
Linux: Xlib, Cairo and Pango are opened at run time, so there is no development
package to install to build it. To see the interface without connecting an
account:

```bash
EPHEMERIS_DEMO=1 cargo run --release        # PowerShell: $env:EPHEMERIS_DEMO = "1"
```

## Documentation

| | |
|---|---|
| [Getting started](https://josunlp.github.io/Ephemeris/guide/getting-started) | Install and first run |
| [Connecting accounts](https://josunlp.github.io/Ephemeris/guide/accounts) | Google, Microsoft and CalDAV setup |
| [Using the widget](https://josunlp.github.io/Ephemeris/guide/using) | What the panel shows and how to drive it |
| [Configuration](https://josunlp.github.io/Ephemeris/guide/configuration) | Every setting |
| [Troubleshooting](https://josunlp.github.io/Ephemeris/guide/troubleshooting) | When something is wrong |
| [Architecture](https://josunlp.github.io/Ephemeris/development/architecture) | How it is put together, and the traps |
| [Porting](https://josunlp.github.io/Ephemeris/development/porting) | What macOS and Linux still need, and what was decided |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The largest open piece is a native
Wayland back end through `wlr-layer-shell`; adding an interface language is a catalogue
constant with its plural rule, a `catalog_for` arm and an entry in `CATALOGS`
— the steps are written out in
[CONTRIBUTING.md](CONTRIBUTING.md#adding-an-interface-language).

## Honest limitations

The Microsoft Graph and CalDAV back ends were written against the documented
APIs and their response formats are covered by tests, but they have not been
run against every live tenant and server configuration. If something is off,
the log will say what — please
[open an issue](https://github.com/JosunLP/Ephemeris/issues/new/choose).

## Licence

[GNU General Public License, version 3 or later](LICENSE).
