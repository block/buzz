#!/bin/bash
# MIKE-49: pin the nip-am-verify path dependency (desktop/src-tauri/Cargo.toml)
# to an exact, verified commit of cccareers/buzz-auditor instead of whatever
# happens to be checked out in the sibling directory.
#
# Reads the pinned commit from desktop/src-tauri/NIP_AM_VERIFY_PINNED_REV
# (committed input), materializes it as a detached-HEAD git worktree at
# <this-checkout-toplevel>/.mike49-nip-am-verify-pinned, and fails loudly if
# the pinned commit cannot be produced byte-for-byte. Cargo.toml's path
# dependency points at that fixed location, which sits at a constant relative
# offset from Cargo.toml regardless of which worktree of block/buzz this
# script is run from -- unlike a hand-maintained "../../../buzz-auditor"
# relative path, which breaks under nested `.claude/worktrees/<id>/` checkouts.
#
# A pinned commit is not enough on its own: a worktree sitting at the right
# HEAD can still contain modified tracked files or stray untracked files (a
# prior manual edit, a half-finished build artifact left in the source tree,
# etc). This script rejects any dirty pinned worktree rather than silently
# building against it, and never deletes an existing ${PINNED_DIR} itself --
# a HEAD/dirty mismatch is reported so a human can inspect what's actually
# there before anything is removed.
#
# Not CI-portable: this still requires a local clone of the private
# cccareers/buzz-auditor repo on disk (default: a sibling of the MAIN
# block/buzz checkout, override with BUZZ_AUDITOR_SRC). Vendoring the crate
# into block/buzz, or fixing Cargo's git transport
# (net.git-fetch-with-cli), remain open follow-ups -- see the Cargo.toml
# comment above the dependency line.
set -euo pipefail

REPO_TOPLEVEL="$(git rev-parse --show-toplevel)"
PIN_FILE="${REPO_TOPLEVEL}/desktop/src-tauri/NIP_AM_VERIFY_PINNED_REV"
CARGO_TOML="${REPO_TOPLEVEL}/desktop/src-tauri/Cargo.toml"
PINNED_REV="$(tr -d '[:space:]' < "${PIN_FILE}")"
PINNED_DIR="${REPO_TOPLEVEL}/.mike49-nip-am-verify-pinned"

# Resolve a directory to its canonical absolute path (macOS has no `readlink -f`).
realpath_dir() {
  (cd "$1" 2>/dev/null && pwd -P)
}

if [ -z "${PINNED_REV}" ]; then
  echo "mike49-pin-nip-am-verify: ${PIN_FILE} is empty" >&2
  exit 1
fi

# Main block/buzz checkout, even when this script runs from a `.claude/worktrees/<id>` worktree.
MAIN_BUZZ_TOPLEVEL="$(git worktree list --porcelain | awk '/^worktree/{print $2; exit}')"
BUZZ_AUDITOR_SRC="${BUZZ_AUDITOR_SRC:-$(dirname "${MAIN_BUZZ_TOPLEVEL}")/buzz-auditor}"

if [ ! -d "${BUZZ_AUDITOR_SRC}/.git" ] && [ ! -f "${BUZZ_AUDITOR_SRC}/.git" ]; then
  echo "mike49-pin-nip-am-verify: no buzz-auditor git checkout at ${BUZZ_AUDITOR_SRC}" >&2
  echo "  set BUZZ_AUDITOR_SRC to a local clone of git@github.com:cccareers/buzz-auditor.git" >&2
  exit 1
fi

if ! git -C "${BUZZ_AUDITOR_SRC}" cat-file -e "${PINNED_REV}^{commit}" 2>/dev/null; then
  echo "mike49-pin-nip-am-verify: ${BUZZ_AUDITOR_SRC} does not have commit ${PINNED_REV}" >&2
  echo "  fetch it first: git -C '${BUZZ_AUDITOR_SRC}' fetch origin ${PINNED_REV}" >&2
  exit 1
