#!/usr/bin/env bash
# Exercises scripts/mike49-pin-nip-am-verify.sh against fake buzz-auditor /
# block-buzz checkouts (no real cccareers/buzz-auditor access needed).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIN_SCRIPT="${SCRIPT_DIR}/mike49-pin-nip-am-verify.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/mike49-pin-test.XXXXXX")"
trap 'rm -rf "${TEST_ROOT}"' EXIT

# The pin script now runs `cargo metadata` against each fake buzz repo's
# Cargo.toml. Those fake repos live under $TMPDIR, outside this repo's own
# rust-toolchain.toml, so rustup has nothing to pick a default from unless
# told explicitly. Pin it to the same toolchain this repo already commits to.
REPO_TOPLEVEL_FOR_TEST="$(cd "${SCRIPT_DIR}/.." && pwd)"
RUSTUP_TOOLCHAIN="$(sed -nE 's/^channel = "(.*)"/\1/p' "${REPO_TOPLEVEL_FOR_TEST}/rust-toolchain.toml")"
export RUSTUP_TOOLCHAIN

git_id() {
  git -C "$1" config user.email "test@example.com"
  git -C "$1" config user.name "Test"
}

# A fake cccareers/buzz-auditor checkout with a real commit history. Its
# tools/nip-am-verify carries its own Cargo.toml so `cargo metadata` (not
# just grep/sed) can resolve it as a real package.
make_auditor_repo() {
  local dir="$1"
  mkdir -p "${dir}/tools/nip-am-verify"
  git init -q "${dir}"
  git_id "${dir}"
  cat > "${dir}/tools/nip-am-verify/Cargo.toml" <<'EOF'
[package]
name = "nip-am-verify"
version = "0.0.0"
edition = "2021"

[lib]
path = "lib.rs"
EOF
  echo "pub fn verify() {}" > "${dir}/tools/nip-am-verify/lib.rs"
  git -C "${dir}" add -A
  git -C "${dir}" commit -q -m "initial"
}

# A fake block/buzz checkout pinning to $2 via NIP_AM_VERIFY_PINNED_REV.
make_buzz_repo() {
  local dir="$1" rev="$2"
  mkdir -p "${dir}/desktop/src-tauri/src"
  git init -q "${dir}"
  git_id "${dir}"
  printf '%s' "${rev}" > "${dir}/desktop/src-tauri/NIP_AM_VERIFY_PINNED_REV"
  cat > "${dir}/desktop/src-tauri/Cargo.toml" <<'EOF'
[package]
name = "buzz-desktop"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
nip-am-verify = { path = "../../.mike49-nip-am-verify-pinned/tools/nip-am-verify" }
EOF
  echo "pub fn noop() {}" > "${dir}/desktop/src-tauri/src/lib.rs"
  git -C "${dir}" add -A
  git -C "${dir}" commit -q -m "initial"
}

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

# --- Case 1: clean pin, and it stays reproducible on a second run ---
case1="${TEST_ROOT}/case1"
auditor1="${case1}/buzz-auditor"
buzz1="${case1}/buzz"
make_auditor_repo "${auditor1}"
rev1="$(git -C "${auditor1}" rev-parse HEAD)"
make_buzz_repo "${buzz1}" "${rev1}"
pinned1="${buzz1}/.mike49-nip-am-verify-pinned"

if ! ( cd "${buzz1}" && BUZZ_AUDITOR_SRC="${auditor1}" "${PIN_SCRIPT}" ); then
  fail "case 1: clean pin should succeed"
fi
[ "$(git -C "${pinned1}" rev-parse HEAD)" = "${rev1}" ] || fail "case 1: pinned worktree not at expected rev"

if ! ( cd "${buzz1}" && BUZZ_AUDITOR_SRC="${auditor1}" "${PIN_SCRIPT}" ); then
  fail "case 1: re-running against an already-clean pin should succeed"
fi
echo "PASS: case 1 (clean pin, reproducible)"

