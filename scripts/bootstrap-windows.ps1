<#
.SYNOPSIS
    Installs the Flutter SDK version pinned by this repo, on Windows, without hermit.

.DESCRIPTION
    The repo vendors its toolchains with hermit (bin/flutter -> bin/.flutter-<version>.pkg).
    hermit publishes no Windows release: bootstrapping it fetches
    hermit-mingw64_nt-10.0-<build>-amd64.gz and gets HTTP 404, after which
    `source bin/activate-hermit` leaves you with `flutter: command not found`.

    This script closes that gap. It reads the pinned Flutter version straight out of
    bin/.flutter-<version>.pkg, resolves the matching Windows archive from Google's
    release manifest, downloads and SHA256-verifies it, extracts it, and prints the
    PATH / PUB_CACHE lines to export. It is idempotent: an already-downloaded archive
    is verified but not refetched, an already-extracted SDK of the pinned version is
    left alone, and nothing is ever deleted.

.PARAMETER InstallRoot
    Directory holding the SDK and its pub cache. The SDK lands in <InstallRoot>\flutter
    and PUB_CACHE points at <InstallRoot>\pub-cache. Defaults to E:\flutter-sdk.

.PARAMETER PrintEnvOnly
    Skip install entirely and just print the environment lines for an existing install.

.EXAMPLE
    pwsh -File scripts/bootstrap-windows.ps1

.EXAMPLE
    pwsh -File scripts/bootstrap-windows.ps1 -InstallRoot C:\sdks\flutter-sdk

.EXAMPLE
    pwsh -File scripts/bootstrap-windows.ps1 -PrintEnvOnly
#>

[CmdletBinding()]
param(
    [string]$InstallRoot = 'E:\flutter-sdk',
    [switch]$PrintEnvOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ManifestUrl = 'https://storage.googleapis.com/flutter_infra_release/releases/releases_windows.json'

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message"
}

function Fail {
    param([string]$Message)
    [Console]::Error.WriteLine('')
    [Console]::Error.WriteLine("ERROR: $Message")
    exit 1
}

# --- 1. Pinned version, read from the repo -------------------------------------------

function Get-PinnedFlutterVersion {
    param([string]$BinDir)

    if (-not (Test-Path -LiteralPath $BinDir -PathType Container)) {
        Fail "No bin/ directory at '$BinDir'. Run this script from inside the buzz checkout."
    }

    $pkgs = @(Get-ChildItem -Force -LiteralPath $BinDir -Filter '.flutter-*.pkg' | Sort-Object Name)

    if ($pkgs.Count -eq 0) {
        Fail "No bin/.flutter-<version>.pkg found in '$BinDir'. Cannot determine the pinned Flutter version."
    }
    if ($pkgs.Count -gt 1) {
        $names = ($pkgs | ForEach-Object { $_.Name }) -join ', '
        Fail "Ambiguous Flutter pin: found $($pkgs.Count) marker files in '$BinDir' ($names). Exactly one is expected."
    }

    $name = $pkgs[0].Name
    $parsed = [regex]::Match($name, '^\.flutter-(?<version>\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.\-]+)?)\.pkg$')
    if (-not $parsed.Success) {
        Fail "Could not parse a version out of '$name' (expected .flutter-<version>.pkg)."
    }

    return $parsed.Groups['version'].Value
}

# --- 2. Windows archive, resolved from the release manifest ---------------------------

