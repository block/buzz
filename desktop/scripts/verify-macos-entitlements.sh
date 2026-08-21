#!/usr/bin/env bash
# Fail a macOS release if the signing service dropped Buzz's entitlements.

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <path-to-Buzz.app>" >&2
  exit 2
fi

APP_PATH="$1"
INFO_PLIST="$APP_PATH/Contents/Info.plist"

[[ -d "$APP_PATH" ]] || { echo "Missing app bundle: $APP_PATH" >&2; exit 1; }
[[ -f "$INFO_PLIST" ]] || { echo "Missing app Info.plist: $INFO_PLIST" >&2; exit 1; }

EXECUTABLE_NAME="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$INFO_PLIST")"
EXECUTABLE_PATH="$APP_PATH/Contents/MacOS/$EXECUTABLE_NAME"
[[ -f "$EXECUTABLE_PATH" ]] || { echo "Missing app executable: $EXECUTABLE_PATH" >&2; exit 1; }

ENTITLEMENTS="$(mktemp -t buzz-entitlements)"
trap 'rm -f "$ENTITLEMENTS"' EXIT

codesign --display --entitlements "$ENTITLEMENTS" --xml "$EXECUTABLE_PATH" 2>/dev/null
[[ -s "$ENTITLEMENTS" ]] || {
  echo "Signed app has no embedded entitlements: $EXECUTABLE_PATH" >&2
  exit 1
}

required_entitlements=(
  com.apple.security.device.audio-input
  com.apple.security.device.camera
  com.apple.security.cs.disable-library-validation
)

for entitlement in "${required_entitlements[@]}"; do
  value="$(/usr/libexec/PlistBuddy -c "Print :$entitlement" "$ENTITLEMENTS" 2>/dev/null || true)"
  if [[ "$value" != "true" ]]; then
    echo "Signed app is missing required entitlement: $entitlement" >&2
    exit 1
  fi
done

echo "Verified required macOS entitlements on $EXECUTABLE_PATH"
