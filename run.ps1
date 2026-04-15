# run.ps1 — Launch the active Tauri shell from the repo root.
#
# Usage:
#   .\run.ps1
#   $env:BUILD_TYPE = "Release"; .\run.ps1
#   $env:BUILD_DIR = "target-root"; .\run.ps1

[CmdletBinding()]
param([Parameter(ValueFromRemainingArguments)] $Args)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Require-Command {
    param([string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Write-Host "ERROR: required command not found: $Name" -ForegroundColor Red
        exit 1
    }
}

function Ensure-NodeModules {
    $lockFile = Join-Path (Get-Location) "package-lock.json"
    $lockMarker = Join-Path (Get-Location) "node_modules\.package-lock.json"
    if (-not (Test-Path "node_modules") -or -not (Test-Path $lockMarker) -or ((Get-Item $lockFile).LastWriteTimeUtc -gt (Get-Item $lockMarker).LastWriteTimeUtc)) {
        Write-Host "==> Installing frontend dependencies"
        & npm ci
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
}

$UiDir = if ($env:PIER_UI_DIR) { $env:PIER_UI_DIR } else { Join-Path $PSScriptRoot "pier-ui-tauri" }
$BuildType = if ($env:BUILD_TYPE) { $env:BUILD_TYPE } else { "Debug" }
$BuildDir = if ($env:BUILD_DIR) { $env:BUILD_DIR } else { $null }

if (-not (Test-Path $UiDir)) {
    Write-Host "ERROR: active Tauri shell not found at $UiDir" -ForegroundColor Red
    exit 1
}

if ($BuildType -notin @("Debug", "Release")) {
    Write-Host "ERROR: BUILD_TYPE must be Debug or Release (got: $BuildType)" -ForegroundColor Red
    exit 1
}

Require-Command node
Require-Command npm
Require-Command cargo

if ($BuildDir) {
    $resolvedBuildDir = if ([System.IO.Path]::IsPathRooted($BuildDir)) {
        $BuildDir
    } else {
        Join-Path $PSScriptRoot $BuildDir
    }
    $env:CARGO_TARGET_DIR = $resolvedBuildDir
    Write-Host "==> Using Cargo target dir: $resolvedBuildDir"
}

Push-Location $UiDir
try {
    Ensure-NodeModules

    $tauriArgs = @("run", "tauri", "--", "dev")
    if ($BuildType -eq "Release") {
        $tauriArgs += "--release"
    }
    if ($Args.Count -gt 0) {
        $tauriArgs += "--"
        $tauriArgs += $Args
    }

    Write-Host "==> Launching Pier-X Tauri shell ($BuildType)"
    & npm @tauriArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
