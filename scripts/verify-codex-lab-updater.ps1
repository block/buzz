[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstallerPath,
    [Parameter(Mandatory = $true)][string]$SignaturePath,
    [Parameter(Mandatory = $true)][string]$PublicKey,
    [string]$BuildCacheDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$Target = "x86_64-pc-windows-msvc"
$ScriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path

$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$SignaturePath = (Resolve-Path -LiteralPath $SignaturePath).Path
if (-not $BuildCacheDirectory) {
    $BuildCacheDirectory = Join-Path $env:LOCALAPPDATA "BuzzCodexLabBuild"
}
$BuildCacheDirectory = [IO.Path]::GetFullPath($BuildCacheDirectory)
$DependencyDirectory = Join-Path $BuildCacheDirectory "target\$Target\release\deps"
$VerifierSource = Join-Path $ScriptDirectory "verify-codex-lab-updater.rs"

$Rlib = Get-ChildItem -LiteralPath $DependencyDirectory -Filter "libminisign_verify-*.rlib" -File |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $Rlib) {
    throw "minisign-verify was not found under $DependencyDirectory. Build the updater first."
}

$VerificationDirectory = Join-Path $BuildCacheDirectory "updater-verifier"
New-Item -ItemType Directory -Path $VerificationDirectory -Force | Out-Null
$DecodedPublicKeyPath = Join-Path $VerificationDirectory "updater.pub"
$DecodedSignaturePath = Join-Path $VerificationDirectory "installer.sig"
$VerifierBinary = Join-Path $VerificationDirectory "verify-updater.exe"

try {
    $DecodedPublicKey = [Text.Encoding]::UTF8.GetString(
        [Convert]::FromBase64String($PublicKey.Trim())
    )
    $EncodedSignature = Get-Content -LiteralPath $SignaturePath -Raw -Encoding UTF8
    $DecodedSignature = [Text.Encoding]::UTF8.GetString(
        [Convert]::FromBase64String($EncodedSignature.Trim())
    )
}
catch {
    throw "Updater public key or signature is not valid Tauri base64: $($_.Exception.Message)"
}

[IO.File]::WriteAllText($DecodedPublicKeyPath, $DecodedPublicKey, $Utf8NoBom)
[IO.File]::WriteAllText($DecodedSignaturePath, $DecodedSignature, $Utf8NoBom)

& rustc.exe `
    --edition=2021 `
    $VerifierSource `
    --extern "minisign_verify=$($Rlib.FullName)" `
    -L "dependency=$DependencyDirectory" `
    -o $VerifierBinary
if ($LASTEXITCODE -ne 0) {
    throw "rustc exited with status $LASTEXITCODE"
}

& $VerifierBinary $DecodedPublicKeyPath $DecodedSignaturePath $InstallerPath
if ($LASTEXITCODE -ne 0) {
    throw "Updater signature verification failed with status $LASTEXITCODE"
}

Write-Host "Updater signature verified: $InstallerPath"
