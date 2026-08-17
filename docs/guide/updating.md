# Updating and removing

## Updating

The widget checks once a day for a newer release. When one exists, the status
line reads **Version x.y.z available** and the context menu offers *Install
update*.

It never installs anything on its own. A widget that silently replaces its own
binary is the kind of surprise nobody wants, so the decision stays yours.

Choosing *Install update* opens a visible PowerShell window running the same
installer as a fresh install — visible on purpose, because it downloads and
executes a script from the internet and you should be able to watch it. The
widget closes so the file can be replaced, and the installer starts it again.

To update by hand at any time:

```powershell
irm https://github.com/JosunLP/Ephemeris/releases/latest/download/install.ps1 | iex
```

Your settings and connected accounts are untouched by an update.

### Updating from a version called TPMPlaner

The widget was renamed to Ephemeris, and the update handles it: install as
above, and the first start moves your settings, your autostart entry and your
saved calendar credentials to the new location. Your accounts stay connected
and there is nothing to do by hand.

On Windows the installer also removes the old program directory, its start menu
shortcut and its entry in the app list, so you are not left with two of
everything. If anything could not be moved it is left exactly where it was, and
the reason is in the log — *Open log* in the context menu.

## Removing

```powershell
irm https://github.com/JosunLP/Ephemeris/releases/latest/download/uninstall.ps1 | iex
```

This removes the program, the autostart entry, the Start menu shortcut and the
entry in the Windows app list.

**Settings and credentials are kept**, so reinstalling does not mean signing in
again. To remove those as well:

```powershell
& ([scriptblock]::Create((irm .../uninstall.ps1))) -Purge
```

Ephemeris also appears in *Settings → Apps → Installed apps*, where the normal
uninstall button runs the same script.

## What is left behind

Without `-Purge`, the data directory survives:

```text
%APPDATA%\Ephemeris\
  config.json          settings
  token-*.bin          encrypted credentials
  cache.json           last synced day
  ephemeris.log        log
  client_secret.json   your Google OAuth client
  microsoft_client.json
  caldav-*.json
```

Deleting that folder removes every trace. Revoking the widget's access at the
provider is a separate step — see
[Google](https://myaccount.google.com/permissions) and
[Microsoft](https://myapplications.microsoft.com/).
