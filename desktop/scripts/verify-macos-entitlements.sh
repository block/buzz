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
HELPER_PATH="$APP_PATH/Contents/MacOS/buzz-apple-inputs"
[[ -x "$HELPER_PATH" ]] || { echo "Missing executable Apple-input helper: $HELPER_PATH" >&2; exit 1; }
codesign --verify --strict --verbose=2 "$HELPER_PATH"

ENTITLEMENTS="$(mktemp -t buzz-entitlements)"
trap 'rm -f "$ENTITLEMENTS"' EXIT

app_entitlements=(
  com.apple.security.device.audio-input
  com.apple.security.device.camera
  com.apple.security.cs.disable-library-validation
  com.apple.security.personal-information.calendars
  com.apple.security.automation.apple-events
)

helper_entitlements=(
  com.apple.security.personal-information.calendars
  com.apple.security.automation.apple-events
)

verify_entitlements() {
  local executable=$1
  shift
  : >"$ENTITLEMENTS"
  codesign --display --entitlements "$ENTITLEMENTS" --xml "$executable" 2>/dev/null
  [[ -s "$ENTITLEMENTS" ]] || {
    echo "Signed executable has no embedded entitlements: $executable" >&2
    exit 1
  }
  for entitlement in "$@"; do
    value="$(/usr/libexec/PlistBuddy -c "Print :$entitlement" "$ENTITLEMENTS" 2>/dev/null || true)"
    if [[ "$value" != "true" ]]; then
      echo "Signed executable is missing required entitlement: $entitlement ($executable)" >&2
      exit 1
    fi
  done
}

verify_entitlements "$EXECUTABLE_PATH" "${app_entitlements[@]}"
verify_entitlements "$HELPER_PATH" "${helper_entitlements[@]}"

for usage_key in \
  NSCalendarsUsageDescription \
  NSCalendarsFullAccessUsageDescription \
  NSRemindersUsageDescription \
  NSRemindersFullAccessUsageDescription \
  NSAppleEventsUsageDescription; do
  /usr/libexec/PlistBuddy -c "Print :$usage_key" "$INFO_PLIST" >/dev/null
done

echo "Verified required macOS entitlements, privacy strings, and Apple-input helper"
