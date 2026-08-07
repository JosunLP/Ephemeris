# Configuration

Settings live in `%APPDATA%\TPMPlaner\config.json`. Right-click the widget and
choose *Edit configuration* to open it.

Changes are picked up **within a minute, without a restart**. A change of
`scale`, of `appearance` or of reading direction rebuilds the renderer;
everything else applies on the next redraw.

If the file has a syntax error, the status line says **Invalid configuration**
and a click opens the file. The exact line is in the log. A byte order mark —
which several Windows editors add — is tolerated.

## Complete reference

```jsonc
{
  // Window position in physical pixels. Written when you move the widget.
  "x": 1152,
  "y": 50,

  // Size of the visible glass body in device independent pixels.
  "width": 380.0,
  "height": 620.0,

  // How often to fetch, in minutes.
  "sync_minutes": 30,

  // Which services to talk to. Empty means "one Google account".
  "accounts": [
    { "kind": "google", "id": "private", "label": "Private", "enabled": true }
  ],

  // Which calendars and task lists to show. Empty means all of them.
  // Easier to set through the context menu than by hand.
  "calendar_ids": [],
  "tasklist_ids": [],

  // Show tasks that have no due date at all.
  "show_undated_tasks": false,
  // Keep events that already ended visible, dimmed.
  "show_past_events": true,
  // Hide events you declined yourself.
  "hide_declined": true,

  // Panel opacity, 0.15 to 1.0. Ignored when the system has transparency off.
  "opacity": 0.82,
  // "none" draws the widget's own shadow and rounded corners.
  // "acrylic" uses the Windows system backdrop instead.
  "backdrop": "none",
  // Extra scaling on top of the monitor DPI, 0.6 to 3.0.
  "scale": 1.0,

  // "system", or a BCP-47 tag such as "en-US", "fr-FR", "ar-SA".
  "language": "system",
  // "system", "dark", "light" or "contrast".
  "theme": "system",
  // "system", or a fixed "#RRGGBB".
  "accent": "system",

  // Everything the system does not decide for you. See "Customising the
  // look" below. "system" means nothing is customised.
  "appearance": "system",

  // Shortcut that brings the widget forward. Empty disables it.
  "peek_hotkey": "Ctrl+Alt+Shift+K",
  // How long it stays in front, 1 to 60 seconds.
  "peek_seconds": 5,

  // Grace period before a ticked task is sent. 0 disables undo.
  "undo_seconds": 4
}
```

Out-of-range values are clamped rather than rejected, so a typo cannot produce
a zero-sized window or a storm of requests.

## Language

`"system"` follows your display language. The interface text ships in English,
German, French, Spanish and Italian; any other locale gets English labels.

**Dates, times and reading direction always follow the chosen locale**, catalogue
or not. Set `"language": "ja-JP"` and you get Japanese date formatting with
English labels. Set `"ar-SA"` and the entire layout mirrors, with a Hijri
calendar, still with English labels.

That distinction is the difference between a translated program and an
internationalised one: a hard-coded `HH:MM` would show a user in the United
States "20:09" instead of "8:09 PM".

## Appearance

`"theme": "contrast"` forces the flat, opaque, system-colour rendering that a
Windows contrast theme would produce, without switching your whole desktop over.
Useful if you want maximum contrast in this one window.

`"accent"` takes the system colour by default. Because a system accent may be
black, white or neon yellow — all valid settings, none usable as a text colour
— only its **lightness** is corrected into a readable band; the hue is left
alone so you still recognise your colour.

If your accent is red, the overdue marker automatically moves to amber or
magenta. Otherwise "now" and "overdue" would be the same colour and the two
meanings could not be told apart.

## Customising the look

Following the system is the right default — the widget should feel like part
of it — but a stock desktop is not everybody's desktop. A wallpaper the panel
disappears into, a glass surface next to an otherwise flat theme, calendar
colours the provider chose that are indistinguishable at 82% opacity, or
simply a font too small next to everything else on screen: `"appearance"`
covers those.

Everything in it is optional and everything defaults to `"system"`, which
means "leave it to the derivation rules above".

```jsonc
"appearance": {
  // "system" (follows "backdrop"), "aero", "flat" or "borderless".
  // "aero" is the glass gadget look: soft shadow, double edge, sheen.
  // "flat" keeps the border and drops the rest. "borderless" drops that too.
  "surface": "system",

  // "system", "compact", "normal" or "roomy". Spacing only — type is not
  // touched, because a compact layout with clipped labels helps nobody.
  "density": "system",

  // A font family name, or "system". A name Windows does not have falls
  // back to the system font rather than failing.
  "font_family": "system",
  // Added to every font size, -3 to +8, independent of "scale". "scale"
  // moves the whole layout; sometimes only the text needs to grow. The
  // boxes that hold text grow with it.
  "font_size_offset": 0,
  // Weight of the SCHEDULE and TASKS labels: "system", "regular",
  // "medium", "semibold" or "bold".
  "header_weight": "system",

  // Each is "system" or "#RRGGBB".
  "colors": {
    "panel": "system",       // base of the glass body
    "text": "system",        // primary text
    "text_muted": "system",  // times, meta lines, footer
    "separator": "system",   // the rules between sections
    "now": "system",         // the running event, the now line, day progress
    "overdue": "system",     // overdue tasks
    "conflict": "system"     // the badge on overlapping events
  },

  // Provider colours are not always distinguishable at 0.82 opacity.
  // Keyed by calendar id (as in "calendar_ids") or by the name shown in
  // the widget, whichever is easier to type.
  "calendar_colors": {
    "work@example.com": "#FF8800",
    "Family": "#00AA55"
  }
}
```

Only the colours a person reasons about are exposed. The shades between them —
the sheen, the hover highlight, the secondary and faint text — are derived from
these, so the set stays coherent instead of becoming twenty knobs that can
contradict each other.

### Named themes

A whole set of choices belongs in one file rather than a growing pile of keys.
Put it next to `config.json` as `<name>.theme.json` and point at it by name:

```jsonc
"appearance": "midnight"
```

reads `%APPDATA%\TPMPlaner\midnight.theme.json`, whose contents are the object
above. That file can be shared as-is. A theme that is missing or malformed
falls back to the system look and says so in the log.

### What always wins

- **A contrast theme.** With one active, or with `"theme": "contrast"`, the
  custom colours and the surface style are ignored: the colours come from the
  system because there are several such schemes with entirely different
  palettes, and the flatness is the point rather than a style. Typography and
  density still apply — nothing about a larger font or more room works against
  contrast, and whoever needs one often needs the other.
- **Readability.** A custom colour is corrected until it reads against the
  surface it sits on, the same bargain the accent has always struck: the
  lightness moves, the hue does not, so you still recognise your colour. A
  settings file must not be able to produce invisible text. The correction is
  written to the log when it happens, so it does not look like the setting
  having no effect.
- **The panel colour over the theme.** Ask for a near-white panel while the
  dark theme is active and the text, separators and edges follow it — a white
  separator on light glass is invisible whatever the theme was called.

A value that is not a colour is treated like any other out-of-range value: it
falls back to the system one, and the log names it.

## Files in the data directory

| File | Contents |
|---|---|
| `config.json` | These settings |
| `<name>.theme.json` | A named appearance, if you use one |
| `client_secret.json` | Your Google OAuth client |
| `microsoft_client.json` | Your Azure application id |
| `caldav-<id>.json` | CalDAV server and user name |
| `token-<id>.bin` | Encrypted credentials, bound to your Windows account |
| `cache.json` | Last synced day, so something shows at start-up |
| `tpmplaner.log` | Log, rotated at 256 KB |
