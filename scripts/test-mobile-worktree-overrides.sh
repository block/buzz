#!/usr/bin/env bash
# Regression tests for the mobile worktree identity contract:
#   - scripts/mobile-worktree-overrides.sh writes debug-only override files in
#     a worktree and removes them in the main checkout.
#   - install identity is keyed to the worktree DIRECTORY (stable across
#     branch switches); the branch (or short SHA when detached) is a
#     display-only label sanitized to [A-Za-z0-9._-].
#   - the tracked iOS/Android build files keep production identity, only
#     consume the overrides in debug configurations, and let a developer's
#     AppOverrides.xcconfig take precedence over the worktree defaults.
#   - scripts/mobile-worktree-clean.sh only ever targets suffixed installs,
#     never the production app ids.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/mobile-worktree-overrides.sh"
clean_script="$repo_root/scripts/mobile-worktree-clean.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0
fail() {
  printf 'FAIL: %s\n' "$1" >&2
  failures=$((failures + 1))
}
pass() {
  printf 'ok: %s\n' "$1"
}

make_repo() {
  # $1: repo dir, $2: initial branch name
  local repo="$1" branch="$2"
  mkdir -p "$repo/scripts" "$repo/mobile/ios/Flutter" "$repo/mobile/android"
  cp "$script" "$repo/scripts/mobile-worktree-overrides.sh"
  git -C "$repo" init -q -b "$branch"
  git -C "$repo" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
}

make_worktree() {
  # $1: source repo, $2: worktree dir, $3: branch to create
  local repo="$1" wt="$2" branch="$3"
  git -C "$repo" worktree add -q -b "$branch" "$wt"
  mkdir -p "$wt/scripts" "$wt/mobile/ios/Flutter" "$wt/mobile/android"
  cp "$script" "$wt/scripts/mobile-worktree-overrides.sh"
}

# ── Main checkout: no overrides, stale files removed ─────────────────────────
repo="$tmp/main-checkout"
make_repo "$repo" main
echo stale > "$repo/mobile/ios/Flutter/WorktreeOverrides.xcconfig"
echo stale > "$repo/mobile/android/worktree.properties"
"$repo/scripts/mobile-worktree-overrides.sh" > /dev/null
if [[ -e "$repo/mobile/ios/Flutter/WorktreeOverrides.xcconfig" || -e "$repo/mobile/android/worktree.properties" ]]; then
  fail "main checkout must remove stale worktree override files"
else
  pass "main checkout removes stale worktree override files"
fi

# ── Worktree: identity from DIRECTORY name, label from branch ────────────────
wt="$tmp/Feature_Work-1"
make_worktree "$repo" "$wt" "tho/Fix_Thing-2"
out="$("$wt/scripts/mobile-worktree-overrides.sh")"
ios="$wt/mobile/ios/Flutter/WorktreeOverrides.xcconfig"
android="$wt/mobile/android/worktree.properties"
[[ -f "$ios" && -f "$android" ]] || fail "worktree must write both override files"
grep -q '^BUNDLE_IDENTIFIER = com\.buzz\.buzzMobile\.feature-work-1$' "$ios" \
  && pass "iOS bundle identifier keys to the sanitized worktree directory name" \
  || fail "iOS bundle identifier must key to the worktree dir, got: $(cat "$ios")"
grep -q '^APP_DISPLAY_NAME = Buzz (Fix_Thing-2)$' "$ios" \
  && pass "iOS display name carries the branch label" \
  || fail "iOS display name wrong: $(cat "$ios")"
grep -q '^label=Fix_Thing-2$' "$android" \
  && pass "Android label carries the branch label" \
  || fail "Android label wrong: $(cat "$android")"
grep -q '^applicationIdSuffix=\.feature_work_1$' "$android" \
  && pass "Android applicationIdSuffix keys to the worktree directory name" \
  || fail "Android applicationIdSuffix wrong: $(cat "$android")"
printf '%s' "$out" | grep -q 'Worktree Feature_Work-1' \
  && pass "worktree run reports the worktree name" \
  || fail "worktree run must report the worktree name, got: $out"

# ── Branch switch in the same worktree: identity stable, label follows ───────
git -C "$wt" checkout -q -b "another/branch-name"
"$wt/scripts/mobile-worktree-overrides.sh" > /dev/null
grep -q '^BUNDLE_IDENTIFIER = com\.buzz\.buzzMobile\.feature-work-1$' "$ios" \
  && grep -q '^applicationIdSuffix=\.feature_work_1$' "$android" \
  && pass "branch switch keeps the install identity stable (per worktree)" \
  || fail "install identity must not change on branch switch"
grep -q '^label=branch-name$' "$android" \
  && pass "branch switch updates the display label" \
  || fail "display label must follow the branch, got: $(cat "$android")"

