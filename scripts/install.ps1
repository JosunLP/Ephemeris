# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 TPMPlaner contributors
#
# One-line install:
#   irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/install.ps1 | iex
#
# Installs into the user profile. No administrator rights are needed and
# nothing outside %LOCALAPPDATA% is touched, so uninstalling is a matter of
# deleting one directory and one registry value.

#Requires -Version 5.1
[CmdletBinding()]
param(
    # Pin a specific release, for example "v0.2.0". Defaults to the latest.
    [string]$Version = 'latest',
    # Install without starting the widget or adding it to autostart.
    [switch]$NoStart,
    [switch]$NoAutostart
)

$ErrorActionPreference = 'Stop'
$repo = 'JosunLP/TPMPlaner'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\TPMPlaner'
$exePath = Join-Path $installDir 'tpmplaner.exe'

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Write-Note($text) { Write-Host "    $text" -ForegroundColor DarkGray }

# TLS 1.2 is not the default on stock Windows PowerShell 5.1 and GitHub
# requires it.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# Match the running machine, not the build host: Windows on ARM runs x64
# binaries through emulation, but the native build is considerably faster.
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'ARM64' { 'aarch64-pc-windows-msvc' }
    default { 'x86_64-pc-windows-msvc' }
}
$assetName = "tpmplaner-$arch.exe"

Write-Step "Resolving release ($Version, $arch)"
$base = if ($Version -eq 'latest') {
    "https://github.com/$repo/releases/latest/download"
} else {
    "https://github.com/$repo/releases/download/$Version"
}

$temp = Join-Path ([IO.Path]::GetTempPath()) ("tpmplaner-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $temp | Out-Null
try {
    $downloaded = Join-Path $temp $assetName
    Write-Step "Downloading $assetName"
    Invoke-WebRequest -Uri "$base/$assetName" -OutFile $downloaded -UseBasicParsing

    # Verify before anything is written to the install directory. A truncated
    # download or a swapped asset must never reach disk as an executable.
    #
    # The checksum is fetched to a file rather than read from `.Content`.
    # GitHub serves release assets as application/octet-stream, and for a
    # non-text content type `.Content` is not the text of the file: PowerShell
    # 7 hands back a Byte[] (so the parse below yielded "48", the first byte of
    # "0" in decimal) and Windows PowerShell 5.1 hands back an empty string.
    # Either way the comparison failed on a release that was in fact intact.
    # `-OutFile` bypasses the content-type handling entirely.
    Write-Step 'Verifying checksum'
    $sumFile = Join-Path $temp "$assetName.sha256"
    Invoke-WebRequest -Uri "$base/$assetName.sha256" -OutFile $sumFile -UseBasicParsing
    $expectedLine = (Get-Content $sumFile -Raw)
    $expected = ($expectedLine -split '\s+')[0].ToLower()
    # Distinguish "the checksum file did not arrive" from "the binary is wrong",
    # so the next such failure names its own cause instead of blaming the asset.
    if ($expected -notmatch '^[0-9a-f]{64}$') {
        throw "Could not read the published checksum for $assetName. Nothing was installed."
    }
    $actual = (Get-FileHash $downloaded -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) {
        throw "Checksum mismatch. Expected $expected, got $actual. Nothing was installed."
    }
    Write-Note "sha256 $actual"

    # A running instance holds a lock on its own file.
    $running = Get-Process tpmplaner -ErrorAction SilentlyContinue
    if ($running) {
        Write-Step 'Stopping the running widget'
        $running | Stop-Process -Force
        Start-Sleep -Milliseconds 800
    }

    Write-Step "Installing to $installDir"
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item $downloaded $exePath -Force

    # Start menu entry, so the widget can be found again after closing it.
    $startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
    $shortcut = Join-Path $startMenu 'TPMPlaner.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($shortcut)
    $link.TargetPath = $exePath
    $link.WorkingDirectory = $installDir
    $link.Description = "Today's calendar events and due tasks on your desktop"
    $link.Save()

    if (-not $NoAutostart) {
        Write-Step 'Enabling autostart'
        # `-Force` on New-ItemProperty overwrites an existing value, but it
        # does not create a missing key, and the Run key is absent on a
        # profile where nothing has ever registered for autostart. Without
        # this the install aborts here, after the binary is already in place.
        #
        # The Test-Path is not redundant, unlike in the uninstall entry below:
        # `New-Item -Force` on a registry key that is already there recreates
        # it and drops its values, which for this key means unregistering
        # every other program's autostart.
        $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
        if (-not (Test-Path $runKey)) { New-Item -Path $runKey -Force | Out-Null }
        New-ItemProperty -Path $runKey `
            -Name 'TPMPlaner' -Value "`"$exePath`"" -PropertyType String -Force | Out-Null
    }

    # Record what was installed so the uninstaller and Windows' own app list
    # know about it.
    $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\TPMPlaner'
    New-Item -Path $uninstallKey -Force | Out-Null
    $props = @{
        DisplayName     = 'TPMPlaner'
        DisplayIcon     = $exePath
        InstallLocation = $installDir
        Publisher       = 'TPMPlaner contributors'
        NoModify        = 1
        NoRepair        = 1
        UninstallString = "powershell -NoProfile -ExecutionPolicy Bypass -File `"$installDir\uninstall.ps1`""
    }
    foreach ($k in $props.Keys) {
        New-ItemProperty -Path $uninstallKey -Name $k -Value $props[$k] -Force | Out-Null
    }

    # Keep the uninstaller next to the binary so it survives without network.
    try {
        Invoke-WebRequest -Uri "$base/uninstall.ps1" `
            -OutFile (Join-Path $installDir 'uninstall.ps1') -UseBasicParsing
    } catch {
        Write-Note 'Uninstaller could not be downloaded; use uninstall.ps1 from the repository.'
    }

    if (-not $NoStart) {
        Write-Step 'Starting TPMPlaner'
        Start-Process $exePath
    }

    Write-Host ''
    Write-Host 'TPMPlaner installed.' -ForegroundColor Green
    Write-Note "Binary:   $exePath"
    Write-Note "Settings: $env:APPDATA\TPMPlaner"
    # Kept to plain ASCII on purpose. This file has no BOM, so Windows
    # PowerShell 5.1 reads it as the system ANSI codepage and `irm | iex`
    # decodes it from an HTTP response that carries no charset -- either way a
    # UTF-8 dash reaches the user as mojibake. The CI check keeps it that way.
    Write-Note 'Next step: connect a calendar - right-click the widget.'
    Write-Note 'Docs: https://josunlp.github.io/TPMPlaner/'
} finally {
    Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
}
