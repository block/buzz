#!/usr/bin/env bash
# Reproducible Android emulator lifecycle for Buzz mobile UI tests.
# Toolchain and system image come from `nix develop .#mobile-android`;
# mutable AVD data stays isolated under XDG_STATE_HOME and can be reset safely.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
state_root="${BUZZ_ANDROID_EMULATOR_HOME:-${XDG_STATE_HOME:-$HOME/.local/state}/buzz/android-emulator}"
state_marker="$state_root/.buzz-android-emulator-state"
android_user_home="$state_root/user"
android_avd_home="$state_root/avd"
avd_name="buzz_pixel_api_${BUZZ_ANDROID_EMULATOR_API:-36}"
api="${BUZZ_ANDROID_EMULATOR_API:-36}"
abi="${BUZZ_ANDROID_EMULATOR_ABI:-x86_64}"
serial="${BUZZ_ANDROID_EMULATOR_SERIAL:-emulator-5556}"
port="${serial#emulator-}"
system_image="system-images;android-${api};default;${abi}"
log_file="$state_root/emulator.log"
emulator_sdk="${BUZZ_ANDROID_EMULATOR_SDK:-${ANDROID_HOME:-}}"
emulator_sdk_package="${emulator_sdk%/libexec/android-sdk}"
avdmanager_bin="$emulator_sdk_package/bin/avdmanager"
emulator_bin="$emulator_sdk_package/bin/emulator"

export ANDROID_USER_HOME="$android_user_home"
export ANDROID_AVD_HOME="$android_avd_home"
export ANDROID_SERIAL="$serial"

usage() {
  cat <<'EOF'
Usage: scripts/mobile-android-emulator.sh <command> [args]

Commands:
  start [--window]        Create and boot the isolated emulator
  stop                    Stop the Buzz emulator
  status                  Print emulator and AVD status
  reset                   Stop and delete only the isolated Buzz AVD state
  screenshot [path]       Save a PNG (default: test-results/mobile-emulator/device.png)
  test <target>           Run the selected Flutter integration test

Run inside: nix develop .#mobile-android
EOF
}

require_toolchain() {
  : "${ANDROID_HOME:?Enter nix develop .#mobile-android first}"
  : "${emulator_sdk:?The mobile-android shell did not provide an emulator SDK}"
  for tool in adb flutter; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      echo "missing $tool; enter nix develop .#mobile-android" >&2
      exit 1
    fi
  done
  for tool in "$avdmanager_bin" "$emulator_bin"; do
    if [[ ! -x "$tool" ]]; then
      echo "missing $tool; enter nix develop .#mobile-android" >&2
      exit 1
    fi
  done
}

device_ready() {
  adb -s "$serial" get-state >/dev/null 2>&1 &&
    [[ "$(adb -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]
}

create_avd() {
  mkdir -p "$android_user_home" "$android_avd_home"
  touch "$state_marker"
  if ANDROID_HOME="$emulator_sdk" ANDROID_SDK_ROOT="$emulator_sdk" \
    "$avdmanager_bin" list avd | grep -Fq "Name: $avd_name"; then
    return
  fi

  echo "Creating $avd_name from $system_image"
  printf 'no\n' | ANDROID_HOME="$emulator_sdk" ANDROID_SDK_ROOT="$emulator_sdk" \
    "$avdmanager_bin" create avd \
    --force \
    --name "$avd_name" \
    --package "$system_image"

  cat >>"$android_avd_home/$avd_name.avd/config.ini" <<'EOF'
hw.keyboard = yes
hw.lcd.width = 1008
hw.lcd.height = 2244
hw.lcd.density = 360
showDeviceFrame = no
skin.dynamic = yes
EOF
}

start_emulator() {
  local window_mode="${1:-}"
  if device_ready; then
    echo "$serial is already ready"
    return
  fi

  if adb devices | awk 'NR > 1 {print $1}' | grep -Fxq "$serial"; then
    echo "$serial exists but is not ready; stop or reset it before retrying" >&2
    exit 1
  fi

  create_avd
  mkdir -p "$state_root"

  local -a flags=(
    -avd "$avd_name"
    -port "$port"
    -no-audio
    -no-boot-anim
    -no-snapshot
    -gpu swiftshader_indirect
  )
  if [[ "$window_mode" != "--window" ]]; then
    flags+=(-no-window)
  fi
  if [[ ! -r /dev/kvm ]]; then
    echo "KVM is unavailable; using slower software emulation"
    flags+=(-accel off)
  fi

  echo "Starting $avd_name as $serial"
  ANDROID_HOME="$emulator_sdk" ANDROID_SDK_ROOT="$emulator_sdk" \
    nohup "$emulator_bin" "${flags[@]}" </dev/null >"$log_file" 2>&1 &

  local deadline=$((SECONDS + 600))
  until device_ready; do
    if ((SECONDS >= deadline)); then
      echo "emulator did not become ready; see $log_file" >&2
      exit 1
    fi
    sleep 5
  done

  adb -s "$serial" shell settings put global window_animation_scale 0
  adb -s "$serial" shell settings put global transition_animation_scale 0
  adb -s "$serial" shell settings put global animator_duration_scale 0
  adb -s "$serial" shell settings put system pointer_location 0
  adb -s "$serial" shell settings put system show_touches 0
  adb -s "$serial" shell input keyevent 82
  echo "$serial is ready"
}

stop_emulator() {
  if adb -s "$serial" get-state >/dev/null 2>&1; then
    adb -s "$serial" emu kill >/dev/null
    echo "stopped $serial"
  else
    echo "$serial is not running"
  fi
}

reset_emulator() {
  stop_emulator
  if [[ ! -e "$state_root" ]]; then
    echo "isolated emulator state is already absent: $state_root"
    return
  fi
  case "$state_root" in
    "" | / | "$HOME")
      echo "refusing unsafe emulator state path: $state_root" >&2
      exit 1
      ;;
  esac
  if [[ ! -f "$state_marker" ]]; then
    echo "refusing unrecognized emulator state path: $state_root" >&2
    exit 1
  fi
  rm -rf -- "$state_root"
  echo "removed isolated emulator state: $state_root"
}

take_screenshot() {
  local output="${1:-$repo_root/test-results/mobile-emulator/device.png}"
  if ! device_ready; then
    echo "$serial is not ready" >&2
    exit 1
  fi
  mkdir -p "$(dirname "$output")"
  adb -s "$serial" exec-out screencap -p >"$output"
  echo "$output"
}

run_test() {
  local target="${1:?test requires an integration-test target}"
  start_emulator
  "$repo_root/scripts/mobile-worktree-overrides.sh"
  mkdir -p "$repo_root/test-results/mobile-emulator"
  (
    cd "$repo_root/mobile"
    BUZZ_MOBILE_SCREENSHOT_DIR="$repo_root/test-results/mobile-emulator" \
      flutter drive \
      --driver=test_driver/integration_test.dart \
      --target="$target" \
      --device-id="$serial"
  )
}

require_toolchain
case "${1:-}" in
  start)
    start_emulator "${2:-}"
    ;;
  stop)
    stop_emulator
    ;;
  status)
    echo "state: $state_root"
    echo "avd: $avd_name ($system_image)"
    adb devices -l | grep -E "^List|^${serial}[[:space:]]" || true
    ;;
  reset)
    reset_emulator
    ;;
  screenshot)
    take_screenshot "${2:-}"
    ;;
  test)
    run_test "${2:-}"
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