fi

if [ -e "${PINNED_DIR}" ]; then
  # Reject anything at PINNED_DIR that isn't actually a worktree of
  # BUZZ_AUDITOR_SRC -- an unrelated pre-existing file/directory must not be
  # silently adopted (or, previously, force-removed) just because a path
  # happened to collide.
  PINNED_COMMON_DIR="$(git -C "${PINNED_DIR}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "")"
  AUDITOR_COMMON_DIR="$(git -C "${BUZZ_AUDITOR_SRC}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "")"
  # Compare the full canonical common-dir paths directly. Comparing only
  # their parent directories is not sufficient: two unrelated repos whose
  # .git storage happens to sit as siblings under one shared parent (a
  # worktree pool, a CI checkout cache) would wrongly compare equal even
  # though PINNED_DIR belongs to a different repo entirely.
  if [ -z "${PINNED_COMMON_DIR}" ] || [ "$(realpath_dir "${PINNED_COMMON_DIR}")" != "$(realpath_dir "${AUDITOR_COMMON_DIR}")" ]; then
    echo "mike49-pin-nip-am-verify: ${PINNED_DIR} already exists and is not a worktree of ${BUZZ_AUDITOR_SRC}" >&2
    echo "  refusing to touch it -- move or remove it yourself, then re-run this script" >&2
    exit 1
  fi

  CURRENT_HEAD="$(git -C "${PINNED_DIR}" rev-parse HEAD 2>/dev/null || echo "")"
  if [ "${CURRENT_HEAD}" != "${PINNED_REV}" ]; then
    echo "mike49-pin-nip-am-verify: ${PINNED_DIR} exists at ${CURRENT_HEAD}, expected ${PINNED_REV}" >&2
    echo "  not removing it automatically -- inspect it, then run:" >&2
    echo "    git -C '${BUZZ_AUDITOR_SRC}' worktree remove '${PINNED_DIR}'" >&2
    exit 1
  fi

  DIRTY_STATUS="$(git -C "${PINNED_DIR}" status --porcelain --ignore-submodules=none)"
  if [ -n "${DIRTY_STATUS}" ]; then
    echo "mike49-pin-nip-am-verify: ${PINNED_DIR} is at the pinned commit but has modified or untracked files:" >&2
    echo "${DIRTY_STATUS}" >&2
    echo "  not removing it automatically -- inspect it, then run:" >&2
    echo "    git -C '${BUZZ_AUDITOR_SRC}' worktree remove '${PINNED_DIR}'" >&2
    exit 1
  fi
else
  git -C "${BUZZ_AUDITOR_SRC}" worktree add --detach "${PINNED_DIR}" "${PINNED_REV}" >&2
fi

ACTUAL_HEAD="$(git -C "${PINNED_DIR}" rev-parse HEAD)"
if [ "${ACTUAL_HEAD}" != "${PINNED_REV}" ]; then
  echo "mike49-pin-nip-am-verify: pin failed, ${PINNED_DIR} is at ${ACTUAL_HEAD}, not ${PINNED_REV}" >&2
  exit 1
fi

if [ ! -d "${PINNED_DIR}/tools/nip-am-verify" ]; then
  echo "mike49-pin-nip-am-verify: ${PINNED_DIR}/tools/nip-am-verify missing at pinned commit ${PINNED_REV}" >&2
  exit 1
fi

# Prove Cargo's OWN dependency resolution points at this pinned worktree --
# not merely that Cargo.toml's "path = ..." field, read with grep/sed,
# parses to it. Manifest-text parsing can't see what Cargo itself would
# actually resolve (workspace-level [patch]/[replace], a stale Cargo.lock,
# multiple candidate manifests). `cargo metadata` is Cargo's own resolver,
# so this checks the same thing `cargo build` would use.
PINNED_NIP_AM_VERIFY_ABS="$(realpath_dir "${PINNED_DIR}/tools/nip-am-verify")"

