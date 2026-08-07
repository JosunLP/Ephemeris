# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 TPMPlaner contributors
#
# Installs TPMPlaner for real, checks what landed on disk and in the registry,
# then uninstalls it and checks that nothing was left behind.
#
# This exists because v1.0.0 shipped an installer that could not install: the
# published checksum was read out of `Invoke-WebRequest`'s `.Content`, which
# for GitHub's `application/octet-stream` assets is a byte array on PowerShell
# 7 and an empty string on Windows PowerShell 5.1 -- never the text of the
# file. Every unit-level check passed, because the thing that was broken only
# shows up when a real asset is fetched over a real HTTP response. So this
# runs the actual script against an actual release.
#
#   .github/scripts/smoke-install.ps1 -InstallScript scripts/install.ps1 `
#       -UninstallScript scripts/uninstall.ps1
#
# `-InstallScript` and `-UninstallScript` take either a local path or an https
# URL, so the same file can check the scripts in the tree (on a pull request)
# and the scripts actually attached to a release (after publishing).

#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallScript,
    [Parameter(Mandatory)][string]$UninstallScript,
    # Which release the installer should pull its binary from.
    [string]$Version = 'latest',
    [string]$Repo = 'JosunLP/TPMPlaner'
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\TPMPlaner'
$exePath = Join-Path $installDir 'tpmplaner.exe'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\TPMPlaner.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\TPMPlaner'

$failures = @()
function Check($description, [scriptblock]$condition) {
    $ok = $false
    try { $ok = [bool](& $condition) } catch { $ok = $false }
    if ($ok) {
        Write-Host "  PASS  $description" -ForegroundColor Green
    } else {
        Write-Host "  FAIL  $description" -ForegroundColor Red
        $script:failures += $description
    }
}

# A local path is dot-sourced; a URL is fetched and turned into a scriptblock,
# which is exactly how the documented one-liner runs it.
function Get-Script($reference) {
    if ($reference -match '^https?://') {
        return [scriptblock]::Create((Invoke-RestMethod -Uri $reference -UseBasicParsing))
    }
    if (-not (Test-Path $reference)) { throw "No such script: $reference" }
    return [scriptblock]::Create((Get-Content -Raw -LiteralPath $reference))
}

Write-Host "==> Smoke test ($Version)" -ForegroundColor Cyan
Write-Host "    install:   $InstallScript"
Write-Host "    uninstall: $UninstallScript"

# Starting from an existing install would let a stale binary satisfy every
# check below.
if (Test-Path $installDir) { throw "$installDir exists before the test started" }

$install = Get-Script $InstallScript
$uninstall = Get-Script $UninstallScript

# --- install -----------------------------------------------------------------
# -NoStart because the runner has no interactive desktop to draw on, and the
# process would hold a lock on its own binary for the rest of the test.
Write-Host "==> Installing" -ForegroundColor Cyan
& $install -Version $Version -NoStart -NoAutostart

Check 'the binary is installed' { Test-Path $exePath }

# The whole point of the installer's checksum step. If it silently degraded to
# "download whatever and copy it", this is what would notice.
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'ARM64' { 'aarch64-pc-windows-msvc' }
    default { 'x86_64-pc-windows-msvc' }
}
$base = if ($Version -eq 'latest') {
    "https://github.com/$Repo/releases/latest/download"
} else {
    "https://github.com/$Repo/releases/download/$Version"
}
$sumFile = Join-Path ([IO.Path]::GetTempPath()) "smoke-$arch.sha256"
Invoke-WebRequest -Uri "$base/tpmplaner-$arch.exe.sha256" -OutFile $sumFile -UseBasicParsing
$published = ((Get-Content $sumFile -Raw) -split '\s+')[0].ToLower()

Check 'the published checksum is a sha256' { $published -match '^[0-9a-f]{64}$' }
Check 'the installed binary matches the published checksum' {
    (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLower() -eq $published
}

Check 'a Start menu shortcut points at the binary' {
    (Test-Path $shortcut) -and
    (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath -eq $exePath
}
Check 'the uninstall entry is registered' {
    (Get-ItemProperty -Path $uninstallKey -ErrorAction SilentlyContinue).DisplayName -eq 'TPMPlaner'
}
Check 'the uninstaller was placed next to the binary' {
    Test-Path (Join-Path $installDir 'uninstall.ps1')
}
Check '-NoAutostart left no autostart entry' {
    $null -eq (Get-ItemProperty -Path $runKey -Name TPMPlaner -ErrorAction SilentlyContinue)
}

# --- reinstall over an existing install ---------------------------------------
# The common case for an update, and the one where a locked or read-only file
# would surface. Also the only way to cover the autostart branch.
Write-Host "==> Reinstalling with autostart" -ForegroundColor Cyan
& $install -Version $Version -NoStart

Check 'reinstalling over an existing install works' { Test-Path $exePath }
Check 'the binary still matches the published checksum' {
    (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLower() -eq $published
}
Check 'autostart is registered' {
    (Get-ItemProperty -Path $runKey -Name TPMPlaner -ErrorAction SilentlyContinue).TPMPlaner `
        -eq "`"$exePath`""
}

# --- uninstall ----------------------------------------------------------------
# Run the copy from outside the install directory: the copy the installer put
# *inside* it hands the removal to a detached process, which would make the
# checks below a race.
Write-Host "==> Uninstalling" -ForegroundColor Cyan
& $uninstall -Force

Check 'the program directory is gone' { -not (Test-Path $installDir) }
Check 'the Start menu shortcut is gone' { -not (Test-Path $shortcut) }
Check 'the autostart entry is gone' {
    $null -eq (Get-ItemProperty -Path $runKey -Name TPMPlaner -ErrorAction SilentlyContinue)
}
Check 'the uninstall entry is gone' { -not (Test-Path $uninstallKey) }

Write-Host ''
if ($failures.Count -gt 0) {
    Write-Host "$($failures.Count) check(s) failed:" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}
Write-Host 'All smoke checks passed.' -ForegroundColor Green
