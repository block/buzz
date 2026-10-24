#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
promoter="$root/scripts/promote-oss-desktop-release.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin"

cat > "$tmp/bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1 $2" == "release view" ]]; then
  printf '%s\n' "$MOCK_RELEASE_JSON"
elif [[ "$1" == api ]]; then
  [[ "$2" == *commits/desktop-v* ]] && printf '%s\n' "${MOCK_TAG_SHA:-abc123}" || printf '%s\n' "${MOCK_TARGET_SHA:-abc123}"
elif [[ "$1 $2" == "release download" ]]; then
  tag="$3"; shift 3; pattern= dir=
  while [[ $# -gt 0 ]]; do
    case "$1" in --pattern) pattern="$2"; shift 2;; --dir) dir="$2"; shift 2;; *) shift;; esac
  done
  if [[ "$tag" == desktop-v* ]]; then
    cp "$MOCK_CANDIDATE" "$dir/$pattern"
  else
    count=0; [[ -f "$MOCK_DOWNLOAD_COUNT" ]] && count="$(cat "$MOCK_DOWNLOAD_COUNT")"
    count=$((count + 1)); printf '%s' "$count" > "$MOCK_DOWNLOAD_COUNT"
    source="$MOCK_CURRENT"
    if [[ "$count" -eq 2 && -n "${MOCK_CURRENT_SECOND:-}" ]]; then
      source="$MOCK_CURRENT_SECOND"
    elif [[ "$count" -gt 2 && -n "${MOCK_POST_WRITE:-}" ]]; then
      source="$MOCK_POST_WRITE"
    elif [[ "$count" -gt 2 ]]; then
      source="$MOCK_CANDIDATE"
    fi
    cp "$source" "$dir/$pattern"
  fi
elif [[ "$1 $2" == "release upload" ]]; then
  [[ "${MOCK_UPLOAD_FAIL:-false}" != true ]] || exit 1
  : > "$MOCK_UPLOAD_MARKER"
else
  echo "unexpected gh invocation: $*" >&2; exit 70
fi
MOCK
chmod +x "$tmp/bin/gh"

write_manifest() {
  local file="$1" version="$2" signature="${3-signed}" base_version="${4-$2}"
  jq -n --arg version "$version" --arg signature "$signature" --arg base "https://github.com/block/buzz/releases/download/desktop-v${base_version}" '{
    version: $version, notes: ("Buzz v" + $version), pub_date: "2026-08-09T00:00:00Z",
    platforms: {
      "darwin-aarch64": {signature: $signature, url: ($base + "/mac-arm.tar.gz")},
      "darwin-x86_64": {signature: $signature, url: ($base + "/mac-x64.tar.gz")},
      "linux-x86_64": {signature: $signature, url: ($base + "/linux.AppImage")},
      "windows-x86_64": {signature: $signature, url: ($base + "/windows.exe")}
    }
  }' > "$file"
}

all_assets='["updater-manifest.json","mac-arm.tar.gz","mac-x64.tar.gz","linux.AppImage","windows.exe"]'
release_json() {
  local draft="${1:-false}" prerelease="${2:-false}" asset_json="${3:-$all_assets}"
  jq -cn --argjson draft "$draft" --argjson prerelease "$prerelease" --argjson assets "$asset_json" \
    '{isDraft:$draft,isPrerelease:$prerelease,targetCommitish:"abc123",assets:[$assets[]|{name:.}]}'
}

