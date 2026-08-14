# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Ephemeris contributors
#
# Removes Ephemeris. Settings and the stored calendar credentials are kept
# unless -Purge is given, so reinstalling does not mean signing in again.
#
#   irm https://github.com/JosunLP/Ephemeris/releases/latest/download/uninstall.ps1 | iex

#Requires -Version 5.1
[CmdletBinding()]
param(
    # Also delete settings, cached agenda, log and stored credentials.
    [switch]$Purge,
    # Skip the confirmation prompt.
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Ephemeris'
$dataDir = Join-Path $env:APPDATA 'Ephemeris'
# The widget was called TPMPlaner until the rename. An installation that never
# started under the new name still has all of its state under the old one, and
# an uninstaller that only knew the new name would leave every bit of it behind
# while reporting success.
$legacyInstallDir = Join-Path $env:LOCALAPPDATA 'Programs\TPMPlaner'
$legacyDataDir = Join-Path $env:APPDATA 'TPMPlaner'

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Write-Note($text) { Write-Host "    $text" -ForegroundColor DarkGray }

if (-not $Force) {
    $what = if ($Purge) { 'Ephemeris including all settings and credentials' } else { 'Ephemeris' }
    $answer = Read-Host ('Remove {0}? [y/N]' -f $what)
    if ($answer -notmatch '^[yY]') {
        Write-Host 'Cancelled.'
        return
    }
}

Write-Step 'Stopping the widget'
Get-Process ephemeris, tpmplaner -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

Write-Step 'Removing autostart entry'
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'Ephemeris' -ErrorAction SilentlyContinue
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'TPMPlaner' -ErrorAction SilentlyContinue

Write-Step 'Removing Start menu shortcut'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Ephemeris.lnk'
Remove-Item $shortcut -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\TPMPlaner.lnk') `
    -Force -ErrorAction SilentlyContinue

Write-Step 'Removing registry entry'
Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Ephemeris' `
    -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\TPMPlaner' `
    -Recurse -Force -ErrorAction SilentlyContinue

Write-Step 'Removing program files'
# This script may live inside the directory it is deleting, so the removal is
# handed to a detached process that outlives it.
if (Test-Path $installDir) {
    $self = $MyInvocation.MyCommand.Path
    if ($self -and $self.StartsWith($installDir, [StringComparison]::OrdinalIgnoreCase)) {
        # The path is quoted for the child's parser, so a single quote in it —
        # an account named O'Brien is enough — has to be doubled or the child
        # command is a parse error, silently, behind -WindowStyle Hidden.
        $quoted = $installDir -replace "'", "''"
        Start-Process powershell -WindowStyle Hidden -ArgumentList @(
            '-NoProfile', '-Command',
            "Start-Sleep -Seconds 2; Remove-Item -LiteralPath '$quoted' -Recurse -Force -ErrorAction SilentlyContinue"
        )
        Write-Note 'Program directory will be removed in a moment.'
    } else {
        Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
# Never the directory this script is running from, so it needs none of the care
# above.
Remove-Item $legacyInstallDir -Recurse -Force -ErrorAction SilentlyContinue

if ($Purge) {
    Write-Step 'Removing settings and credentials'
    Remove-Item $dataDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $legacyDataDir -Recurse -Force -ErrorAction SilentlyContinue
} else {
    Write-Note "Settings kept in $dataDir"
    if (Test-Path $legacyDataDir) { Write-Note "and in $legacyDataDir" }
    Write-Note 'Run with -Purge to remove them as well.'
}

Write-Host ''
Write-Host 'Ephemeris removed.' -ForegroundColor Green
