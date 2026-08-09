#!/bin/sh
set -eu

archive="${1:?encrypted Memory archive must be decrypted before restore}"
target="${2:?Memory volume mount is required}"
action="${3:-prepare}"
stage="${target}/.buzz-restore-stage"
old="${target}/.buzz-restore-old"
garbage="${target}/.buzz-restore-garbage"
current="${target}/current"
listing="${target}/.buzz-restore-listing"
verbose_listing="${target}/.buzz-restore-verbose"

cleanup_ephemeral() {
  rm -rf "${stage}" "${garbage}" "${listing}" "${verbose_listing}"
}

recover_old() {
  if [ -d "${old}" ]; then
    if [ -e "${current}" ]; then
      rm -rf "${stage}"
      mv "${current}" "${stage}"
    fi
    mv "${old}" "${current}"
    rm -rf "${stage}"
  fi
}

case "${action}" in
  rollback)
    recover_old
    cleanup_ephemeral
    exit 0
    ;;
  finalize)
    test -d "${current}"
    rm -rf "${garbage}"
    if [ -d "${old}" ]; then
      # Once the prior vault has been renamed, it is no longer authoritative.
      # A crash during deletion can leave garbage, but rollback will never
      # mistake a partially deleted directory for the last-known-good vault.
      mv "${old}" "${garbage}"
      if [ "${BUZZ_TEST_MEMORY_RESTORE_FAILURE:-}" = "crash_during_finalize_delete" ]; then
        rm -rf "${garbage}/one"
        kill -KILL $$
      fi
    fi
    cleanup_ephemeral
    exit 0
    ;;
  prepare)
    ;;
  *)
    exit 64
    ;;
esac

# A prior process may have been killed after moving the old vault aside or
# after installing the unverified new vault. The old directory is always the
# last-known-good state, so recover it before beginning another attempt.
recover_old
cleanup_ephemeral
tar -tzf "${archive}" >"${listing}"
entry_count="$(wc -l <"${listing}" | tr -d ' ')"
test "${entry_count}" -gt 0
test "${entry_count}" -le 100000
if LC_ALL=C grep -q '[[:cntrl:]]' "${listing}"; then
  exit 73
fi
if awk '/^\// || /(^|\/)\.\.(\/|$)/ { found=1 } END { exit found ? 0 : 1 }' \
  "${listing}"; then
  exit 74
fi
tar -tvzf "${archive}" >"${verbose_listing}"
awk '
  $1 !~ /^[-d]/ { exit 1 }
  { total += $3 }
  END { if (total > 1073741824) exit 1 }
' "${verbose_listing}"
mkdir "${stage}"
tar -xzf "${archive}" -C "${stage}"
test -n "$(find "${stage}" -mindepth 1 -print -quit)"

if [ "${BUZZ_TEST_MEMORY_RESTORE_FAILURE:-}" = "after_extract" ]; then
  exit 71
fi

rollback() {
  status=$?
  if [ "${status}" -ne 0 ] && [ -d "${old}" ]; then
    recover_old
  fi
  cleanup_ephemeral
  exit "${status}"
}
trap rollback EXIT HUP INT TERM

if [ -e "${current}" ]; then
  mv "${current}" "${old}"
fi
if [ "${BUZZ_TEST_MEMORY_RESTORE_FAILURE:-}" = "after_old_rename" ]; then
  exit 72
fi
if [ "${BUZZ_TEST_MEMORY_RESTORE_FAILURE:-}" = "crash_after_old_rename" ]; then
  kill -KILL $$
fi
mv "${stage}" "${current}"
if [ "${BUZZ_TEST_MEMORY_RESTORE_FAILURE:-}" = "crash_after_new_install" ]; then
  kill -KILL $$
fi
rm -f "${listing}" "${verbose_listing}"
trap - EXIT HUP INT TERM
