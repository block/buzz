#!/usr/bin/env bash
# Build a drag-to-Applications DMG without requiring a GUI login session.
# Finder styling is optional; the disk image itself is always authoritative.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <Buzz.app> <output.dmg>" >&2
  exit 2
fi

app_path="$1"
out_dmg="$2"
app_name="$(basename "$app_path")"
volume_name="${VOL_NAME:-Buzz}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
background="$script_dir/../src-tauri/icons/dmg-background.png"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/buzz-dmg.XXXXXX")"
source_dir="$work_dir/source"
rw_dmg="$work_dir/read-write.dmg"
mount_point="$work_dir/mount"
applescript="$work_dir/style.applescript"
device=""

finish() {
  local status="$?"
  trap - EXIT
  if [[ -n "$device" ]]; then
    hdiutil detach "$device" >/dev/null 2>&1 || true
    hdiutil detach -force "$device" >/dev/null 2>&1 || true
  fi
  rm -rf "$work_dir"
  exit "$status"
}
trap finish EXIT

[[ -d "$app_path" ]] || { echo "App bundle not found: $app_path" >&2; exit 1; }
[[ -f "$background" ]] || { echo "DMG background not found: $background" >&2; exit 1; }

mkdir -p "$(dirname "$out_dmg")" "$source_dir/.background" "$mount_point"
ditto "$app_path" "$source_dir/$app_name"
ln -s /Applications "$source_dir/Applications"
cp "$background" "$source_dir/.background/background.png"

rm -f "$rw_dmg" "$out_dmg"
hdiutil create -volname "$volume_name" -srcfolder "$source_dir" \
  -format UDRW -ov "$rw_dmg" >/dev/null

attach_output="$(hdiutil attach -readwrite -noverify -noautoopen -nobrowse \
  -mountpoint "$mount_point" "$rw_dmg")"
device="$(printf '%s\n' "$attach_output" | awk '/^\/dev\// { print $1; exit }')"
[[ -n "$device" ]] || { echo "Failed to attach writable DMG" >&2; exit 1; }

detach() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if hdiutil detach "$device" >/dev/null 2>&1; then
      device=""
      return 0
    fi
    sleep 1
  done
  hdiutil detach -force "$device" >/dev/null
  device=""
}

if command -v SetFile >/dev/null 2>&1; then
  SetFile -a V "$mount_point/.background" || true
  icon="$mount_point/$app_name/Contents/Resources/icon.icns"
  if [[ -f "$icon" ]]; then
    cp "$icon" "$mount_point/.VolumeIcon.icns" || true
    SetFile -c icnC "$mount_point/.VolumeIcon.icns" || true
    SetFile -a C "$mount_point" || true
  fi
fi

cat >"$applescript" <<'APPLESCRIPT'
on run argv
  set mountPath to item 1 of argv
  set appName to item 2 of argv
  tell application "Finder"
    set rootFolder to POSIX file mountPath as alias
    open rootFolder
    set imageWindow to container window of rootFolder
    set current view of imageWindow to icon view
    set toolbar visible of imageWindow to false
    set statusbar visible of imageWindow to false
    set bounds of imageWindow to {200, 120, 860, 652}
    set viewOptions to icon view options of imageWindow
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to 128
    set text size of viewOptions to 14
    set background picture of viewOptions to file ".background:background.png" of rootFolder
    set position of item appName of rootFolder to {191, 330}
    set position of item "Applications" of rootFolder to {469, 330}
    set extension hidden of item appName of rootFolder to true
    delay 1
    close imageWindow
  end tell
end run
APPLESCRIPT

style_with_finder() {
  local child elapsed=0
  /usr/bin/osascript "$applescript" "$mount_point" "$app_name" &
  child=$!
  while kill -0 "$child" 2>/dev/null; do
    if (( elapsed >= 100 )); then
      echo "Finder styling timed out; continuing without it" >&2
      kill "$child" 2>/dev/null || true
      wait "$child" 2>/dev/null || true
      return 124
    fi
    sleep 0.1
    elapsed=$((elapsed + 1))
  done
  wait "$child"
}

if ! style_with_finder; then
  echo "Finder styling unavailable; continuing without it" >&2
fi

sync
detach
hdiutil convert "$rw_dmg" -format UDZO -imagekey zlib-level=9 \
  -o "$out_dmg" >/dev/null
printf 'DMG ready: %s\n' "$out_dmg"
