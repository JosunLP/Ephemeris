# Connecting accounts

Ephemeris talks to three kinds of service. You can use several at once, and of
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
  { "kind": "caldav",    "id": "icloud",  "label": "iCloud" }
]
```

- **`kind`** — `google`, `microsoft` or `caldav`
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

## Choosing which calendars appear

Once an account has synced once, right-click the widget: **Calendars** and
**Task lists** list everything the accounts offer, with a tick next to the ones
in use. Turning them on and off there writes to the settings for you.

By default every calendar and list is used.
