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

function Find-Qt6 {
    if ($env:QT_DIR -and (Test-Path (Join-Path $env:QT_DIR "lib\cmake\Qt6\Qt6Config.cmake"))) {
        return $env:QT_DIR
    }
    foreach ($name in @("qmake6", "qmake")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) {
            try {
                $prefix = (& $cmd -query QT_INSTALL_PREFIX 2>$null) -join ""
                if ($prefix -and (Test-Path (Join-Path $prefix "lib\cmake\Qt6\Qt6Config.cmake"))) {
                    return $prefix
                }
            } catch {}
        }
    }
    $found = @()
    $roots = @(
        "C:\Qt", "C:\Qt\Tools\Qt",
        "D:\Qt", "D:\Qt\Tools\Qt",
        "E:\Qt", "E:\Qt\Tools\Qt",
        (Join-Path $env:USERPROFILE "Qt")
    )
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) { continue }
        $versionDirs = Get-ChildItem $root -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^6\.\d+(\.\d+)?$' } |
            Sort-Object { [version]$_.Name } -Descending
        foreach ($vd in $versionDirs) {
            foreach ($arch in @("msvc2022_64", "msvc2019_64", "mingw_64")) {
                $cand = Join-Path $vd.FullName $arch
                if (Test-Path (Join-Path $cand "lib\cmake\Qt6\Qt6Config.cmake")) {
                    $found += $cand
                }
            }
        }
    }
    if ($found.Count -gt 0) { return $found[0] }
    return $null
}

$BuildType = if ($env:BUILD_TYPE) { $env:BUILD_TYPE } else { "Release" }
$BuildDir  = if ($env:BUILD_DIR)  { $env:BUILD_DIR }  else { "build" }

$qtDir = Find-Qt6
if (-not $qtDir) {
    Write-Host ""
    Write-Host "ERROR: Qt 6.8 not found." -ForegroundColor Red
    Write-Host ""
    Write-Host "Install Qt 6.8 LTS first. Easiest path:"
    Write-Host "    pip install aqtinstall"
    Write-Host "    aqt install-qt windows desktop 6.8.1 win64_msvc2022_64 --outputdir C:\Qt"
    Write-Host "    `$env:QT_DIR = `"C:\Qt\6.8.1\msvc2022_64`""
    Write-Host "    .\build.ps1"
    Write-Host ""
    Write-Host "Or set `$env:QT_DIR explicitly if Qt is installed elsewhere."
    Write-Host ""
    exit 1
}

Write-Host "==> Found Qt at: $qtDir"

$cmakeArgs = @("-B", $BuildDir, "-S", ".", "-DCMAKE_BUILD_TYPE=$BuildType", "-DCMAKE_PREFIX_PATH=$qtDir")

Write-Host "==> Configuring Pier-X ($BuildType) in $BuildDir"
& cmake @cmakeArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "==> Building"
& cmake --build $BuildDir --config $BuildType --parallel
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "[OK] Build complete: $BuildDir"
