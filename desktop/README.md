# Buzz

Desktop chat shell with:

- Tauri + React + TypeScript + Vite
- Tailwind CSS
- shadcn/ui-ready shared components
- Biome (lint/format/check)
- Feature-driven frontend structure

## Scripts

- `pnpm dev` - run the web frontend
- `pnpm tauri dev` - run the desktop app
- `pnpm build` - typecheck and build frontend
- `pnpm typecheck` - TypeScript checks
- `pnpm lint` - Biome lint
- `pnpm format` - Biome format (write)
- `pnpm check` - Biome check

## Structure

- `src/shared` - reusable app-wide code (`ui`, `lib`, `styles`)
- `src/features` - feature modules (vertical slices)
- `src/app` - top-level app composition

## Heartbeat-preflight packaging

Official package builds must prepare sidecars with
`scripts/bundle-sidecars.sh`. The script probes `buzz-acp` for the exact
heartbeat-preflight capability and writes a build-only attestation beside the
target binary. `build.rs` then verifies that attestation, re-probes native
targets, and embeds the executable-code digest used by the Desktop runtime.

Custom distributions that support owner heartbeat-preflight designations must
also set `BUZZ_BUILD_REQUIRE_HEARTBEAT_PREFLIGHT_SIDECAR=1` and, on macOS,
`BUZZ_BUILD_HEARTBEAT_HARNESS_MACOS_TEAM_IDENTIFIER` to their 10-character
signing TeamIdentifier. They must also set `BUZZ_BUILD_SOURCE_REVISION` to the
immutable 40- or 64-hex source commit used for operator instructions. The build
fails if the regular non-symlink attestation, exact capability, stable digest,
required signer pin, or immutable documentation revision is missing.
Development placeholder builds may omit these values. They continue to support
normal agents, while any designated agent refuses to start without a verified
bundled sidecar.

### Trusted heartbeat harness on macOS

Designated heartbeat agents deliberately do not execute `buzz-acp` from the
user-writable app bundle. After installing or updating Buzz, an administrator
must install the exact signed bundled harness into the root-owned trust domain.
The privileged commands below are fixed macOS system binaries; no script or
executable from the user-writable app bundle is ever run as root:

