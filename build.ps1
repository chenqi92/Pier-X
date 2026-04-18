# build.ps1 — Build the active GPUI shell from the repo root.
#
# Usage:
#   .\build.ps1
#   $env:BUILD_TYPE = "Debug"; .\build.ps1
#   $env:BUILD_DIR = "target-root"; .\build.ps1
#   $env:PACKAGE_FORMATS = "portable,installer,msix"; .\build.ps1

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Require-Command {
    param([string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Write-Host "ERROR: required command not found: $Name" -ForegroundColor Red
        exit 1
    }
}

function Get-Crate-Version {
    param([string]$CargoTomlPath)

    $content = Get-Content -LiteralPath $CargoTomlPath -Raw
    $match = [regex]::Match($content, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) {
        throw "Failed to resolve version from $CargoTomlPath"
    }
    return $match.Groups[1].Value
}

function Resolve-AbsolutePath {
    param([string]$Path)

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return Join-Path $PSScriptRoot $Path
}

function Get-TargetRoot {
    if ($env:CARGO_TARGET_DIR) {
        return $env:CARGO_TARGET_DIR
    }
    return (Join-Path $PSScriptRoot "target")
}

function Get-PackageRoot {
    if ($env:PACKAGE_OUTPUT_DIR) {
        return (Resolve-AbsolutePath $env:PACKAGE_OUTPUT_DIR)
    }
    return (Join-Path (Get-TargetRoot) "$ProfileDir\packages")
}

function Get-NormalizedPackageFormats {
    param([string]$Formats)

    if (-not $Formats) {
        return @()
    }

    return @(
        $Formats.ToLowerInvariant().Split(@(",", ";", " "), [System.StringSplitOptions]::RemoveEmptyEntries) |
        Select-Object -Unique
    )
}

function Test-PackageFormat {
    param(
        [string[]]$Formats,
        [string]$Name
    )

    return ($Formats -contains "all") -or ($Formats -contains $Name)
}

function New-CleanDirectory {
    param([string]$Path)

    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
    New-Item -ItemType Directory -Path $Path -Force | Out-Null
}

function Copy-PackagePayload {
    param(
        [string]$DestinationDir,
        [string]$BinaryPath,
        [string]$ExecutableName
    )

    New-CleanDirectory $DestinationDir
    Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $DestinationDir $ExecutableName)
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "README.md") -Destination (Join-Path $DestinationDir "README.md")
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "LICENSE") -Destination (Join-Path $DestinationDir "LICENSE")
}

