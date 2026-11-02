# Connecting accounts

Ephemeris talks to three kinds of service, and reads subscribed calendar feeds
as a fourth. You can use several at once, and of
the same kind more than once — a work Google account and a private one, for
instance. All configured accounts are queried **in parallel**, and one of them
failing never hides the others.

Every account needs a one-time setup, because none of these services let an
application read your calendar without credentials of its own.

## Declaring the accounts

Right-click the widget, choose *Edit configuration*, and add an `accounts`
list:

```jsonc
"accounts": [
  { "kind": "google",    "id": "private", "label": "Private" },
  { "kind": "microsoft", "id": "work",    "label": "Work" },
  { "kind": "caldav",    "id": "icloud",  "label": "iCloud" },
  { "kind": "ics",       "id": "holidays", "label": "Holidays" }
]
```

- **`kind`** — `google`, `microsoft`, `caldav` or `ics`
- **`id`** — free text, used for the credential file name. Keep it stable.
- **`label`** — shown in the widget when more than one account is configured
- **`enabled`** — set to `false` to switch an account off without deleting it

If you leave `accounts` out entirely, the widget behaves as it always did: one
Google account.

The settings file is re-read within a minute; no restart needed.

---

## Google

Reads Google Calendar and Google Tasks.

### 1. Create a project

