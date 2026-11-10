#!/bin/bash
# Exercise the production native gallery on a booted iOS simulator.
# Usage: /bin/bash scripts/test-mobile-photo-zoom.sh <simulator-udid>
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
device=${1:?Pass a booted iOS simulator UDID}
flutter_root=$(sed -n 's/^FLUTTER_ROOT=//p' "$repo_root/mobile/ios/Flutter/Generated.xcconfig")
frameworks="$flutter_root/bin/cache/artifacts/engine/ios/Flutter.xcframework/ios-arm64_x86_64-simulator"
fixture=$(mktemp -d)
trap 'rm -r "$fixture"' EXIT
cat "$repo_root/mobile/ios/Runner/NativePhotoZoomCoordinator.swift" \
  "$repo_root/mobile/ios/NativeControlTests/NativePhotoZoomTests.swift" > "$fixture/test.swift"
xcrun --sdk iphonesimulator swiftc -parse-as-library \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  -target "$(uname -m)-apple-ios18.0-simulator" \
  -F "$frameworks" -framework Flutter \
  -Xlinker -rpath -Xlinker "$frameworks" \
  "$fixture/test.swift" -o "$fixture/photo-zoom-tests"
xcrun simctl spawn "$device" "$fixture/photo-zoom-tests"
