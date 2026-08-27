#!/usr/bin/env bash
# Pre-push scope signal (warn-only, never fails the push).
#
# The scoped pre-push lanes derive their file set from `git diff origin/main...HEAD`,
# and lefthook's own stock discovery is `git diff HEAD @{push}` — both inspect the
# checked-out HEAD, not the SHAs actually being pushed. Pushing a non-checked-out
# ref (e.g. `git push origin other-branch`, an explicit refspec, or `--all`) can
# therefore run the scoped lanes against HEAD while pushing a different commit, so
# the pushed ref's path-scoped checks are only exercised by CI.
#
# This lane reads the pre-push stdin refspecs and prints a loud, non-fatal warning
# naming each pushed ref whose commit is not the checked-out HEAD. It never blocks
# the push; CI remains the authoritative gate for a non-HEAD ref.
#
# Stdin format (one line per ref, git-provided):
#   <local_ref> <local_sha> <remote_ref> <remote_sha>
set -euo pipefail

head_sha=$(git rev-parse HEAD 2>/dev/null) || exit 0

divergent=""
while read -r local_ref local_sha remote_ref remote_sha; do
  # Empty stdin (up-to-date push with no refs) or malformed line: skip.
  [ -n "${local_sha:-}" ] || continue
  # Deletion push (local sha is all-zeros): nothing built locally to validate.
  case "$local_sha" in *[!0]*) : ;; *) continue ;; esac
  # Pushed commit equals the checked-out HEAD: scoped lanes validated it.
  [ "$local_sha" = "$head_sha" ] && continue
  divergent="${divergent}  ${local_ref:-?} -> ${remote_ref:-?} ($(git rev-parse --short "$local_sha" 2>/dev/null || echo "$local_sha"))
"
done

if [ -n "$divergent" ]; then
  {
    echo "Pushing commits that are not the checked-out HEAD:"
    printf '%s' "$divergent"
    echo "Local scoped lanes validated HEAD only; these refs rely on CI for"
    echo "path-scoped checks. This is a warning, not a failure."
  } >&2
fi

exit 0
