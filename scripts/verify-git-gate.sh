#!/usr/bin/env bash
# Verifies crates/buzz-git-gate with Verus.
#
# Cargo compiles the same function bodies with the proof annotations erased,
# so `cargo test` alone checks nothing that this script checks.
#
# Two ways this step could pass without proving anything, both refused here:
# - cargo-verus warns and exits 0 when no selected package opts in through
#   `[package.metadata.verus] verify = true`;
# - Cargo can reuse a previous verification and print no result line.
# So the opt-in is asserted, the crate's artifacts are removed first, and the
# verifier's own result line must report at least MIN_VERIFIED obligations and
# zero errors.
set -euo pipefail

cd "$(dirname "$0")/.."

# Raise this when obligations are added; lowering it needs a reason.
MIN_VERIFIED=46

opted_in=$(cargo metadata --no-deps --format-version 1 --locked |
  jq -r '.packages[] | select(.name == "buzz-git-gate") | .metadata.verus.verify')
if [ "$opted_in" != "true" ]; then
  echo "error: buzz-git-gate must set [package.metadata.verus] verify = true" >&2
  exit 1
fi

# Verus runs rustc itself; point it at the repository's toolchain libraries
# rather than letting it select one through rustup.
export VERUS_USE_RUSTUP=0
rust_lib="$(rustc --print sysroot)/lib"
export LD_LIBRARY_PATH="$rust_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export DYLD_LIBRARY_PATH="$rust_lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"

cargo clean -p buzz-git-gate --quiet
log=$(mktemp "${TMPDIR:-/tmp}/buzz-git-gate-verus.XXXXXX")
trap 'rm -f "$log"' EXIT
status=0
cargo verus verify -p buzz-git-gate --locked 2>&1 | tee "$log" || status=$?

# The last result line is the crate's; earlier ones belong to vstd.
result=$(grep -E '^verification results:: [0-9]+ verified, [0-9]+ errors' "$log" | tail -1 || true)
if [ -z "$result" ]; then
  echo "error: Verus printed no verification result for buzz-git-gate" >&2
  exit 1
fi
verified=$(sed -E 's/^verification results:: ([0-9]+) verified, ([0-9]+) errors.*/\1/' <<<"$result")
errors=$(sed -E 's/^verification results:: ([0-9]+) verified, ([0-9]+) errors.*/\2/' <<<"$result")
if [ "$status" -ne 0 ] || [ "$errors" -ne 0 ]; then
  echo "error: Verus reported $errors errors (exit status $status)" >&2
  exit 1
fi
if [ "$verified" -lt "$MIN_VERIFIED" ]; then
  echo "error: $verified obligations verified, expected at least $MIN_VERIFIED" >&2
  exit 1
fi
echo "buzz-git-gate: $verified verified, 0 errors"
