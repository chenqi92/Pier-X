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

function Resolve-NpmCommand {
    $npmCmd = Get-Command "npm.cmd" -ErrorAction SilentlyContinue
    if ($npmCmd) {
        return $npmCmd.Source
    }

    $npm = Get-Command "npm" -ErrorAction SilentlyContinue
    if ($npm) {
        return $npm.Source
    }

    Write-Host "ERROR: required command not found: npm" -ForegroundColor Red
    exit 1
}

function Ensure-NodeModules {
    $lockFile = Join-Path (Get-Location) "package-lock.json"
    $lockMarker = Join-Path (Get-Location) "node_modules\.package-lock.json"
    if (-not (Test-Path "node_modules") -or -not (Test-Path $lockMarker) -or ((Get-Item $lockFile).LastWriteTimeUtc -gt (Get-Item $lockMarker).LastWriteTimeUtc)) {
        Write-Host "==> Installing frontend dependencies"
        & $NpmCommand ci
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
Require-Command cargo
$NpmCommand = Resolve-NpmCommand

if ($BuildDir) {
    $resolvedBuildDir = if ([System.IO.Path]::IsPathRooted($BuildDir)) {
        $BuildDir
    } else {
        Join-Path $PSScriptRoot $BuildDir
    }
    $env:CARGO_TARGET_DIR = $resolvedBuildDir
    Write-Host "==> Using Cargo target dir: $resolvedBuildDir"
}

$tempConfigPath = $null
Push-Location $UiDir
try {
    Ensure-NodeModules

    $portResolver = Join-Path $UiDir "scripts\resolve-dev-port.mjs"
    $portInfo = @{}
    $portLines = & node $portResolver
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    foreach ($line in $portLines) {
        if (-not $line) { continue }
        $key, $value = $line -split "=", 2
        if ($key -and $value) {
            $portInfo[$key] = $value
            [System.Environment]::SetEnvironmentVariable($key, $value)
        }
    }

    if (-not $portInfo["PIER_DEV_URL"] -or -not $portInfo["PIER_DEV_PORT"]) {
        Write-Host "ERROR: failed to resolve a Tauri dev server port" -ForegroundColor Red
        exit 1
    }

    $tempConfigSeed = [System.IO.Path]::GetTempFileName()
    $tempConfigPath = [System.IO.Path]::ChangeExtension($tempConfigSeed, ".json")
    Move-Item -LiteralPath $tempConfigSeed -Destination $tempConfigPath -Force
    @{ build = @{ devUrl = $portInfo["PIER_DEV_URL"] } } |
        ConvertTo-Json -Depth 4 |
        Set-Content -LiteralPath $tempConfigPath -NoNewline

    $tauriArgs = @("run", "tauri", "--", "dev", "--config", $tempConfigPath)
    if ($BuildType -eq "Release") {
        $tauriArgs += "--release"
    }
    if ($Args.Count -gt 0) {
        $tauriArgs += "--"
        $tauriArgs += $Args
    }

    Write-Host "==> Launching Pier-X Tauri shell ($BuildType)"
    Write-Host "==> Using Vite dev server: $($portInfo["PIER_DEV_URL"])"
    & $NpmCommand @tauriArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    if ($tempConfigPath -and (Test-Path $tempConfigPath)) {
        Remove-Item -LiteralPath $tempConfigPath -Force
    }
    Pop-Location
}
