#!/bin/sh
set -eu

archive="${1:?encrypted Memory archive must be decrypted before restore}"
target="${2:?Memory volume mount is required}"
stage="${target}/.buzz-restore-stage"
old="${target}/.buzz-restore-old"
current="${target}/current"
listing="${target}/.buzz-restore-listing"
verbose_listing="${target}/.buzz-restore-verbose"

rm -rf "${stage}" "${old}" "${listing}" "${verbose_listing}"
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
  if [ "${status}" -ne 0 ] && [ -d "${old}" ] && [ ! -e "${current}" ]; then
    mv "${old}" "${current}"
  fi
  rm -rf "${stage}" "${listing}" "${verbose_listing}"
  exit "${status}"
}
trap rollback EXIT HUP INT TERM

if [ -e "${current}" ]; then
  mv "${current}" "${old}"
fi
if [ "${BUZZ_TEST_MEMORY_RESTORE_FAILURE:-}" = "after_old_rename" ]; then
  exit 72
fi
mv "${stage}" "${current}"
rm -rf "${old}"
rm -f "${listing}" "${verbose_listing}"
trap - EXIT HUP INT TERM
