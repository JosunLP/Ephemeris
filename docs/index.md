---
layout: home

hero:
  name: TPMPlaner
  text: Your day, on your desktop
  tagline: >
    Today's calendar events and due tasks, always visible, never in the way.
    Google, Outlook, Teams and CalDAV side by side.
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: Connect an account
      link: /guide/accounts
    - theme: alt
      text: View on GitHub
      link: https://github.com/JosunLP/TPMPlaner

features:
  - title: Every calendar at once
    details: >
      Google, Microsoft (Outlook, Teams and To Do) and any CalDAV server —
      iCloud, Nextcloud, Fastmail — queried in parallel. One account failing
      never blanks the rest.
  - title: Out of the way by design
    details: >
      Sits below every normal window but above the desktop. No taskbar entry,
      no Alt-Tab entry, and clicking it never steals focus from what you were
      doing. One shortcut brings it forward when you want it.
  - title: Costs nothing while idle
    details: >
      Measured 0.03 seconds of CPU across 40 seconds and under 4 MB resident.
      There is no render loop: it draws when something changes, and stops.
  - title: Sees what a list hides
    details: >
      A day rail showing the shape of your day, a marker for right now,
      overlapping meetings flagged, and a preview of tomorrow once today is
      done.
  - title: Speaks your system's language
    details: >
      Five interface languages, but dates, times and reading direction come
      from the operating system — so a 12-hour clock, a Hijri calendar or a
      right-to-left layout are all simply correct.
  - title: Fits your theme
    details: >
      Follows light and dark, the accent colour, contrast themes, and the
      transparency and animation settings. Including the ones people turn on
      because they need them.
---

## Install

```powershell
irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/install.ps1 | iex
```

No administrator rights. The checksum is verified before anything is written.

## What it looks like

The panel sits on your wallpaper with rounded corners and a soft shadow — no
window frame, no grey box. At the top: the weekday, the date, the time, and a
rail showing the whole day at a glance with one coloured segment per event.
Below that, the meeting that is running right now with a countdown, then the
rest of the day, then what is due.

## Honest limitations

Version 1.0 runs on **Windows only**. The portable core already builds and
tests on macOS and Linux — the interface layer is what still has to follow.

The Microsoft and CalDAV back ends were built against the documented APIs and
their response formats are covered by tests, but they have not yet been run
against a live tenant or server in every configuration. If something is off,
[the log](/guide/troubleshooting) will say what.
