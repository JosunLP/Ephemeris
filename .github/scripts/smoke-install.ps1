# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Ephemeris contributors
#
# Installs Ephemeris for real, checks what landed on disk and in the registry,
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
#
# Note for a run on a real machine: the autostart Run key is exported, emptied
# for the duration of the test and imported back afterwards. Nothing else on
# the machine is touched beyond a normal install and uninstall.

#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallScript,
    [Parameter(Mandatory)][string]$UninstallScript,
    # Which release the installer should pull its binary from.
    [string]$Version = 'latest',
    [string]$Repo = 'JosunLP/Ephemeris'
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Ephemeris'
$exePath = Join-Path $installDir 'ephemeris.exe'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Ephemeris.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Ephemeris'

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

# The installer has to *create* the Run key, not just write into it: a profile
# on which nothing has ever registered for autostart does not have that key,
# and the install then aborted there -- after the binary was already in place.
# A machine that has the key cannot show this, so the key is taken away for the
# duration of the test. It is exported first and put back at the end, because
# this script is also meant to be runnable on a real machine.
$runKeyNative = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Run'
$runKeyBackup = Join-Path ([IO.Path]::GetTempPath()) 'smoke-run-key.reg'
$runKeyExisted = Test-Path $runKey

# reg.exe writes to stderr on paths that are not failures here, and redirected
# native stderr is a terminating error while $ErrorActionPreference is 'Stop'.
# The assignment below is function-scoped, so it only covers this call.
function Invoke-Reg {
    param([Parameter(Mandatory)][string[]]$RegArgs)
    $ErrorActionPreference = 'Continue'
    & reg.exe @RegArgs 2>&1 | Out-Null
    return $LASTEXITCODE
}

Remove-Item -LiteralPath $runKeyBackup -Force -ErrorAction SilentlyContinue
if ($runKeyExisted) {
    if ((Invoke-Reg @('export', $runKeyNative, $runKeyBackup, '/y')) -ne 0) {
        throw "could not back up $runKeyNative"
    }
}

# Puts the machine back as it was found: whatever the test wrote is dropped
# along with the key, then the export goes back on over the empty slate.
# Reached from every way out of this script, a failing check and a throw
# included: a trap covers the whole scope it is written in, wherever in that
# scope the error came from. That includes the failed export above -- which is
# what the first line guards against. A key that was there but has no backup
# means the export died before anything was taken away, so there is nothing to
# undo here and nothing to put back either.
function Restore-RunKey {
    if ($runKeyExisted -and -not (Test-Path -LiteralPath $runKeyBackup)) { return }
    Remove-Item -Path $runKey -Recurse -Force -ErrorAction SilentlyContinue
    if (-not $runKeyExisted) { return }
    if ((Invoke-Reg @('import', $runKeyBackup)) -ne 0) {
        Write-Host "  WARN  $runKeyNative not restored, backup kept at $runKeyBackup" -ForegroundColor Yellow
        return
    }
    Remove-Item -LiteralPath $runKeyBackup -Force -ErrorAction SilentlyContinue
}
trap { Restore-RunKey; break }

Remove-Item -Path $runKey -Recurse -Force -ErrorAction SilentlyContinue
Check 'the test starts without a Run key' { -not (Test-Path $runKey) }

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
# Both names, for the same reason the installer tries both: a release published
# before the rename carries its assets under the former one, and this check has
# to look up whichever the release actually has -- otherwise it reports a
# checksum mismatch for a release that is perfectly intact, which is the exact
# failure this whole job exists to catch.
$published = $null
foreach ($candidate in @("ephemeris-$arch.exe", "tpmplaner-$arch.exe")) {
    try {
        Invoke-WebRequest -Uri "$base/$candidate.sha256" -OutFile $sumFile -UseBasicParsing
        $published = ((Get-Content $sumFile -Raw) -split '\s+')[0].ToLower()
        break
    } catch {
        Write-Host "    no $candidate.sha256 in this release"
    }
}

Check 'the published checksum is a sha256' { $published -match '^[0-9a-f]{64}$' }
Check 'the installed binary matches the published checksum' {
    (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLower() -eq $published
}

Check 'a Start menu shortcut points at the binary' {
    (Test-Path $shortcut) -and
    (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath -eq $exePath
}
Check 'the uninstall entry is registered' {
    (Get-ItemProperty -Path $uninstallKey -ErrorAction SilentlyContinue).DisplayName -eq 'Ephemeris'
}
Check 'the uninstaller was placed next to the binary' {
    Test-Path (Join-Path $installDir 'uninstall.ps1')
}
Check '-NoAutostart left no autostart entry' {
    $null -eq (Get-ItemProperty -Path $runKey -Name Ephemeris -ErrorAction SilentlyContinue)
}
# Creating the key belongs to the autostart branch alone, so -NoAutostart must
# not leave one behind either.
Check '-NoAutostart did not create the Run key' { -not (Test-Path $runKey) }

# --- reinstall over an existing install ---------------------------------------
# The common case for an update, and the one where a locked or read-only file
# would surface. Also the only way to cover the autostart branch.
Write-Host "==> Reinstalling with autostart" -ForegroundColor Cyan
& $install -Version $Version -NoStart

Check 'reinstalling over an existing install works' { Test-Path $exePath }
Check 'the binary still matches the published checksum' {
    (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLower() -eq $published
}
# The Run key was removed above, so this is also the check that the installer
# creates it rather than failing on a profile that never had one.
Check 'autostart is registered in a Run key the installer created' {
    (Get-ItemProperty -Path $runKey -Name Ephemeris -ErrorAction SilentlyContinue).Ephemeris `
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
    $null -eq (Get-ItemProperty -Path $runKey -Name Ephemeris -ErrorAction SilentlyContinue)
}
Check 'the uninstall entry is gone' { -not (Test-Path $uninstallKey) }

Restore-RunKey

Write-Host ''
if ($failures.Count -gt 0) {
    Write-Host "$($failures.Count) check(s) failed:" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}
Write-Host 'All smoke checks passed.' -ForegroundColor Green
