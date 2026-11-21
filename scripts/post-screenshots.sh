#!/usr/bin/env bash
set -euo pipefail

# Host PR screenshots as blobs on a GitHub branch and post them to a PR.
#
# Images are pushed to a per-developer branch (agent-screenshots/<login>) on a
# remote the caller can actually push to — which is NOT necessarily the repo
# the PR lives in. Fork contributors have no push access to block/buzz, so the
# hosting remote is auto-detected (or picked with --remote) and the raw image
# base URL is derived from that remote's owner/repo. The PR comment always
# targets the PR repo (block/buzz by default).
#
# macOS ships bash 3.2: no mapfile, no associative arrays, and empty arrays
# must be expanded with the ${arr[@]+"${arr[@]}"} guard so `set -u` is happy.

PR_REPO_DEFAULT="block/buzz"
MAX_PUSH_ATTEMPTS=5

usage() {
  cat <<USAGE
Usage: $0 [options] <pr-number> <png-dir> [comment-body-file]

Options:
  --remote <name>    Git remote to host the screenshot blobs on.
                     Default: auto-detected (first pushable of origin, fork,
                     a remote owned by your gh login, any other remote).
  --pr-repo <o/r>    Repo the PR lives in (default: ${PR_REPO_DEFAULT}).
  --dry-run          Do everything except 'git push' and 'gh pr comment'.
                     Prints the resolved remote, owner/repo, RAW_BASE, image
                     URLs and the rendered comment body.
  --self-test        Run the URL-parsing / remote-detection unit tests and exit.
                     Needs no repo and no network.
  -h, --help         Show this help.

Environment:
  POST_SCREENSHOTS_GH_USER  Override the gh login used for the branch name.
USAGE
}

# ---------------------------------------------------------------------------
# URL parsing
# ---------------------------------------------------------------------------

# Parse "owner/repo" out of a git remote URL.
# Handles https://github.com/o/r(.git), git@github.com:o/r(.git),
# ssh://git@github.com[:22]/o/r.git, git://…, and userinfo in the authority.
parse_owner_repo() {
  local url="${1:-}" path=""

  case "$url" in
    ssh://* | https://* | http://* | git+ssh://* | git://*)
      path="${url#*://}" # [user[:pass]@]host[:port]/owner/repo(.git)
      path="${path#*@}"  # strip userinfo, if any
      path="${path#*/}"  # strip host[:port]
      ;;
    *:*)
      # scp-like syntax: [user@]host:owner/repo(.git)
      path="${url#*:}"
      ;;
    *)
      path="$url"
      ;;
  esac

  path="${path#/}"
  path="${path%/}"
  path="${path%.git}"
  path="${path%/}"

  if [[ "$path" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]; then
    printf '%s\n' "$path"
    return 0
  fi
  return 1
}

# ---------------------------------------------------------------------------
# Remote detection (these four are the seams the self-test stubs out)
# ---------------------------------------------------------------------------

list_remotes() {
  git remote
}

remote_push_url() {
  git remote get-url --push "$1" 2>/dev/null && return 0
  # git < 2.7 has no `remote get-url`.
  git config --get "remote.$1.pushurl" 2>/dev/null && return 0
  git config --get "remote.$1.url" 2>/dev/null
}

gh_login() {
  if [[ -n "${POST_SCREENSHOTS_GH_USER:-}" ]]; then
    printf '%s\n' "$POST_SCREENSHOTS_GH_USER"
    return 0
  fi
  gh api user --jq .login 2>/dev/null
}

