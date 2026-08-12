[CmdletBinding()]
param(
    [switch]$SkipDependencyInstall,
    [string]$DeepLinkScheme = "buzz-codex-lab",
    [string]$ReleaseRepository = "chemyibinjiang/buzz"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($DeepLinkScheme -notmatch '^[a-z][a-z0-9+.-]*$') {
    throw "Invalid deep-link scheme: $DeepLinkScheme"
}
if ($ReleaseRepository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
    throw "Invalid GitHub release repository: $ReleaseRepository"
}

$ScriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $ScriptDirectory "..")).Path
$WebDirectory = Join-Path $RepositoryRoot "web"

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

if (-not (Get-Command "corepack.cmd" -ErrorAction SilentlyContinue)) {
    throw "Required build command was not found: corepack.cmd"
}

$PreviousScheme = [Environment]::GetEnvironmentVariable("VITE_BUZZ_DEEP_LINK_SCHEME")
$PreviousRepository = [Environment]::GetEnvironmentVariable("VITE_BUZZ_RELEASE_REPOSITORY")

try {
    $env:VITE_BUZZ_DEEP_LINK_SCHEME = $DeepLinkScheme
    $env:VITE_BUZZ_RELEASE_REPOSITORY = $ReleaseRepository

    if (-not $SkipDependencyInstall) {
        Invoke-NativeCommand -FilePath "corepack.cmd" -Arguments @(
            "pnpm",
            "install",
            "--frozen-lockfile"
        ) -WorkingDirectory $RepositoryRoot
    }

    Invoke-NativeCommand -FilePath "corepack.cmd" -Arguments @(
        "pnpm",
        "build"
    ) -WorkingDirectory $WebDirectory
}
finally {
    if ($null -eq $PreviousScheme) {
        Remove-Item -LiteralPath "Env:VITE_BUZZ_DEEP_LINK_SCHEME" -ErrorAction SilentlyContinue
    }
    else {
        $env:VITE_BUZZ_DEEP_LINK_SCHEME = $PreviousScheme
    }

    if ($null -eq $PreviousRepository) {
        Remove-Item -LiteralPath "Env:VITE_BUZZ_RELEASE_REPOSITORY" -ErrorAction SilentlyContinue
    }
    else {
        $env:VITE_BUZZ_RELEASE_REPOSITORY = $PreviousRepository
    }
}

Write-Host ""
Write-Host "Buzz Codex Lab invite site ready:"
Write-Host "  $(Join-Path $WebDirectory 'dist')"
Write-Host "  Deep-link scheme: $DeepLinkScheme"
Write-Host "  Release repository: $ReleaseRepository"