# ── Apostrophes / exotic-but-valid refs: label is sanitized ──────────────────
git -C "$wt" checkout -q -b "it's-\$a\"branch"
"$wt/scripts/mobile-worktree-overrides.sh" > /dev/null
grep -q "^label=it-s-a-branch$" "$android" \
  && pass "apostrophes and shell metacharacters are sanitized out of the label" \
  || fail "label must sanitize special chars, got: $(cat "$android")"
grep -Eq "^APP_DISPLAY_NAME = Buzz \([A-Za-z0-9._-]+\)$" "$ios" \
  && pass "iOS display name only contains resource-safe characters" \
  || fail "iOS display name has unsafe characters: $(cat "$ios")"

# ── Detached HEAD: label falls back to the short SHA ──────────────────────────
sha="$(git -C "$wt" rev-parse --short HEAD)"
git -C "$wt" checkout -q --detach
"$wt/scripts/mobile-worktree-overrides.sh" > /dev/null
grep -q "^label=${sha}$" "$android" \
  && pass "detached HEAD labels with the short SHA instead of literal HEAD" \
  || fail "detached HEAD must use short SHA, got: $(cat "$android")"
grep -q '^applicationIdSuffix=\.feature_work_1$' "$android" \
  && pass "detached HEAD keeps the per-worktree install identity" \
  || fail "detached HEAD must not change the install identity"

# ── Digit-leading worktree dir gets a letter-prefixed Android segment ────────
wt2="$tmp/2fast"
make_worktree "$repo" "$wt2" "some-branch"
"$wt2/scripts/mobile-worktree-overrides.sh" > /dev/null
grep -q '^applicationIdSuffix=\.w_2fast$' "$wt2/mobile/android/worktree.properties" \
  && pass "digit-leading worktree dir yields a valid Android package segment" \
  || fail "digit-leading dir segment wrong: $(cat "$wt2/mobile/android/worktree.properties")"

# ── Tracked build files: overrides are debug-only, release stays production ──
debug_xcconfig="$repo_root/mobile/ios/Flutter/Debug.xcconfig"
release_xcconfig="$repo_root/mobile/ios/Flutter/Release.xcconfig"
gradle="$repo_root/mobile/android/app/build.gradle.kts"
manifest="$repo_root/mobile/android/app/src/main/AndroidManifest.xml"
plist="$repo_root/mobile/ios/Runner/Info.plist"

grep -q 'WorktreeOverrides.xcconfig' "$debug_xcconfig" \
  && pass "Debug.xcconfig includes WorktreeOverrides" \
  || fail "Debug.xcconfig must include WorktreeOverrides.xcconfig"
worktree_line=$(grep -n 'WorktreeOverrides.xcconfig' "$debug_xcconfig" | cut -d: -f1 | head -1)
app_line=$(grep -n 'AppOverrides.xcconfig' "$debug_xcconfig" | grep '#include' | cut -d: -f1 | tail -1)
if [[ -n "$worktree_line" && -n "$app_line" && "$worktree_line" -lt "$app_line" ]]; then
  pass "AppOverrides is included after WorktreeOverrides (developer overrides win)"
else
  fail "Debug.xcconfig must include AppOverrides.xcconfig after WorktreeOverrides.xcconfig"
fi
grep -q 'WorktreeOverrides' "$release_xcconfig" \
  && fail "Release.xcconfig must not include WorktreeOverrides.xcconfig" \
  || pass "Release.xcconfig does not include WorktreeOverrides"
grep -q '^BUNDLE_IDENTIFIER = com\.buzz\.buzzMobile$' "$release_xcconfig" \
  && pass "Release.xcconfig keeps the production bundle identifier" \
  || fail "Release.xcconfig must keep BUNDLE_IDENTIFIER = com.buzz.buzzMobile"
grep -q '^APP_DISPLAY_NAME = Buzz$' "$release_xcconfig" \
  && pass "Release.xcconfig keeps the production display name" \
  || fail "Release.xcconfig must keep APP_DISPLAY_NAME = Buzz"
grep -q '<string>$(APP_DISPLAY_NAME)</string>' "$plist" \
  && pass "Info.plist display name resolves from build settings" \
  || fail "Info.plist CFBundleDisplayName must be \$(APP_DISPLAY_NAME)"
grep -q 'android:label="@string/app_name"' "$manifest" \
  && pass "Android manifest label resolves from resources" \
  || fail "Android manifest label must be @string/app_name"
grep -q 'resValue("string", "app_name", "Buzz")' "$gradle" \
  && pass "Gradle default app_name stays Buzz" \
  || fail "Gradle must declare the default app_name resValue"
grep -q 'worktreeLabel.matches' "$gradle" \
  && pass "Gradle validates the worktree label before use" \
  || fail "Gradle must validate the worktree label against a safe pattern"

