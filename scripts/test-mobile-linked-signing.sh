#!/usr/bin/env bash
# Run the Dart production signer through symbols exported by the actual Runner.
set -euo pipefail
cd "$(dirname "$0")/.."
. ./bin/activate-hermit
device=${1:?Pass a booted simulator UDID}
fixture=$(mktemp -d)
bundle=xyz.block.buzz.signing-probe
trap 'xcrun simctl terminate "$device" "$bundle" >/dev/null 2>&1 || true; rm -rf "$fixture"' EXIT
xcodebuild -workspace mobile/ios/Runner.xcworkspace -scheme Runner \
  -configuration Debug -sdk iphonesimulator -destination "id=$device" \
  -derivedDataPath "${BUZZ_SIGNING_DERIVED_DATA:-$fixture/build}" -jobs 2 \
  BUNDLE_IDENTIFIER="$bundle" CODE_SIGNING_ALLOWED=NO ONLY_ACTIVE_ARCH=YES \
  FLUTTER_TARGET="${BUZZ_SIGNING_PROBE_TARGET:-tool/native_signing_probe.dart}" build > "$fixture/build.log" 2>&1 || {
    cat "$fixture/build.log"
    exit 1
  }
xcrun simctl install "$device" "${BUZZ_SIGNING_DERIVED_DATA:-$fixture/build}/Build/Products/Debug-iphonesimulator/Buzz.app"
launch=$(xcrun simctl launch --terminate-running-process "$device" "$bundle")
probe_pid=${launch##*: }
[[ "$probe_pid" =~ ^[0-9]+$ ]] || { echo "Unexpected launch result: $launch" >&2; exit 1; }
# Flutter's iOS logger writes to the unified log, not redirected stdout.
for ((attempt = 0; attempt < 90; attempt++)); do
  xcrun simctl spawn "$device" log show --last 5m --style compact \
    --predicate "processIdentifier == $probe_pid AND eventMessage CONTAINS \"SIGNING_PROBE\"" \
    > "$fixture/probe.log" 2>/dev/null
  if grep -q 'SIGNING_PROBE FAILED:' "$fixture/probe.log"; then
    cat "$fixture/probe.log"
    exit 1
  fi
  if grep -q 'SIGNING_PROBE COMPLETE' "$fixture/probe.log"; then
    cat "$fixture/probe.log"
    exit 0
  fi
  sleep 1
done
cat "$fixture/probe.log"
echo 'Linked iOS signing probe did not complete' >&2
exit 1