1. Open the [Google Cloud Console](https://console.cloud.google.com/) and
   create a project.
2. Under **APIs and services → Library**, enable the **Google Calendar API**
   and the **Google Tasks API**.

### 2. Configure the consent screen

Under **APIs and services → OAuth consent screen**, choose user type
*External*, and add these scopes:

- `.../auth/calendar.readonly`
- `.../auth/calendar.events.readonly`
- `.../auth/tasks`

::: danger Publish the app, or sign in every week
Set the publishing status to **In production**. While a project stays in
*Testing*, Google **expires refresh tokens after seven days** and the widget
will ask you to sign in again every week.

The "Google hasn't verified this app" screen you get once afterwards is normal
for a personal project; continue through *Advanced*. Verification is not
required for your own use.
:::

### 3. Download the client

Under **Credentials → Create credentials → OAuth client ID**, pick application
type **Desktop app**, download the JSON and save it as:

```text
%APPDATA%\Ephemeris\client_secret.json
```

Click the refresh symbol in the widget. A browser opens for sign-in, and the
day appears.

### Why tasks need write access

Calendars are read only. Tasks are requested with write access for exactly one
reason: so you can tick a task off from the widget. `tasks.readonly` would not
allow that.

---

## Microsoft — Outlook, Teams and To Do

One account covers all three. There is no separate Teams calendar API: a Teams
meeting **is** an Outlook event that carries a join link. The widget surfaces
that link, so clicking a running meeting joins it rather than opening the
calendar entry.

### 1. Register an application

1. Open the [Azure portal](https://portal.azure.com/) → **Microsoft Entra ID**
   → **App registrations** → **New registration**.
2. Give it any name. Under **Supported account types**, pick whichever matches
   your situation; *Accounts in any organizational directory and personal
   Microsoft accounts* works for most people.
3. Under **Redirect URI**, choose platform **Public client/native** and enter
   `http://localhost`.

::: warning Public client, no secret
Do **not** create a client secret. The widget is a public client using PKCE;
Microsoft rejects a secret for this application type.
:::

### 2. Grant the permissions

Under **API permissions**, add these delegated Microsoft Graph permissions:

- `Calendars.Read`
- `Tasks.ReadWrite`
- `offline_access`

### 3. Save the client id

Copy the **Application (client) ID** from the overview page into:

```text
%APPDATA%\Ephemeris\microsoft_client.json
```

```json
{ "client_id": "00000000-1111-2222-3333-444444444444" }
```

Add a `microsoft` account to `accounts`, refresh, and sign in.

---

## CalDAV — iCloud, Nextcloud, Fastmail and others

Any server speaking [RFC 4791](https://www.rfc-editor.org/rfc/rfc4791). Tested
shapes include iCloud, Nextcloud, Fastmail, Synology and mailbox.org.

Create `%APPDATA%\Ephemeris\caldav-<id>.json`, where `<id>` matches the `id`
in your `accounts` entry:

```json
{
  "url": "https://caldav.example.org",
  "username": "you@example.org",
  "password": "app-specific-password"
}
```

Use an **app-specific password**, not your account password. Every provider
listed here supports them, and it means the credential can be revoked on its
own.

::: tip The password moves out of this file
On the first run the widget encrypts the password into its credential store and
**removes it from the JSON**. That is deliberate: an app password sitting in
plain text next to your settings is careless. Nothing is lost — you only need
to put it back if you reset the account.
:::

### Server URLs

| Service | URL |
|---|---|
| iCloud | `https://caldav.icloud.com` |
| Nextcloud | `https://your.server/remote.php/dav` |
| Fastmail | `https://caldav.fastmail.com` |
| Synology | `https://your.nas:5006` |
| mailbox.org | `https://dav.mailbox.org` |

A bare server address is enough — the widget walks from `current-user-principal`
to your calendar home on its own. If you already have the URL of your calendar
home, or of a single calendar, that works too.

### Recurring events

Expanded by the server, through the standard `<C:expand>` request. Reimplementing
recurrence rules on the client is where hand-written CalDAV clients usually go
wrong, and every server already has that code.

---

## Subscribed calendars — `.ics` and `webcal://`

A calendar published as a file: public holidays, school terms, a sports
fixture list, a team calendar someone shared as a link — or any of your own
calendars, through the private iCal address Google, Outlook, iCloud and
Proton all offer. Read-only, and no sign-in.

Create `%APPDATA%\Ephemeris\ics-<id>.json`, where `<id>` matches the `id` in
your `accounts` entry:

```json
{ "url": "webcal://example.org/holidays.ics" }
```

Several feeds can share one account, each with an optional name and colour:

```json
{
  "feeds": [
    { "url": "https://example.org/holidays.ics", "name": "Holidays", "color": "#33b679" },
    { "url": "https://calendar.google.com/calendar/ical/…/basic.ics" },
    { "url": "exported.ics" }
  ]
}
```

- **`url`** — `https://`, `http://` or `webcal://` (read as `https://`, which is
  what every service handing one out serves). A path, absolute or relative to
  the data folder, or a `file://` address reads a file instead — an export, or a
  calendar some other program keeps up to date on disk.
- **`name`** — optional. Otherwise the name the publisher gave the feed, and
  failing that its host name.
- **`color`** — optional, `#RRGGBB`. Otherwise the publisher's colour, and
  failing that one from a fixed set.

Each feed appears under **Calendars** in the right-click menu like any other
calendar. One feed failing fails its account, so a feed you do not trust to
stay up is better off in an account of its own.

::: warning A private address is a password
Anyone holding the private iCal address of your calendar can read it. That is
why it goes in this file and not in `config.json`, why the calendar id the menu
writes to `config.json` is a hash of the address, and why the log names only
its host. If one leaks, reset it at the service; the old one stops working.
:::

### Recurring events

Expanded by the widget itself — a feed is a file, so there is no server to ask.
Daily, weekly, monthly and yearly rules with their intervals, counts and end
dates are all understood, as are "the last Friday of the month", "every
weekday", excluded dates, and single occurrences that were moved or cancelled.

Time zones come from the feed's own `VTIMEZONE` definitions, so a 09:00 meeting
in Berlin stays at 09:00 Berlin time across a daylight saving change wherever
you are. The few rule forms nobody writes by hand — by week number, by day of
the year, several times a day — are not guessed at: the entry shows its first
occurrence and the log says which rule it was.

### How often

With every sync, but a feed that has not changed is not downloaded again: the
server's `ETag` and `Last-Modified` are sent back, and an unchanged feed costs
one short reply. A file on disk is re-read when its modification time changes.

---

## Choosing which calendars appear

Once an account has synced once, right-click the widget: **Calendars** and
**Task lists** list everything the accounts offer, with a tick next to the ones
in use. Turning them on and off there writes to the settings for you.

By default every calendar and list is used.
