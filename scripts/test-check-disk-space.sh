#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
check_disk="${repo_root}/scripts/check-disk-space.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin"
cat >"$tmp/bin/df" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' \
  'Filesystem 1024-blocks Used Available Capacity Mounted on' \
  "/dev/mock ${MOCK_DISK_TOTAL_KIB} 1 ${MOCK_DISK_AVAILABLE_KIB} 1% /"
MOCK
chmod +x "$tmp/bin/df"

# The fixture repo has no Justfile and no node scripts, but lefthook.yml is
# copied verbatim — so any pre-push command that shells out to `just` (today
# file-size-check, unguarded and unglobbed) would fail the fixture push for a
# reason this test is not about. Stub `just` to a no-op: the disk guard runs
# before `&& just ...` in every guarded job, so blocking behaviour is still
# what is being measured.
cat >"$tmp/bin/just" <<'MOCK'
#!/usr/bin/env bash
exit 0
MOCK
chmod +x "$tmp/bin/just"

run_check() {
  PATH="$tmp/bin:$PATH" \
    BUZZ_DISK_BUILD_RESERVE_GIB=15 \
    BUZZ_DISK_MIN_FREE_GIB=10 \
    "$check_disk" "$@"
}

# A 15 GiB build reserve must still leave the configured 10 GiB minimum.
MOCK_DISK_TOTAL_KIB=$((100 * 1024 * 1024)) \
MOCK_DISK_AVAILABLE_KIB=$((25 * 1024 * 1024)) \
  run_check >/dev/null

if MOCK_DISK_TOTAL_KIB=$((100 * 1024 * 1024)) \
  MOCK_DISK_AVAILABLE_KIB=$((24 * 1024 * 1024)) \
  run_check >"$tmp/low.out" 2>&1; then
  echo "disk preflight accepted insufficient post-build free space" >&2
  exit 1
fi
grep -Fq 'Disk preflight blocked' "$tmp/low.out"
grep -Fq 'minimum is 10 GiB' "$tmp/low.out"
grep -Fq 'BUZZ_SKIP_DISK_PREFLIGHT=1' "$tmp/low.out"

if PATH="$tmp/bin:$PATH" \
  MOCK_DISK_TOTAL_KIB=$((100 * 1024 * 1024)) \
  MOCK_DISK_AVAILABLE_KIB=$((40 * 1024 * 1024)) \
  BUZZ_DISK_BUILD_RESERVE_GIB=15 \
  BUZZ_DISK_MIN_FREE_GIB=invalid \
  "$check_disk" >"$tmp/invalid.out" 2>&1; then
  echo "disk preflight accepted an invalid absolute minimum" >&2
  exit 1
fi
grep -Fq 'BUZZ_DISK_MIN_FREE_GIB must be a non-negative integer' \
  "$tmp/invalid.out"

# The escape hatch bypasses only this guard, leaving the other hooks intact.
MOCK_DISK_TOTAL_KIB=$((100 * 1024 * 1024)) \
MOCK_DISK_AVAILABLE_KIB=1 \
BUZZ_SKIP_DISK_PREFLIGHT=1 \
  run_check >/dev/null

# Verify the behavior through a real Git pre-push, not only config inspection.
fixture="$tmp/hook-repo"
remote="$tmp/remote.git"
git init --bare -q "$remote"
git init -q -b main "$fixture"
git -C "$fixture" config user.name test
git -C "$fixture" config user.email test@example.com
git -C "$fixture" remote add origin "$remote"
mkdir -p "$fixture/scripts"
cp "$check_disk" "$fixture/scripts/check-disk-space.sh"
cp "$repo_root/scripts/check-branch-skew.sh" "$fixture/scripts/check-branch-skew.sh"
cp "$repo_root/lefthook.yml" "$fixture/lefthook.yml"
git -C "$fixture" add .
git -C "$fixture" -c core.hooksPath=/dev/null commit -qm baseline
git -C "$fixture" -c core.hooksPath=/dev/null push -q -u origin main
(
  # Use the Hermit stub rather than a bare `lefthook`: CI's `changes` job runs
  # this script without activating Hermit, so only the pinned repo binary is
  # guaranteed to resolve.
  cd "$fixture"
  "$repo_root/bin/lefthook" install --force >/dev/null
)

# Documentation-only pushes skip all guarded jobs.
printf '%s\n' '# docs' >"$fixture/README.md"
git -C "$fixture" add .
git -C "$fixture" -c core.hooksPath=/dev/null commit -qm docs-change
PATH="$tmp/bin:$PATH" \
MOCK_DISK_TOTAL_KIB=$((100 * 1024 * 1024)) \
MOCK_DISK_AVAILABLE_KIB=1 \
  git -C "$fixture" push -q

mkdir -p "$fixture/crates/example/src"
printf '%s\n' 'pub fn example() {}' >"$fixture/crates/example/src/lib.rs"
git -C "$fixture" add .
git -C "$fixture" -c core.hooksPath=/dev/null commit -qm heavy-change
if PATH="$tmp/bin:$PATH" \
  MOCK_DISK_TOTAL_KIB=$((100 * 1024 * 1024)) \
  MOCK_DISK_AVAILABLE_KIB=$((24 * 1024 * 1024)) \
  git -C "$fixture" push >"$tmp/hook.out" 2>&1; then
  echo "real pre-push hook bypassed the disk preflight" >&2
  exit 1
fi
if ! grep -Fq 'Disk preflight blocked' "$tmp/hook.out"; then
  echo "real pre-push failed for an unexpected reason:" >&2
  cat "$tmp/hook.out" >&2
  exit 1
fi

# Unsupported df output fails open so the hook remains portable.
cat >"$tmp/bin/df" <<'MOCK'
#!/usr/bin/env bash
echo unsupported
MOCK
chmod +x "$tmp/bin/df"
run_check >"$tmp/unsupported.out" 2>&1
grep -Fq 'could not determine free space' "$tmp/unsupported.out"

# One guarded pre-push job each for rust-tests, desktop-check, desktop-typecheck,
# desktop-test, desktop-tauri-checks, and mobile-test. Bump when a build-heavy
# job is added to lefthook.yml.
[[ "$(grep -Fc 'run: ./scripts/check-disk-space.sh &&' "$repo_root/lefthook.yml")" -eq 6 ]]
if grep -Fq 'setup:' "$repo_root/lefthook.yml"; then
  echo "disk preflight must not use Lefthook setup on pinned version 2.1.3" >&2
  exit 1
fi
grep -Fq 'scripts/test-check-disk-space.sh' \
  "$repo_root/.github/workflows/ci.yml"

echo "disk-space preflight tests passed"