# Can we push to this remote? Ask GitHub first (cheap and definitive), then
# fall back to a dry-run push probe against a ref that cannot already exist.
remote_is_pushable() {
  local remote="$1" url owner_repo perm probe_ref
  url="$(remote_push_url "$remote")" || return 1
  [[ -n "$url" ]] || return 1

  if owner_repo="$(parse_owner_repo "$url")" && command -v gh >/dev/null 2>&1; then
    perm="$(gh repo view "$owner_repo" --json viewerPermission --jq .viewerPermission 2>/dev/null || true)"
    case "$perm" in
      ADMIN | MAINTAIN | WRITE) return 0 ;;
      READ | TRIAGE | NONE) return 1 ;;
    esac
  fi

  # The probe runs unattended during auto-detection, so it must never block on
  # an interactive prompt — git's own, a GUI credential helper (e.g. Git
  # Credential Manager), or ssh asking for a passphrase. Cached credentials
  # still work; only the interactive fallback is switched off.
  probe_ref="refs/heads/agent-screenshots-pushcheck/$$"
  GIT_TERMINAL_PROMPT=0 \
    GIT_ASKPASS=echo \
    SSH_ASKPASS=echo \
    GIT_SSH_COMMAND="${GIT_SSH_COMMAND:-ssh} -o BatchMode=yes" \
    git -c credential.interactive=false \
    push --dry-run "$remote" "HEAD:${probe_ref}" >/dev/null 2>&1
}

# Prefer origin, then a remote literally named "fork", then a remote owned by
# the gh login, then anything else that accepts a push.
detect_remote() {
  local login remote url owner_repo
  local fallbacks=""

  for remote in $(list_remotes); do
    case "$remote" in
      origin)
        if remote_is_pushable "$remote"; then
          printf '%s\n' "$remote"
          return 0
        fi
        ;;
      *) fallbacks="${fallbacks}${remote} " ;;
    esac
  done

  for remote in $fallbacks; do
    if [[ "$remote" == "fork" ]] && remote_is_pushable "$remote"; then
      printf '%s\n' "$remote"
      return 0
    fi
  done

  login="$(gh_login || true)"
  if [[ -n "$login" ]]; then
    for remote in $fallbacks; do
      url="$(remote_push_url "$remote")" || continue
      owner_repo="$(parse_owner_repo "$url")" || continue
      if [[ "${owner_repo%%/*}" == "$login" ]] && remote_is_pushable "$remote"; then
        printf '%s\n' "$remote"
        return 0
      fi
    done
  fi

  for remote in $fallbacks; do
    if remote_is_pushable "$remote"; then
      printf '%s\n' "$remote"
      return 0
    fi
  done

  return 1
}

# ---------------------------------------------------------------------------
# Self-test: exercises URL parsing and remote detection with stubbed seams.
# ---------------------------------------------------------------------------

SELF_TEST_FAILURES=0

expect_parse() {
  local url="$1" want="$2" got
  got="$(parse_owner_repo "$url" || echo '<none>')"
  if [[ "$got" == "$want" ]]; then
    echo "ok   parse_owner_repo '$url' -> $got"
  else
    echo "FAIL parse_owner_repo '$url' -> '$got' (want '$want')"
    SELF_TEST_FAILURES=$((SELF_TEST_FAILURES + 1))
  fi
}

expect_remote() {
  local want="$1" got
  got="$(detect_remote || echo '<none>')"
  if [[ "$got" == "$want" ]]; then
    echo "ok   detect_remote [${TEST_REMOTES}] pushable=[${TEST_PUSHABLE}] -> $got"
  else
    echo "FAIL detect_remote [${TEST_REMOTES}] pushable=[${TEST_PUSHABLE}] -> '$got' (want '$want')"
    SELF_TEST_FAILURES=$((SELF_TEST_FAILURES + 1))
  fi
}

