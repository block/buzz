#!/bin/bash
# Test the actual UIKit carousel and Flutter bridge on a booted iOS simulator.
# Usage: /bin/bash scripts/test-mobile-carousel.sh <simulator-udid>
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
  "$repo_root/mobile/ios/Runner/IosMediaCarousel.swift" \
  "$repo_root/mobile/ios/NativeControlTests/IosMediaCarouselTests.swift" \
  -o "$fixture/carousel-tests"
xcrun simctl spawn "$device" "$fixture/carousel-tests"
