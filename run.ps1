# run.ps1 — Configure, build, and launch Pier-X on Windows.
#
# Usage:
#   .\run.ps1
#   $env:BUILD_TYPE = "Debug"; .\run.ps1
#   $env:QT_DIR = "C:\Qt\6.8.1\msvc2022_64"; .\run.ps1
#   $env:BUILD_DIR = "build-debug"; .\run.ps1

[CmdletBinding()]
param([Parameter(ValueFromRemainingArguments)] $Args)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$BuildType = if ($env:BUILD_TYPE) { $env:BUILD_TYPE } else { "Release" }
$BuildDir  = if ($env:BUILD_DIR)  { $env:BUILD_DIR }  else { "build" }

$cmakeArgs = @("-B", $BuildDir, "-S", ".", "-DCMAKE_BUILD_TYPE=$BuildType")
if ($env:QT_DIR) {
    $cmakeArgs += "-DCMAKE_PREFIX_PATH=$env:QT_DIR"
}

Write-Host "→ Configuring Pier-X ($BuildType) in $BuildDir"
& cmake @cmakeArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "→ Building"
& cmake --build $BuildDir --config $BuildType --parallel
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Locate the binary
$exe = Join-Path $BuildDir "pier-ui-qt\$BuildType\pier-x.exe"
if (-not (Test-Path $exe)) {
    # Single-config generator fallback
    $exe = Join-Path $BuildDir "pier-ui-qt\pier-x.exe"
}

if (-not (Test-Path $exe)) {
    Write-Error "Binary not found at $exe"
    exit 1
}

Write-Host "→ Launching $exe"
& $exe @Args
