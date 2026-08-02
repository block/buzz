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
grep -q '^BUNDLE_IDENTIFIER = xyz\.block\.buzz\.dogfood\.mobile\.feature-work-1$' "$ios" \
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
grep -q '^BUNDLE_IDENTIFIER = xyz\.block\.buzz\.dogfood\.mobile\.feature-work-1$' "$ios" \
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
pbxproj="$repo_root/mobile/ios/Runner.xcodeproj/project.pbxproj"
runner_entitlements="$repo_root/mobile/ios/Runner/Runner.entitlements"
gradle="$repo_root/mobile/android/app/build.gradle.kts"
manifest="$repo_root/mobile/android/app/src/main/AndroidManifest.xml"
plist="$repo_root/mobile/ios/Runner/Info.plist"

grep -q '^BUNDLE_IDENTIFIER = xyz\.block\.buzz\.dogfood\.mobile$' "$debug_xcconfig" \
  && pass "Debug.xcconfig defaults to the dogfood bundle identifier" \
  || fail "Debug.xcconfig must default to xyz.block.buzz.dogfood.mobile"
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
grep -q '^ios_prefix="xyz.block.buzz.dogfood.mobile\."$' "$clean_script" \
  && pass "cleanup targets the iOS dogfood worktree prefix" \
  || fail "cleanup must share the iOS dogfood prefix used by worktree overrides"

grep -q 'WorktreeOverrides' "$release_xcconfig" \
  && fail "Release.xcconfig must not include WorktreeOverrides.xcconfig" \
  || pass "Release.xcconfig does not include WorktreeOverrides"
grep -q '^BUNDLE_IDENTIFIER = xyz\.block\.buzz\.mobile$' "$release_xcconfig" \
  && pass "Release.xcconfig keeps the production bundle identifier" \
  || fail "Release.xcconfig must keep BUNDLE_IDENTIFIER = xyz.block.buzz.mobile"
grep -q '^APP_DISPLAY_NAME = Buzz$' "$release_xcconfig" \
  && pass "Release.xcconfig keeps the production display name" \
  || fail "Release.xcconfig must keep APP_DISPLAY_NAME = Buzz"

# These checks assert declarations in the two tracked xcconfigs only. They do
# not prove resolved build settings. The later gitignored includes
# (WorktreeOverrides.xcconfig and AppOverrides.xcconfig) can override these
# declarations and are explicitly outside this tracked-source assertion. The
# value check and declaration census are complementary: xcconfig is last-wins,
# while the census deliberately flags even a harmless duplicate declaration so
# a human reviews the changed declaration surface.
assert_xcconfig_value() {
  # $1: file, $2: anchored value regex, $3: pass/failure description
  if grep -qE "$2" "$1"; then
    pass "$3"
  else
    fail "$3"
  fi
}

assert_xcconfig_declaration_count() {
  # $1: file, $2: key, $3: expected count, $4: configuration label
  local file="$1" key="$2" expected="$3" label="$4" count
  count=$(grep -cE "^[[:space:]]*$key([[:space:]]*\[[^]]*\])*[[:space:]]*=" "$file" || true)
  if [[ "$count" -eq "$expected" ]]; then
    if [[ "$expected" -eq 0 ]]; then
      pass "$label $key has no tracked declaration sites"
    elif [[ "$expected" -eq 1 ]]; then
      pass "$label $key has one tracked declaration site"
    else
      pass "$label $key has $count tracked declaration sites"
    fi
  elif [[ "$expected" -eq 0 ]]; then
    fail "$label $key has $count tracked declaration sites; expected zero"
  elif [[ "$expected" -eq 1 ]]; then
    fail "$label $key has $count tracked declaration sites; expected exactly one"
  else
    fail "$label $key has $count tracked declaration sites; expected $expected"
  fi
}

assert_single_xcconfig_declaration() {
  # $1: file, $2: key, $3: configuration label
  assert_xcconfig_declaration_count "$1" "$2" 1 "$3"
}

assert_xcconfig_value "$debug_xcconfig" \
  '^BUZZ_IOS_PUSH_ENVIRONMENT = development$' \
  "Debug push environment is declared as development"
assert_xcconfig_value "$debug_xcconfig" \
  '^BUZZ_APP_ATTEST_ENVIRONMENT = development$' \
  "Debug App Attest environment is declared as development"
assert_xcconfig_value "$debug_xcconfig" \
  '^BUZZ_APP_GROUP_IDENTIFIER = group\.\$\(BUNDLE_IDENTIFIER\)$' \
  "Debug App Group is declared as derived from the bundle identifier"