# Extract a brace-balanced block: everything from the first line matching $2
# to the line where its braces close. Unlike a /start/,/}/ awk range, nested
# blocks cannot end the scan early.
extract_block() {
  # $1: file (or - for stdin), $2: start regex
  awk -v start="$2" '
    !in_block && $0 ~ start { in_block = 1 }
    in_block {
      print
      depth += gsub(/\{/, "{") - gsub(/\}/, "}")
      if (depth <= 0) exit
    }
  ' "$1"
}

# Self-test: the extractor must see past a nested block — this is exactly the
# hole the old /release \{/,/\}/ range had.
sneaky=$'buildTypes {\n  release {\n    if (nested) {\n      x = 1\n    }\n    worktreeSneakyReference()\n  }\n}'
printf '%s\n' "$sneaky" | extract_block - 'release \{' | grep -q 'worktreeSneakyReference' \
  && pass "release-block extractor scans past nested braces" \
  || fail "release-block extractor must not stop at the first nested close brace"

# The worktree suffix/label must only appear inside the debug build type.
extract_block "$gradle" 'buildTypes \{' | extract_block - 'release \{' | grep -q 'worktree' \
  && fail "release build type must not reference worktree identity" \
  || pass "release build type does not reference worktree identity"

git -C "$repo_root" check-ignore -q mobile/ios/Flutter/WorktreeOverrides.xcconfig \
  && pass "iOS override file is gitignored" \
  || fail "mobile/ios/Flutter/WorktreeOverrides.xcconfig must be gitignored"
git -C "$repo_root" check-ignore -q mobile/android/worktree.properties \
  && pass "Android override file is gitignored" \
  || fail "mobile/android/worktree.properties must be gitignored"
grep -Eq '^\s+\./scripts/mobile-worktree-overrides\.sh$' "$repo_root/Justfile" \
  && pass "just mobile-dev applies the worktree identity" \
  || fail "Justfile mobile-dev must run scripts/mobile-worktree-overrides.sh"
grep -Eq '^\s+\./scripts/mobile-worktree-clean\.sh$' "$repo_root/Justfile" \
  && pass "just mobile-clean is wired to the cleanup script" \
  || fail "Justfile mobile-clean must run scripts/mobile-worktree-clean.sh"

# ── Cleanup safety: suffixed installs matched, production ids preserved ──────
stub_bin="$tmp/stub-bin"
mkdir -p "$stub_bin"
cat > "$stub_bin/adb" <<'STUB'
#!/usr/bin/env bash
case "$1 $2" in
  "devices ") printf 'List of devices attached\nemulator-5554\tdevice\n' ;;
esac
if [[ "$1" == "devices" ]]; then exit 0; fi
if [[ "$3 $4 $5" == "shell pm list" ]]; then
  printf 'package:xyz.block.buzz.mobile\n'
  printf 'package:xyz.block.buzz.mobile.feature_work_1\n'
  printf 'package:xyz.block.buzz.mobile.w_2fast\n'
  printf 'package:com.android.settings\n'
  exit 0
fi
if [[ "$3" == "uninstall" ]]; then echo Success; exit 0; fi
exit 0
STUB
chmod +x "$stub_bin/adb"
# No xcrun stub: the iOS pass is skipped when xcrun is absent, which also
# keeps this test honest on Linux CI.
clean_out="$(PATH="$stub_bin:/usr/bin:/bin" bash "$clean_script" --dry-run)"
printf '%s\n' "$clean_out" | grep -q 'xyz\.block\.buzz\.mobile\.feature_work_1' \
  && pass "cleanup targets worktree-suffixed Android installs" \
  || fail "cleanup must list suffixed installs, got: $clean_out"
printf '%s\n' "$clean_out" | grep -q 'xyz\.block\.buzz\.mobile\.w_2fast' \
  && pass "cleanup targets letter-prefixed suffixed installs" \
  || fail "cleanup must list w_-prefixed installs, got: $clean_out"
printf '%s\n' "$clean_out" | grep -q 'mobile\.feature_work_1' || true
if printf '%s\n' "$clean_out" | grep -Eq '(would uninstall|uninstalling).*xyz\.block\.buzz\.mobile$'; then
  fail "cleanup must never target the production Android app id"
else
  pass "cleanup preserves the production Android app id"
fi
if printf '%s\n' "$clean_out" | grep -q 'com\.android\.settings'; then
  fail "cleanup must never target unrelated packages"
else
  pass "cleanup ignores unrelated packages"
fi
printf '%s\n' "$clean_out" | grep -q 'dry run:' \
  && pass "cleanup --dry-run reports without uninstalling" \
  || fail "cleanup --dry-run must report a dry-run summary, got: $clean_out"

if [[ "$failures" -gt 0 ]]; then
  printf '%d failure(s)\n' "$failures" >&2
  exit 1
fi
printf 'all mobile worktree identity contract checks passed\n'