function Get-WindowsRelease {
    param([string]$Version)

    Write-Step "Resolving the Windows archive for Flutter $Version from the release manifest"

    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64' -or $env:PROCESSOR_ARCHITEW6432 -eq 'ARM64') {
        'arm64'
    }
    else {
        'x64'
    }

    try {
        $manifest = Invoke-RestMethod -Uri $ManifestUrl -UseBasicParsing
    }
    catch {
        Fail "Failed to fetch the Flutter Windows release manifest from $ManifestUrl : $($_.Exception.Message)"
    }

    $candidates = @($manifest.releases | Where-Object { $_.version -eq $Version })
    if ($candidates.Count -eq 0) {
        Fail @"
Flutter $Version has no Windows build in $ManifestUrl.
The repo pins that version via bin/.flutter-$Version.pkg, but Google publishes no matching
Windows archive for it. Raise the pin, or install that version from source by hand.
"@
    }

    # dart_sdk_arch only appears once multi-arch builds existed; absent means x64.
    $pickArch = {
        param([string]$Want)
        @($candidates | Where-Object {
                $relArch = if ($_.PSObject.Properties.Name -contains 'dart_sdk_arch') { $_.dart_sdk_arch } else { 'x64' }
                $relArch -eq $Want
            })
    }

    $forArch = @(& $pickArch $arch)
    if ($forArch.Count -eq 0 -and $arch -ne 'x64') {
        # Flutter publishes no arm64 Windows SDK at all - releases_windows.json has
        # zero arm64 entries. Windows on ARM runs the x64 build under emulation,
        # which is what flutter.dev tells you to install there.
        $forArch = @(& $pickArch 'x64')
        if ($forArch.Count -gt 0) {
            Write-Host "    note: no arm64 Windows archive exists for Flutter $Version; using the x64 one (Windows on ARM runs it under emulation)."
            $arch = 'x64'
        }
    }
    if ($forArch.Count -eq 0) {
        $channels = ($candidates | ForEach-Object { $_.channel }) -join ', '
        Fail "Flutter $Version has a Windows build, but none usable on $arch (channels found: $channels)."
    }

    $release = $forArch | Where-Object { $_.channel -eq 'stable' } | Select-Object -First 1
    if (-not $release) { $release = $forArch | Select-Object -First 1 }

    $distinct = @($forArch | Select-Object -ExpandProperty archive -Unique)
    if ($distinct.Count -gt 1) {
        Write-Host "    note: $($distinct.Count) $arch archives claim version $Version; using the $($release.channel) one."
    }

    foreach ($field in @('archive', 'sha256')) {
        if (-not ($release.PSObject.Properties.Name -contains $field) -or [string]::IsNullOrWhiteSpace($release.$field)) {
            Fail "The manifest entry for Flutter $Version is missing '$field'."
        }
    }

    return [pscustomobject]@{
        Version = $Version
        Channel = $release.channel
        Arch    = $arch
        Url     = "$($manifest.base_url)/$($release.archive)"
        Sha256  = $release.sha256.ToLowerInvariant()
        File    = Split-Path -Leaf $release.archive.Replace('/', '\')
    }
}

# --- 3. Download + verify -------------------------------------------------------------

function Get-VerifiedArchive {
    param($Release, [string]$Root)

    $archivePath = Join-Path $Root $Release.File

    if (Test-Path -LiteralPath $archivePath -PathType Leaf) {
        Write-Step "Archive already present, skipping download: $archivePath"
    }
    else {
        Write-Step "Downloading $($Release.Url)"
        $partial = "$archivePath.partial"
        $previousProgress = $ProgressPreference
        $ProgressPreference = 'SilentlyContinue'
        try {
            Invoke-WebRequest -Uri $Release.Url -OutFile $partial -UseBasicParsing
        }
        catch {
            Fail "Download failed: $($_.Exception.Message)"
        }
        finally {
            $ProgressPreference = $previousProgress
        }
        Move-Item -LiteralPath $partial -Destination $archivePath -Force
    }

    Write-Step 'Verifying SHA256 against the manifest (reads the whole archive)'
    $actual = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Release.Sha256) {
        Fail @"
SHA256 mismatch for $archivePath
  expected $($Release.Sha256)
  actual   $actual
Refusing to extract. Move or delete that file yourself, then re-run this script.
"@
    }
    Write-Host "    ok: $actual"

    return $archivePath
}

# --- 4. Extract -----------------------------------------------------------------------

function Get-InstalledVersion {
    param([string]$FlutterExe)

    if (-not (Test-Path -LiteralPath $FlutterExe -PathType Leaf)) { return $null }

    $output = (& $FlutterExe --version 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) {
        Fail "'$FlutterExe --version' exited with $LASTEXITCODE.`n$output"
    }

    $parsed = [regex]::Match($output, 'Flutter\s+(?<version>\d+\.\d+\.\d+[0-9A-Za-z.\-+]*)')
    if (-not $parsed.Success) {
        Fail "Could not parse a version from '$FlutterExe --version' output:`n$output"
    }

    return $parsed.Groups['version'].Value
}

function Install-Sdk {
    param($Release, [string]$Root, [string]$SdkDir)

    $flutterExe = Join-Path $SdkDir 'bin\flutter.bat'

    if (Test-Path -LiteralPath $SdkDir -PathType Container) {
        $installed = Get-InstalledVersion -FlutterExe $flutterExe
        if ($null -eq $installed) {
            Fail @"
'$SdkDir' exists but has no bin\flutter.bat, so it is not a usable Flutter SDK.
Move it aside (this script never deletes anything) or pass a different -InstallRoot, then re-run.
"@
        }
        if ($installed -ne $Release.Version) {
            Fail @"
'$SdkDir' holds Flutter $installed but this repo pins $($Release.Version).
Move it aside (this script never deletes anything) or pass a different -InstallRoot, then re-run.
"@
        }
        Write-Step "Flutter $installed already extracted at $SdkDir, skipping download and extraction"
        return
    }

    $archivePath = Get-VerifiedArchive -Release $Release -Root $Root

    Write-Step "Extracting to $SdkDir (a few minutes; the archive is ~1.8 GB)"
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    try {
        # The archive contains a single top-level flutter/ directory, so extract into the root.
        [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $Root)
    }
    catch {
        Fail "Extraction failed: $($_.Exception.Message)"
    }

    if (-not (Test-Path -LiteralPath $flutterExe -PathType Leaf)) {
        Fail "Extraction finished but '$flutterExe' is missing. The archive layout is not what was expected."
    }
}