# --- Case 2: dirty tracked input is rejected, not silently rebuilt ---
case2="${TEST_ROOT}/case2"
auditor2="${case2}/buzz-auditor"
buzz2="${case2}/buzz"
make_auditor_repo "${auditor2}"
rev2="$(git -C "${auditor2}" rev-parse HEAD)"
make_buzz_repo "${buzz2}" "${rev2}"
pinned2="${buzz2}/.mike49-nip-am-verify-pinned"
( cd "${buzz2}" && BUZZ_AUDITOR_SRC="${auditor2}" "${PIN_SCRIPT}" ) >/dev/null

echo "// tampered" >> "${pinned2}/tools/nip-am-verify/lib.rs"
if ( cd "${buzz2}" && BUZZ_AUDITOR_SRC="${auditor2}" "${PIN_SCRIPT}" ) >/dev/null 2>&1; then
  fail "case 2: dirty tracked input should be rejected"
fi
grep -q "// tampered" "${pinned2}/tools/nip-am-verify/lib.rs" || fail "case 2: script must not delete the dirty worktree"
echo "PASS: case 2 (dirty tracked input rejected, worktree preserved)"

# --- Case 3: unexpected untracked source file is rejected ---
case3="${TEST_ROOT}/case3"
auditor3="${case3}/buzz-auditor"
buzz3="${case3}/buzz"
make_auditor_repo "${auditor3}"
rev3="$(git -C "${auditor3}" rev-parse HEAD)"
make_buzz_repo "${buzz3}" "${rev3}"
pinned3="${buzz3}/.mike49-nip-am-verify-pinned"
( cd "${buzz3}" && BUZZ_AUDITOR_SRC="${auditor3}" "${PIN_SCRIPT}" ) >/dev/null

echo "stray" > "${pinned3}/tools/nip-am-verify/extra.rs"
if ( cd "${buzz3}" && BUZZ_AUDITOR_SRC="${auditor3}" "${PIN_SCRIPT}" ) >/dev/null 2>&1; then
  fail "case 3: unexpected untracked source should be rejected"
fi
[ -f "${pinned3}/tools/nip-am-verify/extra.rs" ] || fail "case 3: script must not delete the untracked file/worktree"
echo "PASS: case 3 (unexpected untracked source rejected, worktree preserved)"

# --- Case 4: wrong HEAD is rejected without being force-reset ---
case4="${TEST_ROOT}/case4"
auditor4="${case4}/buzz-auditor"
buzz4="${case4}/buzz"
make_auditor_repo "${auditor4}"
rev4a="$(git -C "${auditor4}" rev-parse HEAD)"
echo "fn two() {}" >> "${auditor4}/tools/nip-am-verify/lib.rs"
git -C "${auditor4}" add -A
git -C "${auditor4}" commit -q -m "second"
rev4b="$(git -C "${auditor4}" rev-parse HEAD)"
make_buzz_repo "${buzz4}" "${rev4a}"
pinned4="${buzz4}/.mike49-nip-am-verify-pinned"
( cd "${buzz4}" && BUZZ_AUDITOR_SRC="${auditor4}" "${PIN_SCRIPT}" ) >/dev/null

git -C "${pinned4}" checkout -q "${rev4b}"
if ( cd "${buzz4}" && BUZZ_AUDITOR_SRC="${auditor4}" "${PIN_SCRIPT}" ) >/dev/null 2>&1; then
  fail "case 4: wrong HEAD should be rejected"
fi
[ "$(git -C "${pinned4}" rev-parse HEAD)" = "${rev4b}" ] || fail "case 4: script must not force-reset the worktree itself"
echo "PASS: case 4 (wrong HEAD rejected, worktree not force-reset)"

# --- Case 5: unrelated pre-existing destination is rejected, not adopted ---
case5="${TEST_ROOT}/case5"
auditor5="${case5}/buzz-auditor"
buzz5="${case5}/buzz"
make_auditor_repo "${auditor5}"
rev5="$(git -C "${auditor5}" rev-parse HEAD)"
make_buzz_repo "${buzz5}" "${rev5}"
pinned5="${buzz5}/.mike49-nip-am-verify-pinned"
mkdir -p "${pinned5}"
echo "not ours" > "${pinned5}/keep-me.txt"