```sh
set -euo pipefail

TEAM_IDENTIFIER="EYF346PHUG"
SOURCE="/Applications/Buzz.app/Contents/MacOS/buzz-acp"
SYSTEM_PARENT="/Library/Application Support"
TARGET_PARENT="/Library/Application Support/Buzz"
TARGET_DIRECTORY="$TARGET_PARENT/TrustedHeartbeat"
TARGET="$TARGET_DIRECTORY/buzz-acp"
APP_REQUIREMENT="identifier \"xyz.block.buzz.app\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = \"$TEAM_IDENTIFIER\""
HARNESS_REQUIREMENT="identifier \"buzz-acp\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = \"$TEAM_IDENTIFIER\""

/usr/bin/codesign --verify --deep --strict --verbose=2 -R="$APP_REQUIREMENT" "/Applications/Buzz.app"
/usr/bin/codesign --verify --strict --verbose=2 -R="$HARNESS_REQUIREMENT" "$SOURCE"
test ! -L "$SOURCE"
SOURCE_SHA=$(/usr/bin/shasum -a 256 "$SOURCE" | /usr/bin/awk '{print $1}')

test ! -L "$SYSTEM_PARENT"
test "$(/usr/bin/stat -f '%u %Lp %HT' "$SYSTEM_PARENT")" = "0 755 Directory"
test "$(/bin/ls -lde "$SYSTEM_PARENT" | /usr/bin/wc -l | /usr/bin/tr -d ' ')" = "1"

if [ ! -e "$TARGET_PARENT" ] && [ ! -L "$TARGET_PARENT" ]; then
  sudo /usr/bin/install -d -o root -g wheel -m 0755 "$TARGET_PARENT"
fi
test ! -L "$TARGET_PARENT" && test ! -L "$TARGET_DIRECTORY"
test "$(/usr/bin/stat -f '%u %Lp %HT' "$TARGET_PARENT")" = "0 755 Directory"
sudo /bin/chmod -N "$TARGET_PARENT"
sudo /usr/sbin/chown root:wheel "$TARGET_PARENT"
sudo /bin/chmod 0755 "$TARGET_PARENT"

if [ ! -e "$TARGET_DIRECTORY" ] && [ ! -L "$TARGET_DIRECTORY" ]; then
  sudo /usr/bin/install -d -o root -g wheel -m 0755 "$TARGET_DIRECTORY"
fi
test ! -L "$TARGET_DIRECTORY"
test "$(/usr/bin/stat -f '%u %Lp %HT' "$TARGET_DIRECTORY")" = "0 755 Directory"
sudo /bin/chmod -N "$TARGET_DIRECTORY"
sudo /usr/sbin/chown root:wheel "$TARGET_DIRECTORY"
sudo /bin/chmod 0755 "$TARGET_DIRECTORY"
test "$(/bin/ls -lde "$TARGET_PARENT" | /usr/bin/wc -l | /usr/bin/tr -d ' ')" = "1"
test "$(/bin/ls -lde "$TARGET_DIRECTORY" | /usr/bin/wc -l | /usr/bin/tr -d ' ')" = "1"

TARGET_NEW=$(sudo /usr/bin/mktemp "$TARGET_DIRECTORY/.buzz-acp.XXXXXX")
case "$TARGET_NEW" in
  "$TARGET_DIRECTORY"/.buzz-acp.*) ;;
  *) exit 1 ;;
esac
cleanup() {
  if [ -n "${TARGET_NEW:-}" ]; then
    sudo /bin/rm -f "$TARGET_NEW"
  fi
}
trap cleanup EXIT HUP INT TERM
/bin/cat "$SOURCE" | sudo /usr/bin/tee "$TARGET_NEW" >/dev/null
sudo /bin/chmod -N "$TARGET_NEW"
sudo /usr/sbin/chown root:wheel "$TARGET_NEW"
sudo /bin/chmod 0755 "$TARGET_NEW"
TARGET_SHA=$(/usr/bin/shasum -a 256 "$TARGET_NEW" | /usr/bin/awk '{print $1}')
test "$SOURCE_SHA" = "$TARGET_SHA"
/usr/bin/codesign --verify --strict --verbose=2 -R="$HARNESS_REQUIREMENT" "$TARGET_NEW"
/usr/bin/codesign -dv --verbose=4 "$TARGET_NEW" 2>&1 | /usr/bin/grep -Eq 'flags=0x[[:xdigit:]]+\([^)]*runtime[^)]*\)'
test -z "$(/usr/bin/codesign -d --entitlements - --xml "$TARGET_NEW" 2>/dev/null)"
test "$(/bin/ls -lde "$TARGET_NEW" | /usr/bin/wc -l | /usr/bin/tr -d ' ')" = "1"
if [ -d "$TARGET" ] && [ ! -L "$TARGET" ]; then
  exit 1
fi
sudo /bin/mv -fh "$TARGET_NEW" "$TARGET"
TARGET_NEW=""
test ! -L "$TARGET"
test "$(/usr/bin/stat -f '%u %Lp %HT' "$TARGET")" = "0 755 Regular File"
test "$SOURCE_SHA" = "$(/usr/bin/shasum -a 256 "$TARGET" | /usr/bin/awk '{print $1}')"
/usr/bin/codesign --verify --strict --verbose=2 -R="$HARNESS_REQUIREMENT" "$TARGET"
/usr/bin/codesign -dv --verbose=4 "$TARGET" 2>&1 | /usr/bin/grep -Eq 'flags=0x[[:xdigit:]]+\([^)]*runtime[^)]*\)'
test -z "$(/usr/bin/codesign -d --entitlements - --xml "$TARGET" 2>/dev/null)"
test "$(/bin/ls -lde "$TARGET" | /usr/bin/wc -l | /usr/bin/tr -d ' ')" = "1"
test "$("$TARGET" heartbeat-preflight-capability)" = '{"kind":"buzz_acp_heartbeat_preflight_capability","protocol_version":1,"build_capability":"buzz-acp-source-witness-gateway-v1"}'
```

Only fixed macOS system utilities run with `sudo`; an unprivileged `cat` reads
the signed app binary and root receives only those bytes through standard
input into an exclusive root-created temporary file. The recipe refuses unsafe
parent paths, clears inherited
ACLs, authenticates the official signing identity and hardened-runtime policy,
atomically replaces any prior regular file or symlink without a delete gap,
and reads the final file back before use. Desktop independently checks the
build-pinned executable-code digest, signing identity, exact capability, ACLs,
and every path component before each designated launch. A missing, stale,
writable, differently owned, or substituted install fails closed; ordinary
non-designated agents keep using the bundled sidecar.

The bundling script does not code-sign executables. macOS release automation
signs the assembled app afterward and verifies the app bundle, but the pinned
source-gateway program named by an owner's heartbeat policy is a separate
installed trust boundary and is not produced by this package. Production
macOS policy requires both its designated-requirement and TeamIdentifier pins;
deployment must install that signed gateway before a designated agent can run.
