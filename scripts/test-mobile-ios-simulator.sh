#!/usr/bin/env bash
# Exercise the production app and native ML Kit bridge on a fresh simulator.
set -euo pipefail
cd "$(dirname "$0")/.."
. ./bin/activate-hermit

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo 'This regression check requires an Apple Silicon Mac with Xcode.' >&2
  exit 1
fi

simulator_id=$(python3 - <<'PY'
import json
import re
import subprocess

devices = json.loads(subprocess.check_output(
    ['xcrun', 'simctl', 'list', 'devices', 'available', '--json'], text=True))['devices']
for runtime in sorted(devices, key=lambda key: tuple(map(int, re.findall(r'\d+', key))), reverse=True):
    if '.iOS-' not in runtime:
        continue
    for device in devices[runtime]:
        if device['name'].startswith('iPhone') and device.get('isAvailable'):
            print(subprocess.check_output([
                'xcrun', 'simctl', 'create', 'Buzz simulator smoke',
                device['deviceTypeIdentifier'], runtime,
            ], text=True).strip())
            raise SystemExit(0)
raise SystemExit('Install an iOS runtime and iPhone simulator in Xcode first.')
PY
)
cleanup() {
  xcrun simctl shutdown "$simulator_id" >/dev/null 2>&1 || true
  xcrun simctl delete "$simulator_id"
}
trap cleanup EXIT
xcrun simctl boot "$simulator_id"
open -a Simulator --args -CurrentDeviceUDID "$simulator_id"
python3 - "$simulator_id" <<'PY'
import subprocess
import sys
# A fresh runtime performs first-boot migrations, which are slower on CI hosts.
subprocess.run(['xcrun', 'simctl', 'bootstatus', sys.argv[1], '-b'], check=True, timeout=300)
PY

unset GIT_DIR GIT_WORK_TREE
cd mobile
flutter test integration_test/simulator_smoke_test.dart --no-pub -d "$simulator_id"
