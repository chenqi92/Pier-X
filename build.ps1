# build.ps1 — Configure and build Pier-X without launching it.
#
# Usage:
#   .\build.ps1
#   $env:BUILD_TYPE = "Debug"; .\build.ps1
#   $env:QT_DIR = "C:\Qt\6.8.1\msvc2022_64"; .\build.ps1
#   $env:BUILD_DIR = "build-debug"; .\build.ps1

[CmdletBinding()]
param()

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

Write-Host "✓ Build complete: $BuildDir"
