---
layout: home

hero:
  name: Ephemeris
  text: Your day, on your desktop
  image:
    src: /logo.svg
    alt: ''
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
      link: https://github.com/JosunLP/Ephemeris

# The icons are drawn here rather than linked, so they inherit the text colour
# and need no request of their own. Each is the plainest line drawing of the
# thing its card is about; the fourth is the widget's own day rail.
features:
  - icon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="6" width="14" height="14" rx="3"/><path d="M7 3v3M13 3v3M3 10.5h14"/><path d="M17 7h1.5A2.5 2.5 0 0 1 21 9.5V17"/></svg>'
    title: Every calendar at once
    details: >
      Google, Microsoft (Outlook, Teams and To Do) and any CalDAV server —
      iCloud, Nextcloud, Fastmail — queried in parallel. One account failing
      never blanks the rest.
  - icon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="3.5" width="13" height="11" rx="2.5"/><path d="M8.5 20.5h10a2 2 0 0 0 2-2v-10"/></svg>'
    title: Out of the way by design
    details: >
      Sits below every normal window but above the desktop. No taskbar entry,
      no Alt-Tab entry, and clicking it never steals focus from what you were
      doing. One shortcut brings it forward when you want it.
  - icon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M4 17a8 8 0 1 1 16 0"/><path d="M12 17 7.8 13.6"/><circle cx="12" cy="17" r="1.2" fill="currentColor" stroke="none"/></svg>'
    title: Costs nothing while idle
    details: >
      Measured 0.03 seconds of CPU across 40 seconds and under 4 MB resident.
      There is no render loop: it draws when something changes, and stops.
  - icon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="7.75" stroke-dasharray="8.2 3.9"/><path d="M12 12 15 8.4"/><circle cx="12" cy="12" r="1.2" fill="currentColor" stroke="none"/></svg>'
    title: Sees what a list hides
    details: >
      A day rail showing the shape of your day, a marker for right now,
      overlapping meetings flagged, and a preview of tomorrow once today is
      done.
  - icon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.6 2.6 2.6 14.4 0 17M12 3.5c-2.6 2.6-2.6 14.4 0 17"/></svg>'
    title: Speaks your system's language
    details: >
      Twenty interface catalogues, Arabic and Hebrew included — and dates,
      times and reading direction come from the operating system, so a 12-hour
      clock, a Hijri calendar or a right-to-left layout are all simply correct
      even in a language nobody has translated yet.
  - icon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 3.5a8.5 8.5 0 0 1 0 17z" fill="currentColor" stroke="none"/></svg>'
    title: Fits your theme
    details: >
      Follows light and dark, the accent colour, contrast themes, and the
      transparency and animation settings. Including the ones people turn on
      because they need them.
---

## What it looks like

The panel sits on your wallpaper with rounded corners and a soft shadow — no
window frame, no grey box.

<WidgetPreview />

At the top: the weekday, the date, the time, and a rail showing the whole day
at a glance with one coloured segment per event. Below that, the meeting that
is running right now with a countdown, then the rest of the day, then what is
due.

## Install

```powershell
irm https://github.com/JosunLP/Ephemeris/releases/latest/download/install.ps1 | iex
```

No administrator rights. The checksum is verified before anything is written.

## Where it runs

<PlatformSupport />

## Honest limitations

The Microsoft and CalDAV back ends were built against the documented APIs and
their response formats are covered by tests, but they have not yet been run
against a live tenant or server in every configuration. If something is off,
[the log](/guide/troubleshooting) will say what.
