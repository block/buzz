#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"
cat >"$tmp/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
printf '%q ' "$@" >>"$GH_CALLS"
printf '\n' >>"$GH_CALLS"

[[ "${1:-}" == api ]] || { echo "expected gh api" >&2; exit 91; }
if [[ "${2:-}" == graphql ]]; then
  expected_query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewDecision}}}'
  [[ "$#" -eq 12 && "$3" == -f && "$4" == "query=$expected_query" &&
    "$5" == -F && "$6" == owner=block &&
    "$7" == -F && "$8" == repo=buzz &&
    "$9" == -F && "${10}" == number=123 &&
    "${11}" == --jq && "${12}" == '.data.repository.pullRequest' ]] || {
    echo "GraphQL call does not match the deployed query contract" >&2; exit 92;
  }
  if [[ -n "${REVIEW_DECISION:-}" ]]; then printf '%s\n' "$REVIEW_DECISION"; else printf '%s\n' '{"reviewDecision":"APPROVED"}'; fi
elif [[ "$#" -eq 4 && "$2" == --paginate && "$3" == --slurp && "$4" == "repos/block/buzz/pulls/123/reviews?per_page=100&page=1" ]]; then
  [[ "${GH_FAIL_REVIEWS:-false}" != true ]] || { echo "simulated reviews API failure" >&2; exit 94; }
  if [[ -n "${REVIEWS:-}" ]]; then printf '%s\n' "$REVIEWS"; else printf '%s\n' '[[],[{"state":"APPROVED","commit_id":"head","author_association":"MEMBER"}]]'; fi
else
  echo "unexpected or malformed gh call: $*" >&2
  exit 95
fi
GH
chmod +x "$tmp/bin/gh"

run_authorization() {
  (cd "$repo_root" && PATH="$tmp/bin:$PATH" GH_CALLS="$tmp/calls" GH_TOKEN=test \
    GITHUB_REPOSITORY=block/buzz PR_NUMBER=123 PR_HEAD_SHA=head \
    REVIEW_DECISION="${REVIEW_DECISION-}" REVIEWS="${REVIEWS-}" GH_FAIL_REVIEWS="${GH_FAIL_REVIEWS-false}" \
    scripts/verify-desktop-release-authorization.sh)
}

: >"$tmp/calls"
run_authorization
! grep -Fq 'rule-suites' "$tmp/calls"

for invalid in \
  '[[{"state":"APPROVED","commit_id":"stale","author_association":"MEMBER"}]]' \
  '[[{"state":"APPROVED","commit_id":"head","author_association":"NONE"}]]' \
  '[[{"state":"CHANGES_REQUESTED","commit_id":"head","author_association":"MEMBER"}]]'; do
  : >"$tmp/calls"
  if REVIEWS="$invalid" run_authorization >/dev/null 2>&1; then
    echo "invalid approval was accepted: $invalid" >&2
    exit 1
  fi
done

: >"$tmp/calls"
if REVIEW_DECISION='{"reviewDecision":"CHANGES_REQUESTED"}' run_authorization >/dev/null 2>&1; then
  echo "changes-requested review decision was accepted" >&2
  exit 1
fi

: >"$tmp/calls"
if GH_FAIL_REVIEWS=true run_authorization >/dev/null 2>&1; then
  echo "reviews API failure was ignored" >&2
  exit 1
fi

echo "desktop release authorization passed"
