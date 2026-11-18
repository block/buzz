#!/bin/bash
# Exercise the production recipes with macOS's system Bash, not a rewritten copy.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/bin" "$fixture/scripts" "$fixture/mobile/ios/Flutter"
for recipe in mobile-dev mobile-build-android; do
  # Render with the real task runner before installing stubs: a Hermit just
  # shim can otherwise prepend the real Flutter to PATH ahead of our stub.
  just --justfile "$repo_root/Justfile" --dry-run "$recipe" > "$fixture/$recipe" 2>&1
done
cat > "$fixture/bin/flutter" <<'STUB'
#!/bin/bash
set -eu
printf '%s\n' "$@" > "$CALL_LOG"
STUB
cat > "$fixture/bin/xcode-select" <<'STUB'
#!/bin/bash
[[ "$*" == "-p" ]] || exit 1
printf '%s\n' "$SELECTED_DEVELOPER_DIR"
STUB
cat > "$fixture/bin/open" <<'STUB'
#!/bin/bash
printf '%s\n' "$@" >> "$OPEN_LOG"
[[ "$1" == -a && -d "$2" && "$OPEN_FAIL" == 0 ]]
STUB
cat > "$fixture/bin/sleep" <<'STUB'
#!/bin/bash
exit 0
STUB
cat > "$fixture/scripts/mobile-worktree-overrides.sh" <<'STUB'
#!/bin/bash
exit 0
STUB
chmod +x "$fixture/bin/flutter" "$fixture/bin/xcode-select" "$fixture/bin/open" "$fixture/bin/sleep" "$fixture/scripts/mobile-worktree-overrides.sh"
export PATH="$fixture/bin:$PATH"
export CALL_LOG="$fixture/arguments"
export OPEN_LOG="$fixture/open-calls"
# Spaces also exercise quoting of the selected Xcode path.
export SELECTED_DEVELOPER_DIR="$fixture/Xcode 27.app/Contents/Developer"
export OPEN_FAIL=0
mkdir -p "$SELECTED_DEVELOPER_DIR" "$SELECTED_DEVELOPER_DIR/../Applications/DeviceHub.app"
unset BUZZ_PUSH_GATEWAY_URL

for recipe in mobile-dev mobile-build-android; do
  for mode in omitted supplied; do
    unset BUZZ_PUSH_GATEWAY_URL
    if [[ "$mode" == supplied ]]; then
      # A space catches accidental splitting. The build gate validates URLs;
      # this test checks exact argument forwarding.
      export BUZZ_PUSH_GATEWAY_URL='https://push.example/value with space'
    fi
    (cd "$fixture" && /bin/bash "$fixture/$recipe")
    if [[ "$recipe" == mobile-dev ]]; then
      printf '%s\n' run > "$fixture/expected"
    else
      printf '%s\n' build apk --debug --no-pub > "$fixture/expected"
    fi
    if [[ "$mode" == supplied ]]; then
      printf '%s\n' "--dart-define=BUZZ_PUSH_GATEWAY_URL=$BUZZ_PUSH_GATEWAY_URL" >> "$fixture/expected"
    fi
    diff -u "$fixture/expected" "$CALL_LOG"
    printf 'PASS %s %s\n' "$recipe" "$mode"
  done
done

unset BUZZ_PUSH_GATEWAY_URL
printf '%s\n' 'BUZZ_PUSH_GATEWAY_URL = https:/$()/push.example' > "$fixture/mobile/ios/Flutter/AppOverrides.xcconfig"
(cd "$fixture" && /bin/bash "$fixture/mobile-dev")
printf '%s\n' run '--dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example' > "$fixture/expected"
diff -u "$fixture/expected" "$CALL_LOG"
printf 'PASS mobile-dev Xcode override\n'

# Both installations exist: selection must not depend on Launch Services order.
xcode26="$fixture/Xcode 26.app/Contents/Developer"
xcode27="$SELECTED_DEVELOPER_DIR"
mkdir -p "$xcode26/Applications/Simulator.app"
for scenario in selected-xcode26 selected-xcode27 launch-failure missing-app; do
  SELECTED_DEVELOPER_DIR="$xcode27"
  OPEN_FAIL=0
  case "$scenario" in
    selected-xcode26) SELECTED_DEVELOPER_DIR="$xcode26" ;;
    launch-failure) OPEN_FAIL=1 ;;
    missing-app) SELECTED_DEVELOPER_DIR="$fixture/Missing Xcode.app/Contents/Developer" ;;
  esac
  expected_app="$SELECTED_DEVELOPER_DIR/../Applications/DeviceHub.app"
  if [[ "$scenario" == selected-xcode26 || "$scenario" == missing-app ]]; then
    expected_app="$SELECTED_DEVELOPER_DIR/Applications/Simulator.app"
  fi
  rm -f "$CALL_LOG"
  : > "$OPEN_LOG"
  printf '%s\n' -a "$expected_app" > "$fixture/expected-open"
  if (cd "$fixture" && /bin/bash "$fixture/mobile-dev"); then
    [[ "$scenario" != launch-failure && "$scenario" != missing-app ]]
    diff -u "$fixture/expected" "$CALL_LOG"
  else
    [[ "$scenario" == launch-failure || "$scenario" == missing-app ]]
    [[ ! -e "$CALL_LOG" ]]
  fi
  diff -u "$fixture/expected-open" "$OPEN_LOG"
  printf 'PASS mobile-dev %s\n' "$scenario"
done
