#!/bin/bash
# Exercise the production UIKit Me agent row on a booted iOS simulator.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
device=${1:?Pass a booted iOS simulator UDID}
flutter_root=$(sed -n 's/^FLUTTER_ROOT=//p' "$repo_root/mobile/ios/Flutter/Generated.xcconfig")
frameworks="$flutter_root/bin/cache/artifacts/engine/ios/Flutter.xcframework/ios-arm64_x86_64-simulator"
fixture=$(mktemp -d)
trap 'rm -r "$fixture"' EXIT
# The standalone executable has no Flutter asset manifest. Keep the production
# renderer and font, adjusting only asset lookup for this test bundle.
mkdir -p "$fixture/flutter_assets/assets/fonts"
cp "$repo_root/mobile/assets/fonts/TablerIcons.ttf" "$fixture/flutter_assets/assets/fonts/"
sed 's/FlutterDartProject.lookupKey(forAsset: "assets\/fonts\/TablerIcons.ttf")/"flutter_assets\/assets\/fonts\/TablerIcons.ttf"/' \
  "$repo_root/mobile/ios/Runner/NativeTabler.swift" > "$fixture/NativeTabler.swift"
# Compile in one file to exercise the private production controller directly.
cat "$fixture/NativeTabler.swift" \
  "$repo_root/mobile/ios/Runner/MeAvatarLoader.swift" \
  "$repo_root/mobile/ios/Runner/NativeProfileTextEditor.swift" \
  "$repo_root/mobile/ios/NativeControlTests/MeAgentAccessibilityTests.swift" > "$fixture/test.swift"
xcrun --sdk iphonesimulator swiftc -parse-as-library \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  -target "$(uname -m)-apple-ios16.0-simulator" \
  -F "$frameworks" -framework Flutter \
  -Xlinker -rpath -Xlinker "$frameworks" \
  "$fixture/test.swift" \
  -o "$fixture/me-agent-tests"
xcrun simctl spawn "$device" "$fixture/me-agent-tests"
