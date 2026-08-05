# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 TPMPlaner contributors
#
# Removes TPMPlaner. Settings and the stored calendar credentials are kept
# unless -Purge is given, so reinstalling does not mean signing in again.
#
#   irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/uninstall.ps1 | iex

#Requires -Version 5.1
[CmdletBinding()]
param(
    # Also delete settings, cached agenda, log and stored credentials.
    [switch]$Purge,
    # Skip the confirmation prompt.
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\TPMPlaner'
$dataDir = Join-Path $env:APPDATA 'TPMPlaner'

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Write-Note($text) { Write-Host "    $text" -ForegroundColor DarkGray }

if (-not $Force) {
    $what = if ($Purge) { 'TPMPlaner including all settings and credentials' } else { 'TPMPlaner' }
    $answer = Read-Host "Remove $what? [y/N]"
    if ($answer -notmatch '^[yY]') {
        Write-Host 'Cancelled.'
        return
    }
}

Write-Step 'Stopping the widget'
Get-Process tpmplaner -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

Write-Step 'Removing autostart entry'
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'TPMPlaner' -ErrorAction SilentlyContinue

Write-Step 'Removing Start menu shortcut'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\TPMPlaner.lnk'
Remove-Item $shortcut -Force -ErrorAction SilentlyContinue

Write-Step 'Removing registry entry'
Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\TPMPlaner' `
    -Recurse -Force -ErrorAction SilentlyContinue

Write-Step 'Removing program files'
# This script may live inside the directory it is deleting, so the removal is
# handed to a detached process that outlives it.
if (Test-Path $installDir) {
    $self = $MyInvocation.MyCommand.Path
    if ($self -and $self.StartsWith($installDir, [StringComparison]::OrdinalIgnoreCase)) {
        Start-Process powershell -WindowStyle Hidden -ArgumentList @(
            '-NoProfile', '-Command',
            "Start-Sleep -Seconds 2; Remove-Item -LiteralPath '$installDir' -Recurse -Force -ErrorAction SilentlyContinue"
        )
        Write-Note 'Program directory will be removed in a moment.'
    } else {
        Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($Purge) {
    Write-Step 'Removing settings and credentials'
    Remove-Item $dataDir -Recurse -Force -ErrorAction SilentlyContinue
} else {
    Write-Note "Settings kept in $dataDir"
    Write-Note 'Run with -Purge to remove them as well.'
}

Write-Host ''
Write-Host 'TPMPlaner removed.' -ForegroundColor Green
