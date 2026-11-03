#!/bin/bash
# Exercise the production UIKit create-channel field labels on a booted iOS simulator.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
device=${1:?Pass a booted iOS simulator UDID}
flutter_root=$(sed -n 's/^FLUTTER_ROOT=//p' "$repo_root/mobile/ios/Flutter/Generated.xcconfig")
frameworks="$flutter_root/bin/cache/artifacts/engine/ios/Flutter.xcframework/ios-arm64_x86_64-simulator"
fixture=$(mktemp -d)
trap 'rm -r "$fixture"' EXIT
# Compile in one file to exercise the private production controller directly.
cat "$repo_root/mobile/ios/Runner/NavigationTopEdge.swift" \
  "$repo_root/mobile/ios/Runner/NativeCreateChannelSheet.swift" \
  "$repo_root/mobile/ios/NativeControlTests/CreateChannelAccessibilityTests.swift" > "$fixture/test.swift"
xcrun --sdk iphonesimulator swiftc -parse-as-library \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  -target "$(uname -m)-apple-ios16.0-simulator" \
  -F "$frameworks" -framework Flutter \
  -Xlinker -rpath -Xlinker "$frameworks" \
  "$fixture/test.swift" \
  -o "$fixture/create-channel-tests"
xcrun simctl spawn "$device" "$fixture/create-channel-tests"
