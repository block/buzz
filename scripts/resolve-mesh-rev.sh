#!/usr/bin/env bash
set -euo pipefail

# Single source of truth for the Mesh-LLM git revision the desktop macOS build
# actually fetches. Used identically by ci.yml, release.yml, and
# signed-macos-canary.yml.
#
# Why derive from desktop/src-tauri/Cargo.toml and NOT the root Cargo.lock:
# the desktop crate is `exclude`d from the root Cargo workspace and pins its own
# Mesh-LLM git `rev` in desktop/src-tauri/Cargo.toml. The macOS build runs
# `cargo fetch --manifest-path desktop/src-tauri/Cargo.toml`, which checks out
# THAT rev. The root Cargo.lock carries buzz-relay's separate Mesh pin (a
# different SHA), so resolving from it makes CI hunt a checkout that was never
# created ("mesh-llm checkout for <short> not found"). This resolver reads the
# desktop manifest — the manifest the fetch uses — and fails loudly on any
# inconsistency so a bad pin is a clear error, not a mysterious CI miss.
#
# Usage:
#   resolve-mesh-rev.sh                 # print the full 40-char rev
#   resolve-mesh-rev.sh --short         # print the 7-char short rev
#   resolve-mesh-rev.sh --find-checkout # print the cargo git checkout path,
#                                       #   failing loudly if it is absent
#
# Manifest path is overridable via MESH_MANIFEST (used by the tests).

fail() {
  echo "::error::resolve-mesh-rev: $*" >&2
  exit 1
}

manifest="${MESH_MANIFEST:-desktop/src-tauri/Cargo.toml}"

mode="rev"
case "${1:-}" in
  "") mode="rev" ;;
  --short) mode="short" ;;
  --find-checkout) mode="find" ;;
  *) fail "unknown argument: $1 (expected --short or --find-checkout)" ;;
esac

[[ -f "$manifest" ]] || fail "manifest not found: $manifest"

# Non-comment dependency lines that reference the Mesh-LLM git repository.
# (Portable to bash 3.2 — no `mapfile`.)
dep_lines=()
while IFS= read -r line; do
  [[ -n "$line" ]] && dep_lines+=("$line")
done < <(grep -E 'Mesh-LLM/mesh-llm\.git' "$manifest" | grep -vE '^[[:space:]]*#' || true)
[[ ${#dep_lines[@]} -gt 0 ]] \
  || fail "no Mesh-LLM git dependency found in $manifest"

revs=()
for line in "${dep_lines[@]}"; do
  rev="$(sed -nE 's/.*[^a-zA-Z0-9_]rev[[:space:]]*=[[:space:]]*"([0-9a-fA-F]{7,40})".*/\1/p' <<<"$line")"
  [[ -n "$rev" ]] \
    || fail "Mesh-LLM git dependency without an explicit rev in $manifest: ${line}"
  revs+=("$rev")
done

unique=()
while IFS= read -r r; do
  [[ -n "$r" ]] && unique+=("$r")
done < <(printf '%s\n' "${revs[@]}" | sort -u)
if [[ ${#unique[@]} -ne 1 ]]; then
  fail "inconsistent Mesh-LLM revs across desktop dependencies (${unique[*]}) — pin every Mesh-LLM dependency in $manifest to the same rev"
fi

rev="${unique[0]}"
short="${rev:0:7}"

case "$mode" in
  rev) printf '%s\n' "$rev" ;;
  short) printf '%s\n' "$short" ;;
  find)
    root="${CARGO_HOME:-$HOME/.cargo}/git/checkouts"
    [[ -d "$root" ]] \
      || fail "cargo git checkouts dir missing ($root) — run 'cargo fetch --manifest-path $manifest' before resolving the checkout"
    path="$(find "$root" -path "*/$short" -type d -name "$short" 2>/dev/null | head -1)"
    [[ -n "$path" ]] \
      || fail "Mesh-LLM checkout for $short not found under $root — 'cargo fetch --manifest-path $manifest' did not create it"
    printf '%s\n' "$path"
    ;;
esac