# run_case name expected-error-or-pass expected-upload candidate current release
#          [second-current] [post-write] [upload-fail] [tag-sha] [target-sha] [version]
run_case() {
  local name="$1" expected="$2" upload="$3" candidate="$4" current="$5" release="$6"
  local second="${7:-}" post="${8:-}" upload_fail="${9:-false}" tag_sha="${10:-abc123}" target_sha="${11:-abc123}" version="${12:-1.2.3}"
  local case_dir="$tmp/$name" output status
  mkdir -p "$case_dir"; : > "$case_dir/count"; rm -f "$case_dir/uploaded"
  set +e
  output="$(PATH="$tmp/bin:$PATH" GITHUB_REPOSITORY=block/buzz \
    MOCK_RELEASE_JSON="$release" MOCK_CANDIDATE="$candidate" MOCK_CURRENT="$current" \
    MOCK_CURRENT_SECOND="$second" MOCK_POST_WRITE="$post" MOCK_UPLOAD_FAIL="$upload_fail" \
    MOCK_TAG_SHA="$tag_sha" MOCK_TARGET_SHA="$target_sha" MOCK_DOWNLOAD_COUNT="$case_dir/count" \
    MOCK_UPLOAD_MARKER="$case_dir/uploaded" GITHUB_STEP_SUMMARY="$case_dir/summary" \
    "$promoter" "$version" 2>&1)"
  status=$?
  set -e
  if [[ "$expected" == pass ]]; then
    [[ "$status" -eq 0 ]] || { echo "$name expected success: $output" >&2; exit 1; }
  else
    [[ "$status" -ne 0 ]] || { echo "$name expected failure" >&2; exit 1; }
    grep -Fq "$expected" <<<"$output" || { echo "$name missing error '$expected': $output" >&2; exit 1; }
  fi
  if [[ "$upload" == yes ]]; then
    [[ -f "$case_dir/uploaded" ]] || { echo "$name expected upload" >&2; exit 1; }
  else
    [[ ! -f "$case_dir/uploaded" ]] || { echo "$name unexpectedly uploaded" >&2; exit 1; }
  fi
}

write_manifest "$tmp/candidate.json" 1.2.3
write_manifest "$tmp/current.json" 1.2.2
write_manifest "$tmp/newer.json" 1.2.4
write_manifest "$tmp/raced.json" 1.2.2 changed
write_manifest "$tmp/same-different.json" 1.2.3 changed

run_case upgrade pass yes "$tmp/candidate.json" "$tmp/current.json" "$(release_json)"
run_case identical-retry pass no "$tmp/candidate.json" "$tmp/candidate.json" "$(release_json)"
run_case same-version-mismatch 'already promoted with different manifest content' no "$tmp/candidate.json" "$tmp/same-different.json" "$(release_json)"
run_case downgrade 'refusing downgrade' no "$tmp/candidate.json" "$tmp/newer.json" "$(release_json)"
run_case stale-manifest 'current promotion changed during validation' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json)" "$tmp/raced.json"
run_case draft 'is still a draft' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json true false)"
run_case prerelease 'is a prerelease' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json false true)"
run_case target-mismatch 'do not resolve to the same commit' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json)" '' '' false abc123 different
run_case malformed-input 'version must be stable semver' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json)" '' '' false abc123 abc123 '1.2.3";echo owned'

printf '{not-json' > "$tmp/malformed.json"
run_case malformed-json 'failed version, platform, signature, or URL validation' no "$tmp/malformed.json" "$tmp/current.json" "$(release_json)"
write_manifest "$tmp/wrong-version.json" 1.2.4
run_case wrong-version 'failed version, platform, signature, or URL validation' no "$tmp/wrong-version.json" "$tmp/current.json" "$(release_json)"
write_manifest "$tmp/empty-signature.json" 1.2.3 ''
run_case empty-signature 'failed version, platform, signature, or URL validation' no "$tmp/empty-signature.json" "$tmp/current.json" "$(release_json)"
write_manifest "$tmp/foreign-url.json" 1.2.3 signed 9.9.9
run_case foreign-url 'failed version, platform, signature, or URL validation' no "$tmp/foreign-url.json" "$tmp/current.json" "$(release_json)"
jq 'del(.platforms."windows-x86_64")' "$tmp/candidate.json" > "$tmp/missing-platform.json"
run_case missing-platform 'failed version, platform, signature, or URL validation' no "$tmp/missing-platform.json" "$tmp/current.json" "$(release_json)"
jq '.platforms["freebsd-x86_64"] = .platforms["linux-x86_64"]' "$tmp/candidate.json" > "$tmp/extra-platform.json"
run_case extra-platform 'failed version, platform, signature, or URL validation' no "$tmp/extra-platform.json" "$tmp/current.json" "$(release_json)"
missing_assets='["updater-manifest.json","mac-arm.tar.gz","mac-x64.tar.gz","linux.AppImage"]'
run_case missing-asset 'references missing release asset' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json false false "$missing_assets")"
run_case upload-failure 'promotion upload failed' no "$tmp/candidate.json" "$tmp/current.json" "$(release_json)" '' '' true
run_case post-write-mismatch 'served latest.json does not match the promoted candidate' yes "$tmp/candidate.json" "$tmp/current.json" "$(release_json)" '' "$tmp/raced.json"

echo "OSS desktop promotion behavior passed"
