#!/usr/bin/env bash
set -euo pipefail

# Deterministic, offline tests for resolve-mesh-rev.sh. Covers success, mixed
# rev failure, missing rev/dependency failure, and absent checkout failure.

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/resolve-mesh-rev.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

die() {
  echo "FAIL: $*" >&2
  exit 1
}

# Expect the script to exit non-zero for a bad input.
expect_fail() {
  local desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    die "$desc — expected failure but the resolver succeeded"
  fi
}

REV="c441ea7f328692b18b7f49ad819c7b1a603cbdcb"
SHORT="c441ea7"

# --- success: consistent revs across several deps (+ a decoy comment) --------
cat >"$tmp/consistent.toml" <<EOF
[dependencies]
# comment mentioning Mesh-LLM/mesh-llm.git that is not a real dependency
mesh-llm-sdk = { git = "https://github.com/Mesh-LLM/mesh-llm.git", rev = "$REV", package = "mesh-llm-sdk" }
mesh-llm-node = { git = "https://github.com/Mesh-LLM/mesh-llm.git", rev = "$REV", package = "mesh-llm-node" }
serde = "1"
EOF

got="$(MESH_MANIFEST="$tmp/consistent.toml" "$script")"
[[ "$got" == "$REV" ]] || die "consistent rev: expected $REV, got $got"
got="$(MESH_MANIFEST="$tmp/consistent.toml" "$script" --short)"
[[ "$got" == "$SHORT" ]] || die "consistent short: expected $SHORT, got $got"

# --- failure: mixed revs -----------------------------------------------------
cat >"$tmp/mixed.toml" <<EOF
mesh-llm-sdk = { git = "https://github.com/Mesh-LLM/mesh-llm.git", rev = "$REV" }
mesh-llm-node = { git = "https://github.com/Mesh-LLM/mesh-llm.git", rev = "0000000000000000000000000000000000000000" }
EOF
expect_fail "mixed revs" env MESH_MANIFEST="$tmp/mixed.toml" "$script"

# --- failure: a Mesh dep with no rev ----------------------------------------
cat >"$tmp/missing-rev.toml" <<EOF
mesh-llm-sdk = { git = "https://github.com/Mesh-LLM/mesh-llm.git", package = "mesh-llm-sdk" }
EOF
expect_fail "missing rev" env MESH_MANIFEST="$tmp/missing-rev.toml" "$script"

# --- failure: no Mesh dependency at all -------------------------------------
cat >"$tmp/none.toml" <<EOF
serde = "1"
EOF
expect_fail "no dependency" env MESH_MANIFEST="$tmp/none.toml" "$script"

# --- failure: absent checkout ------------------------------------------------
expect_fail "absent checkout" \
  env MESH_MANIFEST="$tmp/consistent.toml" CARGO_HOME="$tmp/empty-cargo" "$script" --find-checkout

# --- success: --find-checkout locates an existing checkout -------------------
mkdir -p "$tmp/cargo/git/checkouts/mesh-llm-abcd1234/$SHORT"
found="$(MESH_MANIFEST="$tmp/consistent.toml" CARGO_HOME="$tmp/cargo" "$script" --find-checkout)"
[[ "$found" == *"/$SHORT" ]] || die "find-checkout: expected a path ending in /$SHORT, got $found"

echo "resolve-mesh-rev: ALL TESTS PASSED"