if ! command -v cargo >/dev/null 2>&1; then
  echo "mike49-pin-nip-am-verify: cargo not on PATH -- cannot verify Cargo's resolved nip-am-verify path" >&2
  echo "  activate this repo's Hermit toolchain first: . ./bin/activate-hermit" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "mike49-pin-nip-am-verify: jq not on PATH -- cannot parse 'cargo metadata' output" >&2
  exit 1
fi

# --locked: resolve against the committed Cargo.lock as-is, and fail rather
# than silently rewriting it if it's stale -- the same lock `cargo build`
# will actually use, not a freshly-recomputed one this check happens to like.
CARGO_METADATA_STDERR_FILE="$(mktemp)"
if ! CARGO_METADATA_JSON="$(cargo metadata --format-version=1 --locked --manifest-path "${CARGO_TOML}" 2>"${CARGO_METADATA_STDERR_FILE}")"; then
  echo "mike49-pin-nip-am-verify: cargo metadata --locked failed to resolve ${CARGO_TOML}:" >&2
  cat "${CARGO_METADATA_STDERR_FILE}" >&2
  rm -f "${CARGO_METADATA_STDERR_FILE}"
  exit 1
fi
rm -f "${CARGO_METADATA_STDERR_FILE}"

# Walk the actual dependency EDGE from buzz-desktop (metadata.resolve.root)
# to its "nip_am_verify" dep, then resolve that edge's package id in
# `packages`. Filtering `packages` by name alone would match the first
# package called "nip-am-verify" anywhere in the resolved graph -- not
# necessarily the one buzz-desktop's own [dependencies] entry actually
# depends on (a same-named package could reach the graph transitively, or
# from a registry, even while the direct edge points somewhere else).
ROOT_PKG_ID="$(printf '%s' "${CARGO_METADATA_JSON}" | jq -r '.resolve.root // empty')"
if [ -z "${ROOT_PKG_ID}" ]; then
  echo "mike49-pin-nip-am-verify: cargo metadata reported no resolve.root for ${CARGO_TOML} (virtual workspace manifest?) -- this script assumes a single-crate manifest" >&2
  exit 1
fi
DEP_PKG_ID="$(printf '%s' "${CARGO_METADATA_JSON}" | jq -r --arg root "${ROOT_PKG_ID}" '
  [.resolve.nodes[] | select(.id == $root) | .deps[] | select(.name == "nip_am_verify") | .pkg] | .[0] // empty
')"
if [ -z "${DEP_PKG_ID}" ]; then
  echo "mike49-pin-nip-am-verify: buzz-desktop's own dependency edge has no 'nip_am_verify' entry in cargo metadata for ${CARGO_TOML}" >&2
  exit 1
fi
RESOLVED_MANIFEST_PATH="$(printf '%s' "${CARGO_METADATA_JSON}" | jq -r --arg id "${DEP_PKG_ID}" '[.packages[] | select(.id == $id) | .manifest_path] | .[0] // empty')"
if [ -z "${RESOLVED_MANIFEST_PATH}" ]; then
  echo "mike49-pin-nip-am-verify: cargo metadata's packages list has no entry for resolved id ${DEP_PKG_ID}" >&2
  exit 1
fi
CARGO_RESOLVED_ABS="$(realpath_dir "$(dirname "${RESOLVED_MANIFEST_PATH}")")"
if [ "${CARGO_RESOLVED_ABS}" != "${PINNED_NIP_AM_VERIFY_ABS}" ]; then
  echo "mike49-pin-nip-am-verify: buzz-desktop's nip_am_verify dependency edge resolves to ${CARGO_RESOLVED_ABS}, not the pinned worktree at ${PINNED_NIP_AM_VERIFY_ABS}" >&2
  exit 1
fi

echo "mike49-pin-nip-am-verify: pinned nip-am-verify to ${PINNED_REV} at ${PINNED_DIR}"
