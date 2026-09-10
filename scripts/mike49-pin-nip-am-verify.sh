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
# Not CI-portable: this still requires a local clone of the private
# cccareers/buzz-auditor repo on disk (default: a sibling of the MAIN
# block/buzz checkout, override with BUZZ_AUDITOR_SRC). Vendoring the crate
# into block/buzz, or fixing Cargo's git transport
# (net.git-fetch-with-cli), remain open follow-ups -- see the Cargo.toml
# comment above the dependency line.
set -euo pipefail

REPO_TOPLEVEL="$(git rev-parse --show-toplevel)"
PIN_FILE="${REPO_TOPLEVEL}/desktop/src-tauri/NIP_AM_VERIFY_PINNED_REV"
PINNED_REV="$(tr -d '[:space:]' < "${PIN_FILE}")"
PINNED_DIR="${REPO_TOPLEVEL}/.mike49-nip-am-verify-pinned"

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
  CURRENT_HEAD="$(git -C "${PINNED_DIR}" rev-parse HEAD 2>/dev/null || echo "")"
  if [ "${CURRENT_HEAD}" != "${PINNED_REV}" ]; then
    echo "mike49-pin-nip-am-verify: ${PINNED_DIR} exists at ${CURRENT_HEAD}, expected ${PINNED_REV}; removing and re-pinning" >&2
    git -C "${BUZZ_AUDITOR_SRC}" worktree remove --force "${PINNED_DIR}" 2>/dev/null || rm -rf "${PINNED_DIR}"
  fi
fi

if [ ! -e "${PINNED_DIR}" ]; then
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

echo "mike49-pin-nip-am-verify: pinned nip-am-verify to ${PINNED_REV} at ${PINNED_DIR}"
