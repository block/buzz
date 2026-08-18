#!/usr/bin/env bash
# Safety contract tests for scripts/mobile-android-emulator.sh.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_root="$(mktemp -d)"
trap 'rm -rf -- "$test_root"' EXIT

export BUZZ_ANDROID_EMULATOR_HOME="$test_root/initial"
# shellcheck source=mobile-android-emulator.sh
source "$repo_root/scripts/mobile-android-emulator.sh"

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

pass() {
  printf 'ok: %s\n' "$1"
}

set_state_root() {
  state_root="$1"
  state_marker="$state_root/.buzz-android-emulator-state"
  android_user_home="$state_root/user"
  android_avd_home="$state_root/avd"
  log_file="$state_root/emulator.log"
}

set_state_root "$test_root/pre-existing"
mkdir -p "$state_root"
printf 'keep\n' >"$state_root/unrelated-data"
if claim_state_root 2>/dev/null; then
  fail "a pre-existing unowned state root must be rejected"
fi
[[ -f "$state_root/unrelated-data" && ! -e "$state_marker" ]] ||
  fail "rejecting an unowned root must preserve its contents"
pass "pre-existing unowned state roots are preserved"

set_state_root "$test_root/fresh"
claim_state_root
[[ "$(<"$state_marker")" == "$state_marker_value" ]] ||
  fail "a fresh state root must carry the versioned ownership marker"
pass "fresh state roots receive a versioned ownership marker"

set_state_root "$test_root/legacy-marker"
mkdir -p "$state_root"
: >"$state_marker"
printf 'keep\n' >"$state_root/unrelated-data"
if reset_emulator 2>/dev/null; then
  fail "an empty legacy marker must not authorize deletion"
fi
[[ -f "$state_root/unrelated-data" ]] || fail "legacy-marker rejection must preserve data"
pass "legacy or forged empty markers cannot authorize reset"

fake_avd_name="$avd_name"
fake_device_present=1
kill_log="$test_root/killed"
adb() {
  if [[ "$*" == "devices" || "$*" == "devices -l" ]]; then
    printf 'List of devices attached\n'
    [[ "$fake_device_present" == "1" ]] && printf '%s\tdevice\n' "$serial"
  elif [[ "$*" == *"emu avd name"* ]]; then
    printf '%s\nOK\n' "$fake_avd_name"
  elif [[ "$*" == *"emu kill"* ]]; then
    printf 'killed\n' >"$kill_log"
  elif [[ "$*" == *"get-state"* ]]; then
    printf 'device\n'
  elif [[ "$*" == *"shell getprop sys.boot_completed"* ]]; then
    printf '1\n'
  fi
}

fake_avd_name="someone-elses-avd"
set_state_root "$test_root/start-foreign"
if start_emulator 2>/dev/null; then
  fail "start must reject a foreign AVD on the configured serial"
fi
[[ ! -e "$state_root" ]] || fail "foreign-device rejection must not create state"
pass "foreign AVDs are never adopted"

if stop_emulator 2>/dev/null; then
  fail "stop must reject a foreign AVD on the configured serial"
fi
[[ ! -e "$kill_log" ]] || fail "a foreign AVD must never receive emu kill"
pass "foreign AVDs are never stopped"

fake_avd_name="$avd_name"
set_state_root "$test_root/same-name-unowned"
if start_emulator 2>/dev/null; then
  fail "start must not adopt a same-named AVD without owned state"
fi
if stop_emulator 2>/dev/null; then
  fail "stop must not kill a same-named AVD without owned state"
fi
[[ ! -e "$state_root" && ! -e "$kill_log" ]] ||
  fail "same-name rejection must not create state or kill the device"
pass "same-named AVDs require Buzz-owned state"

fake_avd_name="someone-elses-avd"
set_state_root "$test_root/owned"
mkdir -p "$state_root"
printf '%s\n' "$state_marker_value" >"$state_marker"
printf 'keep\n' >"$state_root/owned-data"
if reset_emulator 2>/dev/null; then
  fail "reset must reject a foreign AVD before deleting owned state"
fi
[[ -f "$state_root/owned-data" && ! -e "$kill_log" ]] ||
  fail "foreign-device rejection must preserve the owned state root"
pass "reset preserves state when the configured serial belongs to another AVD"

fake_avd_name="$avd_name"
stop_emulator >/dev/null
[[ -f "$kill_log" ]] || fail "the expected Buzz AVD must be stoppable"
pass "the expected Buzz AVD can be stopped"

fake_device_present=0
rm -f "$kill_log"
set_state_root "$test_root/reset-stopped"
mkdir -p "$state_root"
printf '%s\n' "$state_marker_value" >"$state_marker"
reset_emulator >/dev/null
[[ ! -e "$state_root" && ! -e "$kill_log" ]] ||
  fail "reset of a stopped Buzz AVD must remove only its owned state"
pass "owned state resets cleanly when no emulator is running"