function Resolve-InnoSetupCompiler {
    $command = Get-Command iscc -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $commonPaths = @()
    if (${env:ProgramFiles(x86)}) {
        $commonPaths += (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe")
    }
    if ($env:ProgramFiles) {
        $commonPaths += (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    }

    foreach ($path in $commonPaths) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    throw "Inno Setup compiler not found. Install Inno Setup 6 or add iscc to PATH."
}

function Resolve-WindowsSdkTool {
    param([string]$ToolName)

    $command = Get-Command $ToolName -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $roots = @()
    if (${env:ProgramFiles(x86)}) {
        $roots += (Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin")
    }
    if ($env:ProgramFiles) {
        $roots += (Join-Path $env:ProgramFiles "Windows Kits\10\bin")
    }
    $roots = @($roots | Where-Object { $_ -and (Test-Path -LiteralPath $_) })

    foreach ($root in $roots) {
        $match = Get-ChildItem -LiteralPath $root -Filter $ToolName -Recurse -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\' -or $_.FullName -match '\\arm64\\' } |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($match) {
            return $match.FullName
        }
    }

    throw "$ToolName not found. Install the Windows SDK and ensure the packaging tools are available."
}

function Get-WindowsArchitecture {
    if ($env:CARGO_BUILD_TARGET) {
        if ($env:CARGO_BUILD_TARGET -match 'aarch64') {
            return [pscustomobject]@{
                Msix          = "arm64"
                Portable      = "arm64"
                InnoAllowed   = "arm64"
                Inno64BitMode = "arm64"
            }
        }
        if ($env:CARGO_BUILD_TARGET -match 'i686') {
            return [pscustomobject]@{
                Msix          = "x86"
                Portable      = "x86"
                InnoAllowed   = ""
                Inno64BitMode = ""
            }
        }
    }

    switch ($env:PROCESSOR_ARCHITECTURE.ToUpperInvariant()) {
        "ARM64" {
            return [pscustomobject]@{
                Msix          = "arm64"
                Portable      = "arm64"
                InnoAllowed   = "arm64"
                Inno64BitMode = "arm64"
            }
        }
        "X86" {
            return [pscustomobject]@{
                Msix          = "x86"
                Portable      = "x86"
                InnoAllowed   = ""
                Inno64BitMode = ""
            }
        }
        default {
            return [pscustomobject]@{
                Msix          = "x64"
                Portable      = "x64"
                InnoAllowed   = "x64compatible"
                Inno64BitMode = "x64compatible"
            }
        }
    }
}

function Get-AppxVersion {
    param([string]$Version)

    $core = ($Version -split '-', 2)[0]
    $parts = @($core.Split('.'))
    while ($parts.Count -lt 4) {
        $parts += "0"
    }
    if ($parts.Count -gt 4) {
        $parts = $parts[0..3]
    }

    return ($parts -join '.')
}

function Ensure-SystemDrawing {
    Add-Type -AssemblyName System.Drawing
}

function Resize-PngAsset {
    param(
        [string]$SourcePath,
        [string]$DestinationPath,
        [int]$Width,
        [int]$Height
    )

    Ensure-SystemDrawing

    $sourceImage = [System.Drawing.Image]::FromFile($SourcePath)
    try {
        $bitmap = New-Object System.Drawing.Bitmap $Width, $Height
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.Clear([System.Drawing.Color]::Transparent)

                $scale = [Math]::Min($Width / $sourceImage.Width, $Height / $sourceImage.Height)
                $drawWidth = [int][Math]::Round($sourceImage.Width * $scale)
                $drawHeight = [int][Math]::Round($sourceImage.Height * $scale)
                $offsetX = [int][Math]::Floor(($Width - $drawWidth) / 2)
                $offsetY = [int][Math]::Floor(($Height - $drawHeight) / 2)
                $graphics.DrawImage($sourceImage, $offsetX, $offsetY, $drawWidth, $drawHeight)
            } finally {
                $graphics.Dispose()
            }

            $bitmap.Save($DestinationPath, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally {
            $bitmap.Dispose()
        }
    } finally {
        $sourceImage.Dispose()
    }
}

function New-MsixAssets {
    param(
        [string]$SourceIconPath,
        [string]$AssetsDir
    )

    New-Item -ItemType Directory -Path $AssetsDir -Force | Out-Null

    $assetMap = @(
        @{ Name = "Square44x44Logo.png"; Width = 44; Height = 44 }
        @{ Name = "Square71x71Logo.png"; Width = 71; Height = 71 }
        @{ Name = "Square150x150Logo.png"; Width = 150; Height = 150 }
        @{ Name = "Wide310x150Logo.png"; Width = 310; Height = 150 }
        @{ Name = "StoreLogo.png"; Width = 50; Height = 50 }
    )

    foreach ($entry in $assetMap) {
        Resize-PngAsset `
            -SourcePath $SourceIconPath `
            -DestinationPath (Join-Path $AssetsDir $entry.Name) `
            -Width $entry.Width `
            -Height $entry.Height
    }
}

function New-PortableZip {
    param(
        [string]$BinaryPath,
        [string]$PackageRoot,
        [string]$Version,
        [string]$PortableArch
    )

    $portableRoot = Join-Path $PackageRoot "windows\portable"
    $stageDir = Join-Path $portableRoot "Pier-X-$Version-windows-$PortableArch"
    $zipPath = Join-Path $portableRoot "Pier-X_$Version_windows_${PortableArch}_portable.zip"

    Write-Host "==> Packaging portable zip -> $zipPath"
    Copy-PackagePayload -DestinationDir $stageDir -BinaryPath $BinaryPath -ExecutableName "Pier-X.exe"

    if (Test-Path -LiteralPath $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }

    Compress-Archive -LiteralPath $stageDir -DestinationPath $zipPath -CompressionLevel Optimal
    Write-Host "[OK] Windows package ready: $zipPath"
}

function New-InnoSetupInstaller {
    param(
        [string]$BinaryPath,
        [string]$PackageRoot,
        [string]$Version,
        [string]$PortableArch,
        [string]$InstallerArchitecture,
        [string]$Installer64BitMode,
        [string]$AppPublisher,
        [string]$SetupIconPath
    )

    $compilerPath = Resolve-InnoSetupCompiler
    $installerRoot = Join-Path $PackageRoot "windows\installer"
    $payloadDir = Join-Path $installerRoot "payload"
    $scriptPath = Join-Path $installerRoot "Pier-X.iss"
    $outputBaseName = "Pier-X_${Version}_windows_${PortableArch}_setup"

    Write-Host "==> Packaging Inno Setup installer -> $outputBaseName.exe"
    Copy-PackagePayload -DestinationDir $payloadDir -BinaryPath $BinaryPath -ExecutableName "Pier-X.exe"

    $architectureLines = @()
    if ($InstallerArchitecture) {
        $architectureLines += "ArchitecturesAllowed=$InstallerArchitecture"
    }
    if ($Installer64BitMode) {
        $architectureLines += "ArchitecturesInstallIn64BitMode=$Installer64BitMode"
    }
    $architectureBlock = ($architectureLines -join [Environment]::NewLine)

    $iss = @"
[Setup]
AppId={{com.pier-x.desktop}}
AppName=Pier-X
AppVersion=$Version
AppPublisher=$AppPublisher
DefaultDirName={autopf}\Pier-X
DefaultGroupName=Pier-X
DisableProgramGroupPage=yes
$architectureBlock
OutputDir=$installerRoot
OutputBaseFilename=$outputBaseName
Compression=lzma
SolidCompression=yes
WizardStyle=modern
SetupIconFile=$SetupIconPath
UninstallDisplayIcon={app}\Pier-X.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"; GroupDescription: "Additional icons:"

[Files]
Source: "$payloadDir\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Pier-X"; Filename: "{app}\Pier-X.exe"
Name: "{autodesktop}\Pier-X"; Filename: "{app}\Pier-X.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Pier-X.exe"; Description: "Launch Pier-X"; Flags: nowait postinstall skipifsilent
"@

    New-Item -ItemType Directory -Path $installerRoot -Force | Out-Null
    Set-Content -LiteralPath $scriptPath -Value $iss -Encoding ASCII
    & $compilerPath $scriptPath | Out-Null

    $installerPath = Join-Path $installerRoot "$outputBaseName.exe"
    Write-Host "[OK] Windows package ready: $installerPath"
}

function New-MsixPackage {
    param(
        [string]$BinaryPath,
        [string]$PackageRoot,
        [string]$Version,
        [string]$AppxVersion,
        [string]$Architecture,
        [string]$IdentityName,
        [string]$Publisher,
        [string]$PublisherDisplayName,
        [string]$Description,
        [string]$SourceIconPath
    )

    $makeappxPath = Resolve-WindowsSdkTool -ToolName "makeappx.exe"
    $msixRoot = Join-Path $PackageRoot "windows\msix"
    $layoutRoot = Join-Path $msixRoot "layout"
    $appDir = Join-Path $layoutRoot "Pier-X"
    $assetsDir = Join-Path $layoutRoot "Assets"
    $manifestPath = Join-Path $layoutRoot "AppxManifest.xml"
    $msixPath = Join-Path $msixRoot "Pier-X_$Version_windows_${Architecture}.msix"

    Write-Host "==> Packaging MSIX -> $msixPath"
    New-CleanDirectory $layoutRoot
    Copy-PackagePayload -DestinationDir $appDir -BinaryPath $BinaryPath -ExecutableName "Pier-X.exe"
    New-MsixAssets -SourceIconPath $SourceIconPath -AssetsDir $assetsDir

    $manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:desktop="http://schemas.microsoft.com/appx/manifest/desktop/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap desktop rescap">
  <Identity Name="$IdentityName" Publisher="$Publisher" Version="$AppxVersion" ProcessorArchitecture="$Architecture" />
  <Properties>
    <DisplayName>Pier-X</DisplayName>
    <PublisherDisplayName>$PublisherDisplayName</PublisherDisplayName>
    <Description>$Description</Description>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.22621.0" />
  </Dependencies>
  <Applications>
    <Application Id="PierX" Executable="Pier-X\Pier-X.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="Pier-X"
        Description="$Description"
        BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png"
        Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile
          Square71x71Logo="Assets\Square71x71Logo.png"
          Wide310x150Logo="Assets\Wide310x150Logo.png" />
      </uap:VisualElements>
      <Extensions>
        <desktop:Extension Category="windows.fullTrustProcess" Executable="Pier-X\Pier-X.exe" />
      </Extensions>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"@

    New-Item -ItemType Directory -Path $msixRoot -Force | Out-Null
    Set-Content -LiteralPath $manifestPath -Value $manifest -Encoding UTF8

    if (Test-Path -LiteralPath $msixPath) {
        Remove-Item -LiteralPath $msixPath -Force
    }

    & $makeappxPath pack /d $layoutRoot /p $msixPath /o | Out-Null

    if ($env:WINDOWS_MSIX_CERT_PATH) {
        $signtoolPath = Resolve-WindowsSdkTool -ToolName "signtool.exe"
        $timestampUrl = if ($env:WINDOWS_MSIX_TIMESTAMP_URL) { $env:WINDOWS_MSIX_TIMESTAMP_URL } else { "http://timestamp.digicert.com" }

        $signArgs = @(
            "sign",
            "/fd", "SHA256",
            "/f", (Resolve-AbsolutePath $env:WINDOWS_MSIX_CERT_PATH),
            "/tr", $timestampUrl,
            "/td", "SHA256"
        )

        if ($env:WINDOWS_MSIX_CERT_PASSWORD) {
            $signArgs += @("/p", $env:WINDOWS_MSIX_CERT_PASSWORD)
        }

        $signArgs += $msixPath
        & $signtoolPath @signArgs | Out-Null
        Write-Host "[OK] Signed MSIX package: $msixPath"
    } else {
        Write-Host "WARN: WINDOWS_MSIX_CERT_PATH not set; generated MSIX is unsigned." -ForegroundColor Yellow
        Write-Host "      Set WINDOWS_MSIX_PUBLISHER and WINDOWS_MSIX_CERT_PATH to produce an installable package." -ForegroundColor Yellow
    }

    Write-Host "[OK] Windows package ready: $msixPath"
}

$UiCrate = if ($env:PIER_UI_CRATE) { $env:PIER_UI_CRATE } else { "pier-ui-gpui" }
$BuildType = if ($env:BUILD_TYPE) { $env:BUILD_TYPE } else { "Release" }
$BuildDir = if ($env:BUILD_DIR) { $env:BUILD_DIR } else { $null }
$PackageFormats = Get-NormalizedPackageFormats $env:PACKAGE_FORMATS

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
    $resolvedBuildDir = Resolve-AbsolutePath $BuildDir
    $env:CARGO_TARGET_DIR = $resolvedBuildDir
    Write-Host "==> Using Cargo target dir: $resolvedBuildDir"
}

$cargoArgs = @("build", "-p", $UiCrate)
if ($BuildType -eq "Release") {
    $cargoArgs += "--release"
}

Write-Host "==> Building Pier-X GPUI shell ($BuildType)"
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($PackageFormats.Count -eq 0) {
    Write-Host "[OK] Build complete: $(Get-TargetRoot)"
    return
}

$ProfileDir = if ($BuildType -eq "Release") { "release" } else { "debug" }
$TargetRoot = Get-TargetRoot
$BinaryPath = Join-Path $TargetRoot "$ProfileDir\$UiCrate.exe"
$PackageRoot = Get-PackageRoot
$Version = Get-Crate-Version (Join-Path $PSScriptRoot "$UiCrate\Cargo.toml")
$Arch = Get-WindowsArchitecture
$MsixIdentityName = if ($env:WINDOWS_MSIX_IDENTITY_NAME) { $env:WINDOWS_MSIX_IDENTITY_NAME } else { "PierX.Desktop" }
$MsixPublisher = if ($env:WINDOWS_MSIX_PUBLISHER) { $env:WINDOWS_MSIX_PUBLISHER } else { "CN=Pier-X" }
$MsixPublisherDisplayName = if ($env:WINDOWS_MSIX_PUBLISHER_DISPLAY_NAME) { $env:WINDOWS_MSIX_PUBLISHER_DISPLAY_NAME } else { "Pier-X" }
$InstallerPublisher = if ($env:WINDOWS_INSTALLER_PUBLISHER) { $env:WINDOWS_INSTALLER_PUBLISHER } else { "Pier-X" }
$Description = "Cross-platform terminal management on GPUI + Rust core"
$IconIcoPath = Join-Path $PSScriptRoot "pier-ui-gpui\assets\app-icons\icon.ico"
$IconPngPath = Join-Path $PSScriptRoot "pier-ui-gpui\assets\app-icons\icon.png"
$PackagedAny = $false

if (-not (Test-Path -LiteralPath $BinaryPath)) {
    Write-Host "ERROR: binary not found at $BinaryPath" -ForegroundColor Red
    exit 1
}

if ((Test-PackageFormat $PackageFormats "portable") -or (Test-PackageFormat $PackageFormats "zip")) {
    $PackagedAny = $true
    New-PortableZip -BinaryPath $BinaryPath -PackageRoot $PackageRoot -Version $Version -PortableArch $Arch.Portable
}

if (Test-PackageFormat $PackageFormats "installer") {
    if (-not (Test-Path -LiteralPath $IconIcoPath)) {
        Write-Host "ERROR: installer icon not found at $IconIcoPath" -ForegroundColor Red
        exit 1
    }
    $PackagedAny = $true
    New-InnoSetupInstaller `
        -BinaryPath $BinaryPath `
        -PackageRoot $PackageRoot `
        -Version $Version `
        -PortableArch $Arch.Portable `
        -InstallerArchitecture $Arch.InnoAllowed `
        -Installer64BitMode $Arch.Inno64BitMode `
        -AppPublisher $InstallerPublisher `
        -SetupIconPath $IconIcoPath
}

if (Test-PackageFormat $PackageFormats "msix") {
    if (-not (Test-Path -LiteralPath $IconPngPath)) {
        Write-Host "ERROR: MSIX icon source not found at $IconPngPath" -ForegroundColor Red
        exit 1
    }
    $PackagedAny = $true
    New-MsixPackage `
        -BinaryPath $BinaryPath `
        -PackageRoot $PackageRoot `
        -Version $Version `
        -AppxVersion (Get-AppxVersion $Version) `
        -Architecture $Arch.Msix `
        -IdentityName $MsixIdentityName `
        -Publisher $MsixPublisher `
        -PublisherDisplayName $MsixPublisherDisplayName `
        -Description $Description `
        -SourceIconPath $IconPngPath
}

if (-not $PackagedAny) {
    Write-Host "ERROR: unsupported PACKAGE_FORMATS=$($env:PACKAGE_FORMATS) (supported: portable, zip, installer, msix, all)" -ForegroundColor Red
    exit 1
}
