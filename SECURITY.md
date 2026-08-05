# Security policy

## Reporting a vulnerability

Please report security issues privately through
[GitHub's advisory form](https://github.com/JosunLP/TPMPlaner/security/advisories/new)
rather than opening a public issue.

Expect an acknowledgement within a few days. If the issue is confirmed, a fix
and an advisory follow; you will be credited unless you prefer otherwise.

## What the widget holds

TPMPlaner stores credentials for the calendar accounts you connect. Understanding
what is where matters for judging impact:

| Item                      | Where                                         | Protection                                                       |
| ------------------------- | --------------------------------------------- | ---------------------------------------------------------------- |
| OAuth refresh tokens      | `token-<account>.bin` in the data directory   | Encrypted with Windows DPAPI, bound to your user account         |
| CalDAV password           | same                                          | Same encryption; moved out of the settings file on first run     |
| OAuth access tokens       | memory only                                   | Never written to disk                                            |
| Client id and secret      | `client_secret.json`, `microsoft_client.json` | Plain text — these are not secrets for installed applications    |
| Calendar and task content | `cache.json`                                  | Plain text, so the widget can show the day before the first sync |

The cache holds event titles, locations and task names for the current day. If
that is sensitive in your setting, delete `cache.json` and be aware it will be
rewritten on the next sync.

## Scopes requested

- Google: `calendar.readonly`, `calendar.events.readonly`, `tasks`
- Microsoft: `Calendars.Read`, `Tasks.ReadWrite`, `offline_access`

Write access is requested for tasks only, and only so a task can be ticked off
from the widget. Calendars are read only throughout.

## Supported versions

The latest release receives fixes. Older versions do not.
