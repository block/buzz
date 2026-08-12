[CmdletBinding()]
param(
    [switch]$SkipDependencyInstall,
    [switch]$SkipSidecarBuild,
    [switch]$SkipChecks,
    [string]$OutputDirectory,
    [string]$BuildCacheDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Target = "x86_64-pc-windows-msvc"
$ScriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $ScriptDirectory "..")).Path
$DesktopDirectory = Join-Path $RepositoryRoot "desktop"
$TauriDirectory = Join-Path $DesktopDirectory "src-tauri"
$ConfigPath = Join-Path $TauriDirectory "tauri.codex-lab.conf.json"
$BinariesDirectory = Join-Path $TauriDirectory "binaries"

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $RepositoryRoot "dist\codex-lab-windows"
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)

if (-not $BuildCacheDirectory) {
    $BuildCacheDirectory = Join-Path $env:LOCALAPPDATA "BuzzCodexLabBuild"
}
$BuildCacheDirectory = [IO.Path]::GetFullPath($BuildCacheDirectory)
$CargoTargetDirectory = Join-Path $BuildCacheDirectory "target"

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    Push-Location -LiteralPath $WorkingDirectory
    try {
        & $FilePath @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$FilePath exited with status $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "The Codex Lab installer can only be built on Windows."
}

foreach ($command in @("cargo.exe", "corepack.cmd", "node.exe", "rustc.exe")) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        throw "Required build command was not found: $command"
    }
}

$HostTriple = (& rustc.exe -vV | Select-String -Pattern '^host:\s+(.+)$').Matches.Groups[1].Value
if ($HostTriple -ne $Target) {
    throw "This test installer currently requires a native $Target toolchain; found $HostTriple."
}

# Lab builds must never inherit local relay addresses, credentials, reconnect
# hooks, or updater signing configuration from the packaging shell.
$BuildEnvironmentKeys = @(
    "BUZZ_RELAY_URL",
    "BUZZ_RELAY_HTTP",
    "BUZZ_BUILD_AGENT_ENV",
    "BUZZ_BUILD_BUZZ_AGENT_PROVIDER",
    "BUZZ_BUILD_BUZZ_AGENT_MODEL",
    "BUZZ_BUILD_RELAY_RECONNECT_CMD",
    "BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY",
    "BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY",
    "BUZZ_UPDATER_PUBLIC_KEY",
    "BUZZ_UPDATER_ENDPOINT",
    "BUZZ_DESKTOP_BUILD_DEEP_LINK_SCHEME",
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
)
foreach ($key in $BuildEnvironmentKeys) {
    Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue
}
$DeepLinkScheme = "buzz-codex-lab"
$env:BUZZ_DESKTOP_BUILD_DEEP_LINK_SCHEME = $DeepLinkScheme

# Rust embeds dependency source paths in panic metadata and MSVC may record an
# absolute PDB path in each PE image. Remap the packaging account's home and
# strip release symbols so distributing a lab build does not disclose the
# builder's Windows username or checkout location.
$UserHome = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
$PathRemapFlag = "--remap-path-prefix=$UserHome=C:\build-user"
$PdbPathFlag = "-C link-arg=/PDBALTPATH:%_PDB%"
$ExistingRustFlags = [string]$env:RUSTFLAGS
$env:RUSTFLAGS = "$ExistingRustFlags $PathRemapFlag $PdbPathFlag".Trim()
$env:CARGO_PROFILE_RELEASE_STRIP = "symbols"
# Opus and AWS-LC embed __FILE__ strings from native C/C++ dependencies. Rust's
# path remapping cannot reach those sources, so trim the same home prefix in
# MSVC as well.
$NativePathTrimFlag = "/d1trimfile:$UserHome\"
$ExistingCFlags = [string]$env:CFLAGS
$ExistingCxxFlags = [string]$env:CXXFLAGS
$env:CFLAGS = "$ExistingCFlags $NativePathTrimFlag".Trim()
$env:CXXFLAGS = "$ExistingCxxFlags $NativePathTrimFlag".Trim()
# CMake/MSBuild FileTracker fails on deeply nested Cargo paths on some Windows
# installations. A stable short cache also makes repeated packaging builds fast.
$env:CARGO_TARGET_DIR = $CargoTargetDirectory

if (-not $SkipDependencyInstall) {
    Invoke-NativeCommand -FilePath "corepack.cmd" -Arguments @(
        "pnpm",
        "install",
        "--frozen-lockfile"
    ) -WorkingDirectory $RepositoryRoot
}

