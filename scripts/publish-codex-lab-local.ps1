[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Endpoint,
    [string]$PublishDirectory = "$(Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "..\dist\codex-lab-local")",
    [string]$BuildCacheDirectory,
    [switch]$SkipDependencyInstall,
    [switch]$AllowInsecureEndpoint
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
    throw "Version must be semantic; got '$Version'."
}

$endpointUri = $null
if (-not [Uri]::TryCreate($Endpoint, [UriKind]::Absolute, [ref]$endpointUri)) {
    throw "Endpoint must be an absolute URL; got '$Endpoint'."
}
if ($endpointUri.Scheme -ne "https" -and -not $AllowInsecureEndpoint) {
    throw "Endpoint must use HTTPS unless -AllowInsecureEndpoint is specified."
}

$required = @(
    "BUZZ_UPDATER_PUBLIC_KEY",
    "TAURI_SIGNING_PRIVATE_KEY"
)
foreach ($name in $required) {
    if (-not ([string][Environment]::GetEnvironmentVariable($name)).Trim()) {
        throw "Missing required environment variable: $name"
    }
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$buildScript = Join-Path $PSScriptRoot "build-codex-lab-windows.ps1"
$manifestScript = Join-Path $PSScriptRoot "generate-codex-lab-latest.mjs"
$publishPath = [IO.Path]::GetFullPath($PublishDirectory)
$stagingPath = Join-Path $publishPath ".build"
$endpointDirectory = $endpointUri.AbsolutePath.Substring(0, $endpointUri.AbsolutePath.LastIndexOf('/') + 1)

New-Item -ItemType Directory -Path $publishPath -Force | Out-Null
if (Test-Path $stagingPath) {
    Remove-Item -LiteralPath $stagingPath -Recurse -Force
}

$env:BUZZ_UPDATER_ENDPOINT = $Endpoint
$buildArgs = @(
    "-EnableUpdater",
    "-VersionOverride", $Version,
    "-OutputDirectory", $stagingPath
)
if ($BuildCacheDirectory) {
    $buildArgs += @("-BuildCacheDirectory", $BuildCacheDirectory)
}
if ($SkipDependencyInstall) {
    $buildArgs += "-SkipDependencyInstall"
}
if ($AllowInsecureEndpoint) {
    $buildArgs += "-AllowInsecureUpdaterEndpoint"
}

& powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $buildScript @buildArgs
if ($LASTEXITCODE -ne 0) {
    throw "Codex Lab build failed with exit code $LASTEXITCODE."
}

$buildInfo = Get-Content (Join-Path $stagingPath "BUILD-INFO.json") -Raw | ConvertFrom-Json
if ($buildInfo.version -ne $Version -or -not $buildInfo.updater.enabled) {
    throw "Build metadata does not describe the requested signed updater build."
}
$installer = Join-Path $stagingPath $buildInfo.installer.name
$signature = "$installer.sig"
if (-not (Test-Path $installer) -or -not (Test-Path $signature)) {
    throw "Signed installer artifacts are missing from $stagingPath."
}

Copy-Item $installer $publishPath -Force
Copy-Item $signature $publishPath -Force
Copy-Item (Join-Path $stagingPath "BUILD-INFO.json") $publishPath -Force
Copy-Item (Join-Path $stagingPath "SHA256SUMS.txt") $publishPath -Force

$artifactUrl = [Uri]::new($endpointUri, "$endpointDirectory$($buildInfo.installer.name)").AbsoluteUri
$manifestArgs = @(
    "--version", $Version,
    "--signature-file", (Join-Path $publishPath (Split-Path $signature -Leaf)),
    "--url", $artifactUrl,
    "--output", (Join-Path $publishPath "latest.json")
)
if ($AllowInsecureEndpoint) {
    $manifestArgs += "--allow-insecure"
}
& node $manifestScript @manifestArgs
if ($LASTEXITCODE -ne 0) {
    throw "Failed to generate latest.json."
}

Remove-Item $stagingPath -Recurse -Force
Write-Host "Local Buzz Codex Lab updater published:"
Write-Host "  Directory: $publishPath"
Write-Host "  Manifest:  $Endpoint"
Write-Host "  Installer: $($buildInfo.installer.name)"
