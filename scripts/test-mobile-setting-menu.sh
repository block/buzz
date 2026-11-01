#!/bin/bash
# Exercise the production UIKit settings menu on a booted iOS simulator.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
device=${1:?Pass a booted iOS simulator UDID}
flutter_root=$(sed -n 's/^FLUTTER_ROOT=//p' "$repo_root/mobile/ios/Flutter/Generated.xcconfig")
frameworks="$flutter_root/bin/cache/artifacts/engine/ios/Flutter.xcframework/ios-arm64_x86_64-simulator"
fixture=$(mktemp -d)
trap 'rm -r "$fixture"' EXIT
mkdir -p "$fixture/Frameworks/App.framework/flutter_assets/assets/fonts"
cp "$repo_root/mobile/assets/fonts/TablerIcons.ttf" "$fixture/Frameworks/App.framework/flutter_assets/assets/fonts/TablerIcons.ttf"
# Compile the production menu and invoke its actual UIKit UIActions.
cat "$repo_root/mobile/ios/Runner/NativeTabler.swift" \
  "$repo_root/mobile/ios/Runner/JumpToLatestGlassButton.swift" \
  "$repo_root/mobile/ios/NativeControlTests/SettingMenuSelectionTests.swift" > "$fixture/test.swift"
xcrun --sdk iphonesimulator swiftc -parse-as-library \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  -target "$(uname -m)-apple-ios16.0-simulator" \
  -F "$frameworks" -framework Flutter \
  -Xlinker -rpath -Xlinker "$frameworks" \
  "$fixture/test.swift" \
  -o "$fixture/setting-menu-tests"
xcrun simctl spawn "$device" "$fixture/setting-menu-tests"
