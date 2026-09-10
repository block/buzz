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
  if [ -z "${PINNED_COMMON_DIR}" ] || [ "$(realpath_dir "$(dirname "${PINNED_COMMON_DIR}")")" != "$(realpath_dir "$(dirname "${AUDITOR_COMMON_DIR}")")" ]; then
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

# Prove Cargo's own resolved dependency path is this pinned worktree, not a
# stale relative path left over from a different checkout layout.
CARGO_DEP_LINE="$(grep -E '^nip-am-verify = ' "${CARGO_TOML}" || true)"
if [ -z "${CARGO_DEP_LINE}" ]; then
  echo "mike49-pin-nip-am-verify: no nip-am-verify path dependency found in ${CARGO_TOML}" >&2
  exit 1
fi
CARGO_DEP_PATH="$(printf '%s' "${CARGO_DEP_LINE}" | sed -E 's/.*path = "([^"]+)".*/\1/')"
CARGO_DEP_ABS="$(cd "$(dirname "${CARGO_TOML}")" && realpath_dir "${CARGO_DEP_PATH}")"
PINNED_NIP_AM_VERIFY_ABS="$(realpath_dir "${PINNED_DIR}/tools/nip-am-verify")"
if [ "${CARGO_DEP_ABS}" != "${PINNED_NIP_AM_VERIFY_ABS}" ]; then
  echo "mike49-pin-nip-am-verify: Cargo.toml's nip-am-verify path resolves to ${CARGO_DEP_ABS}, not the pinned worktree at ${PINNED_NIP_AM_VERIFY_ABS}" >&2
  exit 1
fi

echo "mike49-pin-nip-am-verify: pinned nip-am-verify to ${PINNED_REV} at ${PINNED_DIR}"
