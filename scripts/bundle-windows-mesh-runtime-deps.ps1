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

function Find-MeshRuntimeDependency($name) {
  foreach ($dir in $candidateDirs) {
    $path = Join-Path $dir $name
    if (Test-Path -LiteralPath $path -PathType Leaf) {
      return $path
    }
  }
  return $null
}

function Get-MissingMeshRuntimeDependencies {
  $required | Where-Object { -not (Find-MeshRuntimeDependency $_) }
}

$missing = @(Get-MissingMeshRuntimeDependencies)
if ($missing.Count -gt 0) {
  Write-Host "Missing Windows MeshLLM runtime dependencies before bootstrap: $($missing -join ', ')"
  $pacman = 'C:\msys64\usr\bin\pacman.exe'
  if (Test-Path -LiteralPath $pacman -PathType Leaf) {
    & $pacman -Sy --noconfirm --needed mingw-w64-x86_64-gcc
  }
}

$missing = @(Get-MissingMeshRuntimeDependencies)
if ($missing.Count -gt 0) {
  $choco = Get-Command choco -ErrorAction SilentlyContinue
  if ($choco) {
    & $choco.Source install mingw -y --no-progress | Out-Null
  }
}

$missing = @(Get-MissingMeshRuntimeDependencies)
if ($missing.Count -gt 0) {
  throw "Missing Windows MeshLLM runtime dependencies after bootstrap: $($missing -join ', ')"
}

foreach ($name in $required) {
  $source = Find-MeshRuntimeDependency $name
  Copy-Item -LiteralPath $source -Destination (Join-Path $dest $name) -Force
  Write-Host "Bundled $name from $source"
}
