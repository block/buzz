param(
  [ValidateNotNullOrEmpty()]
  [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._ -]{0,63}$')]
  [string]$WslDistribution = 'Ubuntu'
)

$ErrorActionPreference = 'Stop'

Invoke-WebRequest -UseBasicParsing 'http://127.0.0.1:3000/_readiness' | Out-Null

$desktop = Join-Path $env:LOCALAPPDATA 'Buzz\buzz-desktop.exe'
$cli = Join-Path $env:LOCALAPPDATA 'Buzz\buzz.exe'
if (-not (Test-Path -LiteralPath $desktop -PathType Leaf)) {
  throw "Buzz Desktop is not installed at $desktop"
}
if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) {
  throw "The Buzz installation is incomplete: $cli is missing"
}
if (Get-Process -Name 'buzz-desktop' -ErrorAction SilentlyContinue) {
  throw 'Close the existing buzz-desktop process, then run this script again.'
}

$banker = (& wsl.exe -d $WslDistribution -- bash -lc `
  'awk -F= ''$1=="CORE_BANKER_PRIVATE_KEY" {printf "%s",$2}'' "$HOME/.config/core-buzz/agent.env"').Trim()
if ([string]::IsNullOrWhiteSpace($banker)) {
  throw 'Core banker identity is unavailable.'
}

$desktopProcess = $null
try {
  $env:BUZZ_PRIVATE_KEY = $banker
  $env:BUZZ_SHARE_IDENTITY = '1'
  $env:BUZZ_RELAY_URL = 'ws://127.0.0.1:3000'
  $desktopProcess = Start-Process -FilePath $desktop -PassThru
  Start-Sleep -Seconds 2
  if ($desktopProcess.HasExited) {
    throw "buzz-desktop exited during startup with code $($desktopProcess.ExitCode)"
  }
} finally {
  Remove-Item Env:\BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue
  Remove-Item Env:\BUZZ_SHARE_IDENTITY -ErrorAction SilentlyContinue
  Remove-Item Env:\BUZZ_RELAY_URL -ErrorAction SilentlyContinue
  $banker = $null
}

if ($null -eq $desktopProcess -or $desktopProcess.HasExited) {
  throw 'buzz-desktop did not remain running; the Core Lab deep link was not opened.'
}

Start-Process 'buzz://add-community?relay=ws%3A%2F%2F127.0.0.1%3A3000&name=Core%20Lab'
Write-Host 'buzz-desktop is running with the Core banker identity; confirm Core Lab in the add-community screen.'
