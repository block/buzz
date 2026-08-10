#!/usr/bin/env bash
# Uninstalls stale worktree-suffixed Buzz debug builds from booted iOS
# simulators and connected Android devices/emulators. Production installs
# (com.buzz.buzzMobile / xyz.block.buzz.mobile, no suffix) are never touched:
# only identifiers with a worktree suffix appended after the production id
# are matched. Run `just mobile-clean` (or this script directly); pass
# --dry-run to list what would be removed without uninstalling.
set -euo pipefail

ios_prefix="com.buzz.buzzMobile."
android_prefix="xyz.block.buzz.mobile."

dry_run=0
if [[ "${1:-}" == "--dry-run" ]]; then
    dry_run=1
fi

removed=0

# ── iOS: booted simulators only ──────────────────────────────────────────────
if command -v xcrun &>/dev/null; then
    booted=$(xcrun simctl list devices booted 2>/dev/null | sed -n 's/.*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p' || true)
    for udid in $booted; do
        # `simctl listapps` emits a plist keyed by bundle id; extract keys
        # that carry the worktree suffix.
        bundles=$(xcrun simctl listapps "$udid" 2>/dev/null \
            | plutil -convert json -o - -- - 2>/dev/null \
            | python3 -c 'import json,sys; print("\n".join(k for k in json.load(sys.stdin) if k.startswith(sys.argv[1])))' "$ios_prefix" \
            || true)
        for bundle in $bundles; do
            if [[ "$dry_run" == "1" ]]; then
                echo "would uninstall (iOS $udid): $bundle"
            else
                echo "uninstalling (iOS $udid): $bundle"
                xcrun simctl uninstall "$udid" "$bundle"
            fi
            removed=$((removed + 1))
        done
    done
fi

# ── Android: all connected devices/emulators ────────────────────────────────
if command -v adb &>/dev/null; then
    devices=$(adb devices 2>/dev/null | awk 'NR>1 && $2=="device" {print $1}' || true)
    for serial in $devices; do
        packages=$(adb -s "$serial" shell pm list packages 2>/dev/null \
            | tr -d '\r' | sed -n "s/^package:\(${android_prefix//./\\.}[a-z0-9_]*\)$/\1/p" || true)
        for pkg in $packages; do
            if [[ "$dry_run" == "1" ]]; then
                echo "would uninstall (Android $serial): $pkg"
            else
                echo "uninstalling (Android $serial): $pkg"
                adb -s "$serial" uninstall "$pkg" >/dev/null
            fi
            removed=$((removed + 1))
        done
    done
fi

if [[ "$removed" == "0" ]]; then
    echo "no worktree-suffixed Buzz installs found (production apps untouched)"
elif [[ "$dry_run" == "1" ]]; then
    echo "dry run: $removed worktree install(s) would be removed (production apps untouched)"
else
    echo "removed $removed worktree install(s) (production apps untouched)"
fi