run_self_test() {
  echo "-- parse_owner_repo --"
  expect_parse "https://github.com/block/buzz.git" "block/buzz"
  expect_parse "https://github.com/block/buzz" "block/buzz"
  expect_parse "https://github.com/block/buzz/" "block/buzz"
  expect_parse "git@github.com:mfethe1/buzz.git" "mfethe1/buzz"
  expect_parse "git@github.com:mfethe1/buzz" "mfethe1/buzz"
  expect_parse "ssh://git@github.com/block/buzz.git" "block/buzz"
  expect_parse "ssh://git@github.com:22/block/buzz.git" "block/buzz"
  expect_parse "git://github.com/block/buzz.git" "block/buzz"
  expect_parse "https://user:token@github.com/block/buzz.git" "block/buzz"
  expect_parse "https://github.com/block/buzz-releases.git" "block/buzz-releases"
  expect_parse "/local/path/to/repo" "<none>"
  expect_parse "" "<none>"

  echo
  echo "-- detect_remote --"
  # Stub the seams. Redefining a function is bash-3.2 safe.
  # shellcheck disable=SC2086 # intentional word splitting: one remote per line
  list_remotes() { printf '%s\n' $TEST_REMOTES; }
  remote_push_url() {
    local pair
    for pair in $TEST_URLS; do
      case "$pair" in
        "$1="*)
          printf '%s\n' "${pair#*=}"
          return 0
          ;;
      esac
    done
    return 1
  }
  remote_is_pushable() {
    local name
    for name in $TEST_PUSHABLE; do
      if [[ "$name" == "$1" ]]; then
        return 0
      fi
    done
    return 1
  }
  gh_login() { printf '%s\n' "$TEST_LOGIN"; }

  TEST_LOGIN="mfethe1"
  TEST_URLS="origin=https://github.com/block/buzz.git fork=git@github.com:mfethe1/buzz.git mine=https://github.com/mfethe1/buzz-two.git other=https://github.com/someone/buzz.git"

  # Maintainer: origin wins.
  TEST_REMOTES="origin fork"
  TEST_PUSHABLE="origin fork"
  expect_remote "origin"

  # Fork contributor: origin is read-only, so "fork" wins.
  TEST_REMOTES="origin fork mine"
  TEST_PUSHABLE="fork mine"
  expect_remote "fork"

  # No remote named fork: owner-matches-login beats an unrelated remote.
  TEST_REMOTES="origin other mine"
  TEST_PUSHABLE="other mine"
  expect_remote "mine"

  # "fork" exists but is not pushable: fall through to the owner match.
  TEST_REMOTES="origin fork mine"
  TEST_PUSHABLE="mine"
  expect_remote "mine"

  # Nothing owned by the login: any pushable remote beats nothing.
  TEST_REMOTES="origin other"
  TEST_PUSHABLE="other"
  expect_remote "other"

  # Nothing is pushable at all.
  TEST_REMOTES="origin other"
  TEST_PUSHABLE=""
  expect_remote "<none>"

  echo
  if [[ $SELF_TEST_FAILURES -eq 0 ]]; then
    echo "self-test: all checks passed"
    return 0
  fi
  echo "self-test: ${SELF_TEST_FAILURES} check(s) failed" >&2
  return 1
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

REMOTE=""
PR_REPO="$PR_REPO_DEFAULT"
DRY_RUN=0
SELF_TEST=0
POSITIONAL=()

require_value() {
  if [[ -z "${2:-}" ]]; then
    echo "error: $1 requires a value" >&2
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --remote)
      require_value --remote "${2:-}"
      REMOTE="$2"
      shift
      ;;
    --remote=*) REMOTE="${1#*=}" ;;
    --pr-repo)
      require_value --pr-repo "${2:-}"
      PR_REPO="$2"
      shift
      ;;
    --pr-repo=*) PR_REPO="${1#*=}" ;;
    --dry-run) DRY_RUN=1 ;;
    --self-test) SELF_TEST=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        POSITIONAL+=("$1")
        shift
      done
      break
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *) POSITIONAL+=("$1") ;;
  esac
  shift
done

if [[ $SELF_TEST -eq 1 ]]; then
  run_self_test
  exit $?
fi

set -- ${POSITIONAL[@]+"${POSITIONAL[@]}"}

