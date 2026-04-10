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

# Use ASCII output throughout — avoids garbled glyphs on consoles still
# defaulting to a non-UTF-8 code page (e.g. cp936 on Chinese Windows).

function Find-Qt6 {
    # 1. Explicit QT_DIR wins
    if ($env:QT_DIR -and (Test-Path (Join-Path $env:QT_DIR "lib\cmake\Qt6\Qt6Config.cmake"))) {
        return $env:QT_DIR
    }

    # 2. qmake / qmake6 already in PATH
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

    # 3. Scan common install roots — pick the highest 6.x version
    $found = @()
    $roots = @("C:\Qt", "C:\Qt\Tools\Qt", (Join-Path $env:USERPROFILE "Qt"))
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
    if ($found.Count -gt 0) {
        return $found[0]
    }

    return $null
}

$BuildType = if ($env:BUILD_TYPE) { $env:BUILD_TYPE } else { "Release" }
$BuildDir  = if ($env:BUILD_DIR)  { $env:BUILD_DIR }  else { "build" }

$qtDir = Find-Qt6
if (-not $qtDir) {
    Write-Host ""
    Write-Host "ERROR: Qt 6.8 not found." -ForegroundColor Red
    Write-Host ""
    Write-Host "Pier-X needs Qt 6.8 LTS (or newer) installed. Pick one:"
    Write-Host ""
    Write-Host "  Option A - aqtinstall (recommended, matches CI):"
    Write-Host "    pip install aqtinstall"
    Write-Host "    aqt install-qt windows desktop 6.8.1 win64_msvc2022_64 --outputdir C:\Qt"
    Write-Host "    `$env:QT_DIR = `"C:\Qt\6.8.1\msvc2022_64`""
    Write-Host "    .\run.ps1"
    Write-Host ""
    Write-Host "  Option B - Official Qt Online Installer:"
    Write-Host "    https://www.qt.io/download-qt-installer"
    Write-Host "    Install Qt 6.8.x -> MSVC 2022 64-bit"
    Write-Host "    `$env:QT_DIR = `"C:\Qt\6.8.1\msvc2022_64`""
    Write-Host "    .\run.ps1"
    Write-Host ""
    Write-Host "  Option C - if Qt is already installed somewhere unusual:"
    Write-Host "    `$env:QT_DIR = `"<path containing lib\cmake\Qt6>`""
    Write-Host "    .\run.ps1"
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

# Locate the binary
$exe = Join-Path $BuildDir "pier-ui-qt\$BuildType\pier-x.exe"
if (-not (Test-Path $exe)) {
    # Single-config generator fallback
    $exe = Join-Path $BuildDir "pier-ui-qt\pier-x.exe"
}

if (-not (Test-Path $exe)) {
    Write-Host "ERROR: Binary not found at $exe" -ForegroundColor Red
    exit 1
}

Write-Host "==> Launching $exe"
& $exe @Args
