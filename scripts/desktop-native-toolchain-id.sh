#!/usr/bin/env bash
set -euo pipefail

platform=${1:?usage: desktop-native-toolchain-id.sh <macos|linux|windows>}
case "$platform" in
  macos)
    {
      sw_vers
      xcodebuild -version
      xcrun --sdk macosx --show-sdk-path
      xcrun --sdk macosx --show-sdk-version
      xcrun clang --version
    } ;;
  linux)
    {
      cat /etc/os-release
      dpkg-query -W -f='${Package}=${Version}\n' \
        build-essential libasound2-dev libayatana-appindicator3-dev \
        libgtk-3-dev librsvg2-dev libssl-dev libwebkit2gtk-4.1-dev \
        libxdo-dev patchelf pkg-config
      gcc -dumpfullversion -dumpversion
      gcc -dumpmachine
      ld --version
    } ;;
  windows)
    {
      cmd.exe //c ver
      powershell.exe -NoProfile -Command '$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"; & $vswhere -latest -products * -property installationVersion; Get-ChildItem "${env:ProgramFiles}\Microsoft Visual Studio\2022" -Directory -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name; Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\Include" -Directory | Select-Object -ExpandProperty Name; Get-ChildItem "${env:ProgramFiles}\Microsoft Visual Studio\2022\*\VC\Tools\MSVC" -Directory | Select-Object -ExpandProperty Name'
      cmake --version
    } ;;
  *)
    echo "unsupported native toolchain platform: $platform" >&2
    exit 1 ;;
esac | tr -d '\r' | python3 -c 'import hashlib, sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())'
