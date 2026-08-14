# Getting started

## Requirements

- Windows 10 version 1809 or newer, or Windows 11
- A graphics adapter with Direct3D 11. Without one the widget falls back to
  software rendering, which works but uses more power.

## Install

```powershell
irm https://github.com/JosunLP/Ephemeris/releases/latest/download/install.ps1 | iex
```

This installs into `%LOCALAPPDATA%\Programs\Ephemeris`, adds a Start menu
entry, enables autostart and launches the widget. Nothing outside your user
profile is touched and no administrator rights are needed.

The SHA-256 checksum published with the release is verified **before** anything
is written to disk, so a truncated download or a swapped file cannot be
installed.

### Options

```powershell
# A specific version instead of the latest
& ([scriptblock]::Create((irm .../install.ps1))) -Version v1.1.0

# Install without starting it or adding it to autostart
& ([scriptblock]::Create((irm .../install.ps1))) -NoStart -NoAutostart
```

### Manual install

Download `ephemeris-x86_64-pc-windows-msvc.exe` from the
[releases page](https://github.com/JosunLP/Ephemeris/releases/latest), verify
it against the `.sha256` file next to it, rename it to `ephemeris.exe` and run
it. It is a single self-contained binary of roughly 1.7 MB with no runtime to
install.

## First run

On first start the widget appears in the top right corner of your primary
monitor and reports **Setup required** in its status line, because no calendar
account is connected yet.

Two things are worth doing before you connect anything:

### Look at it first

```powershell
$env:EPHEMERIS_DEMO = "1"; & "$env:LOCALAPPDATA\Programs\Ephemeris\ephemeris.exe"
```

Preview mode fills the widget with sample data and synchronises nothing. Use it
to settle on size, position, scaling and opacity before dealing with API
credentials. Close it and start normally when you are done.

### Place it

- **Drag anywhere** on the panel to move it.
- **Drag an edge or corner** to resize it.
- Right-click for the menu; *Reset position* puts it back in the corner.

Position and size are remembered.

## Connect a calendar

Continue with [connecting accounts](/guide/accounts). Google, Microsoft and
CalDAV each need a one-time setup step, and each has one trap worth knowing
about in advance.
