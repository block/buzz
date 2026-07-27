$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $repoRoot 'desktop\src-tauri\resources\mesh-llm\windows-x86_64'
New-Item -ItemType Directory -Force -Path $dest | Out-Null

$required = @(
  'libgcc_s_seh-1.dll',
  'libstdc++-6.dll',
  'libgomp-1.dll',
  'libwinpthread-1.dll'
)

$candidateDirs = @(
  'C:\msys64\mingw64\bin',
  'C:\msys64\ucrt64\bin',
  'C:\ProgramData\mingw64\mingw64\bin',
  'C:\Program Files\Git\mingw64\bin'
)

foreach ($name in $required) {
  $source = $null
  foreach ($dir in $candidateDirs) {
    $path = Join-Path $dir $name
    if (Test-Path -LiteralPath $path -PathType Leaf) {
      $source = $path
      break
    }
  }
  if (-not $source) {
    throw "Missing Windows MeshLLM runtime dependency: $name"
  }
  Copy-Item -LiteralPath $source -Destination (Join-Path $dest $name) -Force
  Write-Host "Bundled $name from $source"
}
