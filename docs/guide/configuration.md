# Configuration

Settings live in `%APPDATA%\TPMPlaner\config.json`. Right-click the widget and
choose *Edit configuration* to open it.

Changes are picked up **within a minute, without a restart**. A change of
`scale` or reading direction rebuilds the renderer; everything else applies on
the next redraw.

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

`"system"` follows your display language. The interface text ships in twenty
catalogues:

| | |
|---|---|
| Latin script | English, German, French, Spanish, Italian, Portuguese (`pt` and `pt-BR`), Dutch, Swedish, Polish, Czech, Turkish |
| Cyrillic | Russian, Ukrainian |
| CJK | Japanese, Simplified Chinese, Traditional Chinese, Korean |
| Right to left | Arabic, Hebrew |

Any other locale gets English labels.

Regional variants share a catalogue — `de-AT` and `de-CH` both get German —
with two exceptions. Portuguese splits by region, because Brazilian and
European Portuguese differ in vocabulary and in whether zero takes the
singular. Chinese splits by **script**, so `zh-TW`, `zh-HK` and `zh-MO` reach
the traditional catalogue and `zh-CN`, `zh-SG` and a bare `zh` the simplified
one; an explicit `zh-Hans` or `zh-Hant` outranks the region.

**Dates, times and reading direction always follow the chosen locale**, catalogue
or not. Set `"language": "th-TH"` and you get Thai date formatting with
English labels. Set `"fa-IR"` and the entire layout mirrors, still with English
labels — while `"ar-SA"` now mirrors *and* speaks Arabic.

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

## Files in the data directory

| File | Contents |
|---|---|
| `config.json` | These settings |
| `client_secret.json` | Your Google OAuth client |
| `microsoft_client.json` | Your Azure application id |
| `caldav-<id>.json` | CalDAV server and user name |
| `token-<id>.bin` | Encrypted credentials, bound to your Windows account |
| `cache.json` | Last synced day, so something shows at start-up |
| `tpmplaner.log` | Log, rotated at 256 KB |
