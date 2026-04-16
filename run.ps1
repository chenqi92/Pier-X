# run.ps1 — Launch the active GPUI shell from the repo root.
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

$UiCrate = if ($env:PIER_UI_CRATE) { $env:PIER_UI_CRATE } else { "pier-ui-gpui" }
$BuildType = if ($env:BUILD_TYPE) { $env:BUILD_TYPE } else { "Debug" }
$BuildDir = if ($env:BUILD_DIR) { $env:BUILD_DIR } else { $null }

if (-not (Test-Path (Join-Path $PSScriptRoot $UiCrate))) {
    Write-Host "ERROR: active GPUI shell crate not found at $(Join-Path $PSScriptRoot $UiCrate)" -ForegroundColor Red
    exit 1
}

if ($BuildType -notin @("Debug", "Release")) {
    Write-Host "ERROR: BUILD_TYPE must be Debug or Release (got: $BuildType)" -ForegroundColor Red
    exit 1
}

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

$cargoArgs = @("run", "-p", $UiCrate)
if ($BuildType -eq "Release") {
    $cargoArgs += "--release"
}
if ($Args.Count -gt 0) {
    $cargoArgs += "--"
    $cargoArgs += $Args
}

Write-Host "==> Launching Pier-X GPUI shell ($BuildType)"
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
