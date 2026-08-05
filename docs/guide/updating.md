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
irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/install.ps1 | iex
```

Your settings and connected accounts are untouched by an update.

## Removing

```powershell
irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/uninstall.ps1 | iex
```

This removes the program, the autostart entry, the Start menu shortcut and the
entry in the Windows app list.

**Settings and credentials are kept**, so reinstalling does not mean signing in
again. To remove those as well:

```powershell
& ([scriptblock]::Create((irm .../uninstall.ps1))) -Purge
```

TPMPlaner also appears in *Settings → Apps → Installed apps*, where the normal
uninstall button runs the same script.

## What is left behind

Without `-Purge`, the data directory survives:

```text
%APPDATA%\TPMPlaner\
  config.json          settings
  token-*.bin          encrypted credentials
  cache.json           last synced day
  tpmplaner.log        log
  client_secret.json   your Google OAuth client
  microsoft_client.json
  caldav-*.json
```

Deleting that folder removes every trace. Revoking the widget's access at the
provider is a separate step — see
[Google](https://myaccount.google.com/permissions) and
[Microsoft](https://myapplications.microsoft.com/).
