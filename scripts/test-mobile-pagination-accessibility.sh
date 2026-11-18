#!/bin/bash
# Test the actual UIKit paginator and Flutter bridge on a booted iOS simulator.
# Usage: /bin/bash scripts/test-mobile-pagination-accessibility.sh <simulator-udid>
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
device=${1:?Pass a booted iOS simulator UDID}
flutter_root=$(sed -n 's/^FLUTTER_ROOT=//p' "$repo_root/mobile/ios/Flutter/Generated.xcconfig")
frameworks="$flutter_root/bin/cache/artifacts/engine/ios/Flutter.xcframework/ios-arm64_x86_64-simulator"
fixture=$(mktemp -d)
trap 'rm -r "$fixture"' EXIT
xcrun --sdk iphonesimulator swiftc \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  -target "$(uname -m)-apple-ios15.0-simulator" \
  -F "$frameworks" -framework Flutter \
  -Xlinker -rpath -Xlinker "$frameworks" \
  "$repo_root/mobile/ios/Runner/ThemePaginationGeometry.swift" \
  "$repo_root/mobile/ios/Runner/ThemePaginationGlassControl.swift" \
  "$repo_root/mobile/ios/NativeControlTests/ThemePaginationAccessibilityTests.swift" \
  -o "$fixture/pagination-accessibility-tests"
xcrun simctl spawn "$device" "$fixture/pagination-accessibility-tests"