assert_xcconfig_value "$debug_xcconfig" \
  '^BUZZ_KEYCHAIN_ACCESS_GROUP = \$\(BUNDLE_IDENTIFIER\)$' \
  "Debug Keychain access group is declared as the bundle identifier"
assert_xcconfig_value "$release_xcconfig" \
  '^BUZZ_IOS_PUSH_ENVIRONMENT = production$' \
  "Release push environment is declared as production"
assert_xcconfig_value "$release_xcconfig" \
  '^BUZZ_APP_ATTEST_ENVIRONMENT = production$' \
  "Release App Attest environment is declared as production"
assert_xcconfig_value "$release_xcconfig" \
  '^BUZZ_APP_GROUP_IDENTIFIER = group\.\$\(BUNDLE_IDENTIFIER\)$' \
  "Release App Group is declared as derived from the bundle identifier"
assert_xcconfig_value "$release_xcconfig" \
  '^BUZZ_KEYCHAIN_ACCESS_GROUP = \$\(BUNDLE_IDENTIFIER\)$' \
  "Release Keychain access group is declared as the bundle identifier"
assert_xcconfig_value "$release_xcconfig" \
  '^CODE_SIGN_STYLE = Automatic$' \
  "Release code signing style is declared as automatic"
assert_xcconfig_value "$release_xcconfig" \
  '^CODE_SIGN_IDENTITY = iPhone Developer$' \
  "Release code signing identity is declared as iPhone Developer"

for key in BUNDLE_IDENTIFIER BUZZ_KEYCHAIN_ACCESS_GROUP BUZZ_IOS_PUSH_ENVIRONMENT BUZZ_APP_ATTEST_ENVIRONMENT BUZZ_APP_GROUP_IDENTIFIER; do
  assert_single_xcconfig_declaration "$debug_xcconfig" "$key" "Debug"
  assert_single_xcconfig_declaration "$release_xcconfig" "$key" "Release"
done

for key in CODE_SIGN_STYLE CODE_SIGN_IDENTITY; do
  assert_xcconfig_declaration_count "$debug_xcconfig" "$key" 0 "Debug"
  assert_single_xcconfig_declaration "$release_xcconfig" "$key" "Release"
done

assert_xcconfig_declaration_count \
  "$debug_xcconfig" PROVISIONING_PROFILE_SPECIFIER 0 "Debug"
assert_xcconfig_declaration_count \
  "$release_xcconfig" PROVISIONING_PROFILE_SPECIFIER 0 "Release"
assert_single_xcconfig_declaration "$debug_xcconfig" APP_DISPLAY_NAME "Debug"
assert_single_xcconfig_declaration "$release_xcconfig" APP_DISPLAY_NAME "Release"

# SWIFT_ACTIVE_COMPILATION_CONDITIONS is checked separately from the closed
# identity census above. This is a tracked-source assertion only; resolved Xcode
# build settings are intentionally outside this Linux-compatible test.
assert_xcconfig_value "$debug_xcconfig" \
  '^SWIFT_ACTIVE_COMPILATION_CONDITIONS = \$\(inherited\) DEBUG$' \
  "Debug Swift compilation conditions inherit DEBUG"

debug_swift_condition_count=$(grep -cE \
  '^[[:space:]]*SWIFT_ACTIVE_COMPILATION_CONDITIONS([[:space:]]*\[[^]]*\])*[[:space:]]*=' \
  "$debug_xcconfig" || true)
if [[ "$debug_swift_condition_count" -eq 1 ]]; then
  pass "Debug SWIFT_ACTIVE_COMPILATION_CONDITIONS has one tracked declaration site"
else
  fail "Debug SWIFT_ACTIVE_COMPILATION_CONDITIONS has $debug_swift_condition_count tracked declaration sites; expected exactly one"
fi

release_swift_condition_count=$(grep -cE \
  '^[[:space:]]*SWIFT_ACTIVE_COMPILATION_CONDITIONS([[:space:]]*\[[^]]*\])*[[:space:]]*=' \
  "$release_xcconfig" || true)
if [[ "$release_swift_condition_count" -eq 0 ]]; then
  pass "Release SWIFT_ACTIVE_COMPILATION_CONDITIONS has no tracked declaration sites"
else
  fail "Release SWIFT_ACTIVE_COMPILATION_CONDITIONS has $release_swift_condition_count tracked declaration sites; expected zero"
fi