if ( cd "${buzz5}" && BUZZ_AUDITOR_SRC="${auditor5}" "${PIN_SCRIPT}" ) >/dev/null 2>&1; then
  fail "case 5: an unrelated existing destination should be rejected"
fi
[ -f "${pinned5}/keep-me.txt" ] || fail "case 5: script must not delete an unrelated existing destination"
[ ! -d "${pinned5}/.git" ] || fail "case 5: script must not have adopted the unrelated destination as a git worktree"
echo "PASS: case 5 (unrelated existing destination rejected, left untouched)"

# --- Case 6: same-parent, different common-directory must not be adopted ---
# Two independent repos whose .git storage sits as SIBLINGS under one shared
# parent (a worktree pool / CI checkout-cache layout), with identical
# content, author, and committer metadata so they land on the SAME commit
# hash. A common-dir check that compares only parent directories (instead of
# the full canonical common-dir path) would wrongly treat a worktree of the
# WRONG repo as belonging to the real auditor, since both the parent
# directory and the HEAD rev match.
case6="${TEST_ROOT}/case6"
mkdir -p "${case6}/gitstore"
auditor6_work="${case6}/buzz-auditor"
wrong6_work="${case6}/wrong-repo"
mkdir -p "${auditor6_work}/tools/nip-am-verify" "${wrong6_work}/tools/nip-am-verify"
git init -q --separate-git-dir="${case6}/gitstore/auditor.git" "${auditor6_work}"
git init -q --separate-git-dir="${case6}/gitstore/wrong.git" "${wrong6_work}"
git_id "${auditor6_work}"
git_id "${wrong6_work}"
cat > "${auditor6_work}/tools/nip-am-verify/Cargo.toml" <<'EOF'
[package]
name = "nip-am-verify"
version = "0.0.0"
edition = "2021"

[lib]
path = "lib.rs"
EOF
cp "${auditor6_work}/tools/nip-am-verify/Cargo.toml" "${wrong6_work}/tools/nip-am-verify/Cargo.toml"
echo "pub fn verify() {}" > "${auditor6_work}/tools/nip-am-verify/lib.rs"
echo "pub fn verify() {}" > "${wrong6_work}/tools/nip-am-verify/lib.rs"
(
  export GIT_AUTHOR_NAME="Test" GIT_AUTHOR_EMAIL="test@example.com"
  export GIT_COMMITTER_NAME="Test" GIT_COMMITTER_EMAIL="test@example.com"
  export GIT_AUTHOR_DATE="2020-01-01T00:00:00" GIT_COMMITTER_DATE="2020-01-01T00:00:00"
  git -C "${auditor6_work}" add -A
  git -C "${auditor6_work}" commit -q -m "initial"
  git -C "${wrong6_work}" add -A
  git -C "${wrong6_work}" commit -q -m "initial"
)
rev6="$(git -C "${auditor6_work}" rev-parse HEAD)"
[ "$(git -C "${wrong6_work}" rev-parse HEAD)" = "${rev6}" ] || fail "case 6 setup: expected identical commit hash across independent repos"

buzz6="${case6}/buzz"
make_buzz_repo "${buzz6}" "${rev6}"
pinned6="${buzz6}/.mike49-nip-am-verify-pinned"

# Materialize PINNED_DIR as a worktree of the WRONG repo: same rev, and its
# common-dir shares gitstore/ as a parent with the real auditor's common-dir,
# but the two common-dirs are not the same directory.
git -C "${wrong6_work}" worktree add --detach "${pinned6}" "${rev6}" >/dev/null

if ( cd "${buzz6}" && BUZZ_AUDITOR_SRC="${auditor6_work}" "${PIN_SCRIPT}" ) >/dev/null 2>&1; then
  fail "case 6: pinned worktree from a different repo's common-dir (same parent, same rev) must be rejected"
fi
echo "PASS: case 6 (same-parent different common-directory rejected)"

echo "PASS: all mike49-pin-nip-am-verify.sh cases"