if (-not $SkipSidecarBuild) {
    Invoke-NativeCommand -FilePath "cargo.exe" -Arguments @(
        "build",
        "--release",
        "--target",
        $Target,
        "-p", "buzz-acp",
        "-p", "buzz-agent",
        "-p", "buzz-dev-mcp",
        "-p", "git-credential-nostr",
        "-p", "buzz-cli"
    ) -WorkingDirectory $RepositoryRoot

    $Sidecars = @(
        "buzz-acp",
        "buzz-agent",
        "buzz-dev-mcp",
        "git-credential-nostr",
        "buzz"
    )
    New-Item -ItemType Directory -Path $BinariesDirectory -Force | Out-Null
    foreach ($sidecar in $Sidecars) {
        $source = Join-Path $CargoTargetDirectory "$Target\release\$sidecar.exe"
        $destination = Join-Path $BinariesDirectory "$sidecar-$Target.exe"
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Built sidecar was not found: $source"
        }
        Copy-Item -LiteralPath $source -Destination $destination -Force
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne
            (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash) {
            throw "Sidecar verification failed: $sidecar"
        }
    }
}

$RequiredSidecars = @(
    "buzz-acp-$Target.exe",
    "buzz-agent-$Target.exe",
    "buzz-dev-mcp-$Target.exe",
    "git-credential-nostr-$Target.exe",
    "buzz-$Target.exe"
)
foreach ($sidecar in $RequiredSidecars) {
    $path = Join-Path $BinariesDirectory $sidecar
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required bundled sidecar is missing: $path"
    }
}

if (-not $SkipChecks) {
    Invoke-NativeCommand -FilePath "corepack.cmd" -Arguments @(
        "pnpm",
        "typecheck"
    ) -WorkingDirectory $DesktopDirectory
    Invoke-NativeCommand -FilePath "node.exe" -Arguments @(
        "--import", "./test-loader.mjs",
        "--experimental-strip-types",
        "--test", "src/features/agents/lib/managedAgentControlActions.test.mjs"
    ) -WorkingDirectory $DesktopDirectory
}

# Native build scripts do not consistently tell Cargo that CFLAGS changed.
# Remove only the two native dependency caches that can embed absolute source
# paths so the trim flag above is guaranteed to take effect in release output.
Invoke-NativeCommand -FilePath "cargo.exe" -Arguments @(
    "clean",
    "--release",
    "--target", $Target,
    "-p", "audiopus_sys",
    "-p", "aws-lc-sys"
) -WorkingDirectory $TauriDirectory

Invoke-NativeCommand -FilePath "corepack.cmd" -Arguments @(
    "pnpm",
    "tauri",
    "build",
    "--target", $Target,
    "--bundles", "nsis",
    "--config", $ConfigPath
) -WorkingDirectory $DesktopDirectory

$BundleDirectory = Join-Path $CargoTargetDirectory "$Target\release\bundle\nsis"
$Installer = Get-ChildItem -LiteralPath $BundleDirectory -Filter "*.exe" -File |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $Installer) {
    throw "No NSIS installer was produced in $BundleDirectory"
}

$BaseConfig = Get-Content -LiteralPath (Join-Path $TauriDirectory "tauri.conf.json") -Raw -Encoding UTF8 |
    ConvertFrom-Json
$Commit = (& git.exe -C $RepositoryRoot rev-parse --short=12 HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or -not $Commit) {
    throw "Could not resolve the source commit."
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$ArtifactName = "Buzz-Codex-Lab_$($BaseConfig.version)_${Commit}_x64-setup.exe"
$ArtifactPath = Join-Path $OutputDirectory $ArtifactName
Copy-Item -LiteralPath $Installer.FullName -Destination $ArtifactPath -Force

$ArtifactHash = (Get-FileHash -LiteralPath $ArtifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
$SidecarHashes = foreach ($sidecar in $RequiredSidecars) {
    $path = Join-Path $BinariesDirectory $sidecar
    [ordered]@{
        name = $sidecar
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        size = (Get-Item -LiteralPath $path).Length
    }
}
$BuildInfo = [ordered]@{
    product = "Buzz Codex Lab"
    version = [string]$BaseConfig.version
    commit = $Commit
    target = $Target
    built_at = (Get-Date).ToUniversalTime().ToString("o")
    installer = [ordered]@{
        name = $ArtifactName
        sha256 = $ArtifactHash
        size = (Get-Item -LiteralPath $ArtifactPath).Length
        signed = $false
    }
    bundled_sidecars = $SidecarHashes
    embedded_relay_configuration = $false
    embedded_identity_or_api_key = $false
    deep_link_scheme = $DeepLinkScheme
    builder_home_path_remapped = $true
    native_source_paths_trimmed = $true
    release_symbols_stripped = $true
}

$BuildInfo | ConvertTo-Json -Depth 6 |
    Set-Content -LiteralPath (Join-Path $OutputDirectory "BUILD-INFO.json") -Encoding UTF8
"$ArtifactHash  $ArtifactName" |
    Set-Content -LiteralPath (Join-Path $OutputDirectory "SHA256SUMS.txt") -Encoding ASCII

Write-Host ""
Write-Host "Buzz Codex Lab installer ready:"
Write-Host "  $ArtifactPath"
Write-Host "  SHA256: $ArtifactHash"
Write-Warning "This evaluation installer is unsigned and may trigger Windows SmartScreen."