# --- 5. Environment -------------------------------------------------------------------

function Show-Env {
    param([string]$SdkDir, [string]$PubCache)

    $binDir = Join-Path $SdkDir 'bin'
    # E:\flutter-sdk\flutter\bin -> /e/flutter-sdk/flutter/bin for Git Bash / MSYS.
    $bashBin = '/' + ($binDir -replace '^(?<drive>[A-Za-z]):\\', '${drive}/' -replace '\\', '/')
    $bashBin = $bashBin.Substring(0, 2).ToLowerInvariant() + $bashBin.Substring(2)

    Write-Host ''
    Write-Host 'This script does not change PATH permanently. Set these per shell:'
    Write-Host ''
    Write-Host '  # PowerShell'
    Write-Host "  `$env:PATH = `"$binDir;`$env:PATH`""
    Write-Host "  `$env:PUB_CACHE = `"$PubCache`""
    Write-Host ''
    Write-Host '  # Git Bash'
    Write-Host "  export PATH=`"${bashBin}:`$PATH`""
    Write-Host "  export PUB_CACHE='$PubCache'"
    Write-Host ''
    Write-Host '  # Or call the binaries directly, no PATH change needed:'
    Write-Host "  $(Join-Path $binDir 'flutter.bat') test"
    Write-Host "  $(Join-Path $binDir 'dart.bat') format ."
    Write-Host ''
    Write-Host '  # Optional, persist PUB_CACHE for your user (affects new shells only):'
    Write-Host "  [Environment]::SetEnvironmentVariable('PUB_CACHE', '$PubCache', 'User')"
    Write-Host ''
    Write-Host 'Do not use bin/flutter or bin/activate-hermit on Windows - hermit ships no Windows'
    Write-Host 'build and its bootstrap 404s. See docs/windows-dev-setup.md.'
}

# --- main -----------------------------------------------------------------------------

$repoRoot = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $repoRoot 'bin'

$pinned = Get-PinnedFlutterVersion -BinDir $binDir
Write-Step "Repo pins Flutter $pinned (from bin\.flutter-$pinned.pkg)"

# [IO.Path]::Combine, not Join-Path: Join-Path resolves the drive qualifier and
# throws a raw "Cannot find drive" exception when -InstallRoot names a drive this
# machine does not have (the default root is E:\, which is not universal).
$sdkDir = [IO.Path]::Combine($InstallRoot, 'flutter')
$pubCache = [IO.Path]::Combine($InstallRoot, 'pub-cache')
$flutterExe = [IO.Path]::Combine($sdkDir, 'bin\flutter.bat')

if ($PrintEnvOnly) {
    if (-not (Test-Path -LiteralPath $flutterExe -PathType Leaf)) {
        Fail "-PrintEnvOnly was passed but '$flutterExe' does not exist. Run this script without -PrintEnvOnly first."
    }
    Show-Env -SdkDir $sdkDir -PubCache $pubCache
    exit 0
}

$release = Get-WindowsRelease -Version $pinned
Write-Host "    $($release.Channel)/$($release.Arch): $($release.Url)"

try {
    if (-not (Test-Path -LiteralPath $InstallRoot -PathType Container)) {
        New-Item -ItemType Directory -Path $InstallRoot -ErrorAction Stop | Out-Null
    }
    if (-not (Test-Path -LiteralPath $pubCache -PathType Container)) {
        New-Item -ItemType Directory -Path $pubCache -ErrorAction Stop | Out-Null
    }
}
catch {
    Fail @"
Could not create the install root '$InstallRoot': $($_.Exception.Message)
The default root is E:\flutter-sdk, which does not exist on every machine.
Pass -InstallRoot <dir> to pick a location that exists here.
"@
}

Install-Sdk -Release $release -Root $InstallRoot -SdkDir $sdkDir

Write-Step 'Verifying the install'
$installed = Get-InstalledVersion -FlutterExe $flutterExe
if ($installed -ne $pinned) {
    Fail "The installed Flutter reports $installed but the repo pins $pinned."
}
$dartExe = Join-Path $sdkDir 'bin\dart.bat'
if (-not (Test-Path -LiteralPath $dartExe -PathType Leaf)) {
    Fail "Flutter $installed is installed but '$dartExe' is missing."
}
Write-Host "    ok: Flutter $installed at $sdkDir"

Show-Env -SdkDir $sdkDir -PubCache $pubCache
exit 0