# Split the retired identifiers so the regression test does not match itself.
retired_bundle_id='com.buzz.buzz'"Mobile"
if git -C "$repo_root" grep -q -F "$retired_bundle_id"; then
  fail "tracked files must not retain the retired iOS bundle identifier"
else
  pass "tracked files do not retain the retired iOS bundle identifier"
fi
grep -q '<key>com.apple.developer.devicecheck.appattest-environment</key>' "$runner_entitlements" \
  && pass "Runner uses the App Attest entitlement key accepted by Apple" \
  || fail "Runner must use com.apple.developer.devicecheck.appattest-environment"
retired_entitlement_key='com.apple.developer.app-attest.'"environment"
if grep -q "$retired_entitlement_key" "$runner_entitlements"; then
  fail "Runner must not retain the invalid App Attest entitlement key"
else
  pass "Runner omits the invalid App Attest entitlement key"
fi

duplicate_pbx_object_ids=$(awk '
  # This bounded source-level smoke check recognizes the current two-tab
  # object-key spellings. It is not a general OpenStep uniqueness check:
  # measured exclusions include a comment before the key, a presentation
  # comment spanning lines, and one-tab indentation (jb_b1/jb_b2/jb_b6).
  # The macOS semantic check below owns their resolved build consequences.
  function decomment(s,   head, tailpart) {
    while (match(s, /\/\*/)) {
      head = substr(s, 1, RSTART - 1)
      tailpart = substr(s, RSTART + 2)
      if (!match(tailpart, /\*\//)) return head " "
      s = head " " substr(tailpart, RSTART + RLENGTH)
    }
    return s
  }

  /^\t\t/ {
    line = decomment($0)
    if (match(line, /^\t\t"?[[:alnum:]]+"?[[:space:]]*=/)) {
      object_id = substr(line, RSTART, RLENGTH)
      sub(/^\t\t"?/, "", object_id)
      sub(/"?[[:space:]]*=$/, "", object_id)
      if (++object_id_count[object_id] == 2) print object_id
    }
  }
' "$pbxproj" | sort)
if [[ -n "$duplicate_pbx_object_ids" ]]; then
  fail "recognized iOS project object identifiers repeat: $(printf '%s\n' "$duplicate_pbx_object_ids" | paste -sd ' ' -)"
else
  pass "recognized iOS project object identifiers do not repeat"
fi

signing_map=$(awk '
  # PBX comments are separators, not text: strip them before parsing any
  # object so a comment cannot hide a duplicate key from the ambiguity count.
  function decomment(s,   head, tailpart) {
    while (match(s, /\/\*/)) {
      head = substr(s, 1, RSTART - 1)
      tailpart = substr(s, RSTART + 2)
      if (!match(tailpart, /\*\//)) { return head " " }
      s = head " " substr(tailpart, RSTART + RLENGTH)
    }
    return s
  }

  FNR == 1 { pass++ }

  # Pass 1 indexes xcconfig paths and follows each PBXNativeTarget to its
  # actual configuration-list object. Target names come from object fields,
  # not presentation comments.
  pass == 1 {
    if (/isa[[:space:]]*=[[:space:]]*PBXFileReference/ && /\.xcconfig/) {
      declaration = decomment($0)
      if (match(declaration, /=[[:space:]]*\{[[:space:]]*isa[[:space:]]*=[[:space:]]*PBXFileReference[[:space:]]*;/)) {
        declaration = substr(declaration, RSTART + RLENGTH)
      } else {
        declaration = ""
      }
      sub(/\}.*/, "", declaration)
      xcconfig_path = "MISSING_PATH"
      rest = declaration
      path_matches = 0
      while (match(rest, /(^|;)[[:space:]]*path[[:space:]]*=[[:space:]]*[^;]+/)) {
        candidate = substr(rest, RSTART, RLENGTH)
        rest = substr(rest, RSTART + RLENGTH)
        sub(/^;?[[:space:]]*path[[:space:]]*=[[:space:]]*/, "", candidate)
        gsub(/"/, "", candidate)
        sub(/[[:space:]]+$/, "", candidate)
        path_matches++
        xcconfig_path = candidate
      }
      # More than one `path =` in one object means a decoy (a quoted value or
      # an embedded comment) is shadowing the real key. Never guess which one
      # the build uses: fail the row loudly instead.
      if (path_matches > 1) xcconfig_path = "AMBIGUOUS_PATH"
      xcconfig_paths[$1] = xcconfig_path
    }

    if (/\/\* Begin PBXNativeTarget section \*\//) {
      in_native_targets = 1
      next
    }
    if (/\/\* End PBXNativeTarget section \*\//) {
      in_native_targets = 0
      next
    }
    if (!in_native_targets) next

    if (/^\t\t[^[:space:]]+ .* = \{$/) {
      native_target_id = $1
      native_target_name = ""
      native_target_list = ""
      next
    }
    if (native_target_id != "" && /^\t\t\tname = /) {
      native_target_name = $0
      sub(/^.*= */, "", native_target_name)
      sub(/;.*/, "", native_target_name)
      gsub(/"/, "", native_target_name)
      next
    }
    if (native_target_id != "" && /^\t\t\tbuildConfigurationList = /) {
      native_target_list = $3
      next
    }
    if (native_target_id != "" && /^\t\t\};/) {
      if (native_target_list != "") {
        if (native_target_name == "") native_target_name = "UNNAMED:" native_target_id
        if (native_target_list in list_owners) {
          list_owners[native_target_list] = "DUPLICATE:" list_owners[native_target_list] "+" native_target_name
        } else {
          list_owners[native_target_list] = native_target_name
        }
      }
      native_target_id = ""
    }
    next
  }

  # Pass 2 maps build-configuration object IDs through only those lists that
  # real native targets own. PBXProject and other unowned lists are ignored.
  pass == 2 {
    if (/\/\* Begin XCConfigurationList section \*\//) {
      in_configuration_lists = 1
      next
    }
    if (/\/\* End XCConfigurationList section \*\//) {
      in_configuration_lists = 0
      next
    }
    if (!in_configuration_lists) next

    if (/^\t\t[^[:space:]]+ .* = \{$/) {
      configuration_list_id = $1
      configuration_list_owner = configuration_list_id in list_owners ? list_owners[configuration_list_id] : ""
      next
    }
    if (/buildConfigurations = \(/) {
      in_list_configurations = 1
      next
    }
    if (in_list_configurations && /\);/) {
      in_list_configurations = 0
      next
    }
    if (in_list_configurations && $1 ~ /^[[:alnum:]]+$/ && configuration_list_owner != "") {
      if ($1 in targets) targets[$1] = "DUPLICATE:" targets[$1] "+" configuration_list_owner
      else targets[$1] = configuration_list_owner
    }
    next
  }

  # Pass 3 emits one row for each team-bearing build configuration.
  !in_build_configuration && /\/\* (Debug|Release|Profile) \*\/ = \{/ {
    in_build_configuration = 1
    configuration_id = $1
    configuration = $3
    base_configuration = "NONE"
    team = ""
    entitlements = "NONE"
    depth = 0
  }

  in_build_configuration {
    if (/baseConfigurationReference =/) {
      base_configuration = $3 in xcconfig_paths ? xcconfig_paths[$3] : "UNRESOLVED:" $3
    }
    if (/DEVELOPMENT_TEAM =/) {
      team = $0
      sub(/^.*= */, "", team)
      sub(/;.*/, "", team)
    }
    if (/CODE_SIGN_ENTITLEMENTS =/) {
      entitlements = $0
      sub(/^.*= */, "", entitlements)
      sub(/;.*/, "", entitlements)
    }

    depth += gsub(/\{/, "{") - gsub(/\}/, "}")
    if (depth == 0) {
      if (team != "") {
        target_name = configuration_id in targets ? targets[configuration_id] : "UNMAPPED:" configuration_id
        print target_name, configuration, base_configuration, team, entitlements
      }
      in_build_configuration = 0
    }
  }
' "$pbxproj" "$pbxproj" "$pbxproj" | sort)
expected_signing_map=$(printf '%s\n' \
  'NotificationService Debug Flutter/Debug.xcconfig JMTDPW9CG3 NotificationService/NotificationService.entitlements' \
  'NotificationService Profile Flutter/Release.xcconfig EYF346PHUG NotificationService/NotificationService.entitlements' \
  'NotificationService Release Flutter/Release.xcconfig EYF346PHUG NotificationService/NotificationService.entitlements' \
  'Runner Debug Flutter/Debug.xcconfig JMTDPW9CG3 Runner/Runner.entitlements' \
  'Runner Profile Flutter/Release.xcconfig EYF346PHUG Runner/Runner.entitlements' \
  'Runner Release Flutter/Release.xcconfig EYF346PHUG Runner/Runner.entitlements')
if [[ "$signing_map" == "$expected_signing_map" ]]; then
  pass "Runner and NotificationService signing settings match each build configuration"
else
  fail "unexpected iOS signing map: $signing_map"
fi
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