if [[ $# -lt 2 ]]; then
  usage >&2
  exit 1
fi

PR="$1"
PNG_DIR="$2"
BODY_FILE="${3:-}"

if ! [[ "$PR" =~ ^[0-9]+$ ]]; then
  echo "error: PR number must be a positive integer" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Resolve the hosting remote and its owner/repo
# ---------------------------------------------------------------------------

if [[ -n "$REMOTE" ]]; then
  if ! git remote get-url "$REMOTE" >/dev/null 2>&1; then
    echo "error: no such git remote: $REMOTE" >&2
    exit 1
  fi
else
  if ! REMOTE="$(detect_remote)"; then
    echo "error: could not find a git remote you can push to." >&2
    echo "       Add a fork remote (git remote add fork git@github.com:<you>/buzz.git)" >&2
    echo "       or pass --remote <name> explicitly." >&2
    exit 1
  fi
  echo "Auto-detected hosting remote: ${REMOTE}"
fi

REMOTE_URL="$(remote_push_url "$REMOTE")"
if ! HOST_REPO="$(parse_owner_repo "$REMOTE_URL")"; then
  echo "error: could not parse owner/repo from remote '${REMOTE}' url: ${REMOTE_URL}" >&2
  exit 1
fi

if ! GH_USER="$(gh_login)" || [[ -z "$GH_USER" ]]; then
  echo "error: could not determine your GitHub login (gh api user)." >&2
  echo "       Run 'gh auth login' or set POST_SCREENSHOTS_GH_USER." >&2
  exit 1
fi
BRANCH="agent-screenshots/${GH_USER}"

# macOS ships bash 3.2, which lacks mapfile — build the array with read.
PNGS=()
while IFS= read -r PNG; do
  PNGS+=("$PNG")
done < <(find "$PNG_DIR" -maxdepth 1 -name "*.png" -type f | sort)
if [[ ${#PNGS[@]} -eq 0 ]]; then
  echo "error: no PNGs found in $PNG_DIR" >&2
  exit 1
fi

NEW_ENTRIES=""
TREE_PATHS=()
for PNG in "${PNGS[@]}"; do
  FILENAME=$(basename "$PNG")
  if ! [[ "$FILENAME" =~ ^[a-zA-Z0-9_.-]+$ ]]; then
    echo "error: invalid PNG filename (must be alphanumeric, dots, hyphens, underscores): $FILENAME" >&2
    exit 1
  fi
  BLOB=$(git hash-object -w "$PNG")
  TREE_PATH="pr-${PR}--${FILENAME}"
  NEW_ENTRIES+="$(printf '100644 blob %s\t%s' "$BLOB" "$TREE_PATH")"$'\n'
  TREE_PATHS+=("$TREE_PATH")
done

# ---------------------------------------------------------------------------
# Build the tree on the current branch tip, then push (retrying on races)
# ---------------------------------------------------------------------------

TIP=""
TREE=""
COMMIT=""
LEASE=""

# Re-read the remote branch tip and rebuild the commit on top of it, keeping
# every entry that exists there now (another session may have added images for
# other PRs since we last looked) except this PR's, which we replace.
build_commit_on_tip() {
  local existing="" combined
  local parent_args=()

  TIP=""
  if git fetch --quiet "$REMOTE" "+refs/heads/${BRANCH}:refs/remotes/${REMOTE}/${BRANCH}" 2>/dev/null; then
    TIP="$(git rev-parse --verify --quiet "refs/remotes/${REMOTE}/${BRANCH}" || true)"
  fi

  if [[ -n "$TIP" ]]; then
    existing=$(git ls-tree "$TIP" | grep -v $'\t'"\"\\{0,1\\}pr-${PR}--" || true)
    parent_args=(-p "$TIP")
    LEASE="refs/heads/${BRANCH}:${TIP}"
  else
    # No branch on the remote yet — the lease demands it still not exist.
    LEASE="refs/heads/${BRANCH}:"
  fi

  combined=$(printf '%s\n' "$existing" "$NEW_ENTRIES" | grep -v '^$')
  TREE=$(printf '%s\n' "$combined" | git mktree)
  # ${arr[@]+...} guards the empty-array case, which trips set -u on bash 3.2.
  COMMIT=$(git commit-tree "$TREE" ${parent_args[@]+"${parent_args[@]}"} -m "screenshots: PR #${PR}")
}

ATTEMPT=1
while :; do
  build_commit_on_tip

  if [[ $DRY_RUN -eq 1 ]]; then
    break
  fi

  if PUSH_OUT=$(git push --force-with-lease="$LEASE" "$REMOTE" "${COMMIT}:refs/heads/${BRANCH}" 2>&1); then
    break
  fi

  if [[ $ATTEMPT -ge $MAX_PUSH_ATTEMPTS ]] ||
    ! printf '%s\n' "$PUSH_OUT" | grep -qiE 'stale info|non-fast-forward|fetch first|rejected'; then
    printf '%s\n' "$PUSH_OUT" >&2
    echo "error: failed to push screenshots to ${REMOTE} (${HOST_REPO}) after ${ATTEMPT} attempt(s)." >&2
    exit 1
  fi

  echo "note: ${BRANCH} moved on ${REMOTE} — rebuilding on the new tip (attempt ${ATTEMPT}/${MAX_PUSH_ATTEMPTS})" >&2
  ATTEMPT=$((ATTEMPT + 1))
  sleep 1
done

RAW_BASE="https://raw.githubusercontent.com/${HOST_REPO}/${COMMIT}"

# Parallel name/url arrays instead of an associative array (bash 4+ only).
IMAGE_NAMES=()
IMAGE_URLS=()
for i in "${!PNGS[@]}"; do
  IMAGE_NAMES+=("$(basename "${PNGS[$i]}" .png)")
  IMAGE_URLS+=("${RAW_BASE}/${TREE_PATHS[$i]}")
done

HOSTED_ELSEWHERE=0
if [[ "$HOST_REPO" != "$PR_REPO" ]]; then
  HOSTED_ELSEWHERE=1
fi

if [[ -n "$BODY_FILE" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  "$SCRIPT_DIR/check-pr-image-urls.sh" "$BODY_FILE"
  COMMENT_BODY="$(cat "$BODY_FILE")"
  UNREFERENCED=()
  for i in "${!IMAGE_NAMES[@]}"; do
    NAME="${IMAGE_NAMES[$i]}"
    URL="${IMAGE_URLS[$i]}"
    PLACEHOLDER="{{${NAME}}}"
    if [[ "$COMMENT_BODY" == *"$PLACEHOLDER"* ]]; then
      COMMENT_BODY="${COMMENT_BODY//"$PLACEHOLDER"/![$NAME]($URL)}"
    else
      UNREFERENCED+=("${NAME}"$'\t'"${URL}")
    fi
  done
  if [[ ${#UNREFERENCED[@]} -gt 0 ]]; then
    while IFS=$'\t' read -r NAME URL; do
      COMMENT_BODY+=$'\n\n'"![${NAME}](${URL})"
    done < <(printf '%s\n' "${UNREFERENCED[@]}" | sort)
  fi
else
  COMMENT_BODY="## Screenshots"$'\n\n'
  for URL in "${IMAGE_URLS[@]}"; do
    FILENAME=$(basename "$URL")
    NAME="${FILENAME%.png}"
    COMMENT_BODY+="![${NAME}](${URL})"$'\n\n'
  done
fi

if [[ $HOSTED_ELSEWHERE -eq 1 ]]; then
  COMMENT_BODY+=$'\n'"<sub>Images hosted on \`${HOST_REPO}\` (branch \`${BRANCH}\`, commit \`${COMMIT}\`) — the author has no push access to \`${PR_REPO}\`.</sub>"$'\n'
fi

if [[ $DRY_RUN -eq 1 ]]; then
  echo "--- dry run: nothing pushed, no comment posted ---"
  echo "hosting remote : ${REMOTE}"
  echo "hosting url    : ${REMOTE_URL}"
  echo "hosting repo   : ${HOST_REPO}"
  echo "pr repo        : ${PR_REPO} (PR #${PR})"
  echo "branch         : ${BRANCH}"
  echo "remote tip     : ${TIP:-<none - branch would be created>}"
  echo "push lease     : ${LEASE}"
  echo "tree           : ${TREE}"
  echo "commit         : ${COMMIT}"
  echo "RAW_BASE       : ${RAW_BASE}"
  if [[ $HOSTED_ELSEWHERE -eq 1 ]]; then
    echo "note           : images would be hosted on ${HOST_REPO}, not ${PR_REPO}"
  fi
  echo "images:"
  for URL in "${IMAGE_URLS[@]}"; do
    echo "  ${URL}"
  done
  echo "--- comment body ---"
  printf '%s\n' "$COMMENT_BODY"
  echo "--- end comment body ---"
  exit 0
fi

if [[ $HOSTED_ELSEWHERE -eq 1 ]]; then
  echo "note: images hosted on ${HOST_REPO} (remote '${REMOTE}'), not ${PR_REPO} — the comment still goes to ${PR_REPO}#${PR}"
fi

gh pr comment "$PR" --repo "$PR_REPO" --body "$COMMENT_BODY"
echo "Posted ${#PNGS[@]} screenshot(s) to PR #${PR}"
