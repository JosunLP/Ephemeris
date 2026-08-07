<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# TPMPlaner

**Today's calendar events and due tasks on your desktop.** Google, Outlook,
Teams and CalDAV side by side, in one small window that stays out of the way.

[![CI](https://github.com/JosunLP/TPMPlaner/actions/workflows/ci.yml/badge.svg)](https://github.com/JosunLP/TPMPlaner/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Documentation](https://img.shields.io/badge/docs-josunlp.github.io-brightgreen)](https://josunlp.github.io/TPMPlaner/)

📖 **[Documentation](https://josunlp.github.io/TPMPlaner/)** ·
⬇️ **[Releases](https://github.com/JosunLP/TPMPlaner/releases/latest)**

---

## Install

```powershell
irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/install.ps1 | iex
```

No administrator rights, nothing outside your user profile, and the SHA-256
checksum is verified before anything is written to disk.

Then connect a calendar — see [connecting accounts](https://josunlp.github.io/TPMPlaner/guide/accounts).

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
- **Speaks your system's language.** Interface text in English, German, French,
  Spanish and Italian — but dates, times and reading direction come from the
  operating system, so a 12-hour clock, a Hijri calendar or a right-to-left
  layout are simply correct.
- **Fits your theme.** Light and dark, the accent colour, contrast themes, and
  the transparency and animation settings, all followed at runtime.

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
| **Windows 10/11** | Supported |
| **macOS, Linux** | The binary builds and runs, and prints today's agenda; the window is not ported yet |

`tpmplaner-core` contains the model, the calendar back ends, synchronisation,
localisation and the palette, and calls no operating system API at all — CI
enforces that on Ubuntu, macOS and Windows. What remains platform-specific is
the renderer and the window.

On macOS and Linux the data directory, opening a browser and cryptographic
random bytes are implemented, and running the binary prints the agenda the
widget would have drawn — so the portable half is exercised end to end on both
rather than merely type checked. Credential storage and the window are not, and
[the porting notes](https://josunlp.github.io/TPMPlaner/development/porting)
say which API each of them needs and which decisions have already been
made.

## Build from source

```bash
git clone https://github.com/JosunLP/TPMPlaner
cd TPMPlaner
cargo test --workspace
cargo run --release
```

Rust 1.90 or newer; MSVC build tools on Windows. To see the interface without
connecting an account:

```powershell
$env:TPMPLANER_DEMO = "1"; cargo run --release
```

## Documentation

| | |
|---|---|
| [Getting started](https://josunlp.github.io/TPMPlaner/guide/getting-started) | Install and first run |
| [Connecting accounts](https://josunlp.github.io/TPMPlaner/guide/accounts) | Google, Microsoft and CalDAV setup |
| [Using the widget](https://josunlp.github.io/TPMPlaner/guide/using) | What the panel shows and how to drive it |
| [Configuration](https://josunlp.github.io/TPMPlaner/guide/configuration) | Every setting |
| [Troubleshooting](https://josunlp.github.io/TPMPlaner/guide/troubleshooting) | When something is wrong |
| [Architecture](https://josunlp.github.io/TPMPlaner/development/architecture) | How it is put together, and the traps |
| [Porting](https://josunlp.github.io/TPMPlaner/development/porting) | What macOS and Linux still need, and what was decided |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The largest open piece is the
cross-platform interface; adding an interface language is a single constant.

## Honest limitations

The Microsoft Graph and CalDAV back ends were written against the documented
APIs and their response formats are covered by tests, but they have not been
run against every live tenant and server configuration. If something is off,
the log will say what — please
[open an issue](https://github.com/JosunLP/TPMPlaner/issues/new/choose).

## Licence

[GNU General Public License, version 3 or later](LICENSE).
