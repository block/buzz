#!/usr/bin/env bash
# buzz-session-name.sh — resolve the name of the Claude Code session running it.
#
#   buzz-session-name.sh              print the identity slug (filename-safe)
#   buzz-session-name.sh --display    print the display name (human-readable)
#   buzz-session-name.sh --both       print "<slug>\t<display>"
#
# A Buzz identity belongs to a *session*, not to a directory, so the name has to
# come from the session itself. Claude Code exports CLAUDE_CODE_SESSION_ID and
# writes the session transcript to
#   ~/.claude/projects/<cwd with / and . replaced by ->/<session-id>.jsonl
# where the title set by /rename appears as "customTitle" (and "agentName").
# /rename can be run repeatedly, so the LAST occurrence in the file wins.
#
# Resolution order — the first tier whose slug is non-empty wins, and both the
# slug and the display name are then derived from that same tier so the two can
# never disagree:
#
#   1. customTitle (else agentName) from this session's transcript
#   2. the git worktree directory name
#   3. session-<first 8 of CLAUDE_CODE_SESSION_ID>
#   4. session-<first 8 of sha256(cwd)>   — no session id and no git repo
#
# This never fails and never prints an empty name: a session with no /rename,
# no session id and no git repo still gets a stable identity from tier 4.
#
# Sanitisation. A /rename title is free text — spaces, emoji, slashes, 300
# characters of it. The slug is lowercased, reduced to [a-z0-9._-], collapsed,
# stripped of leading/trailing "-._" and cut to 64 chars, which makes path
# traversal impossible (a "/" can only ever become "-", and a leading ".." is
# stripped). The display name keeps the original characters — emoji included —
# with control characters removed, whitespace collapsed, and a 64-character cap.
set -uo pipefail

MODE=slug
FROM_STDIN=0
while [ $# -gt 0 ]; do
  case "$1" in
    ''|--slug)  MODE=slug ;;
    --display)  MODE=display ;;
    --both)     MODE=both ;;
    # --sanitize applies the rules below to a string on stdin instead of
    # resolving one. Used for explicit names, which land in a filename too.
    --sanitize) FROM_STDIN=1 ;;
    -h|--help)  sed -n '2,32p' "$0"; exit 0 ;;
    *) printf 'usage: %s [--slug|--display|--both] [--sanitize]\n' "$0" >&2; exit 2 ;;
  esac
  shift
done

TAB=$'\t'

# --- sanitiser ---------------------------------------------------------------
# Reads the raw candidate on stdin, prints "<slug>\t<display>".
PY_SANITIZE='
import re, sys, unicodedata

raw = sys.stdin.buffer.read().decode("utf-8", "replace")
kept = []
for ch in raw:
    cat = unicodedata.category(ch)
    if ch in "\t\n\r" or cat in ("Zs", "Zl", "Zp"):
        kept.append(" ")
    elif cat in ("Cc", "Cf", "Cs", "Co", "Cn"):
        continue          # control, format, surrogate, private-use, unassigned
    else:
        kept.append(ch)

display = re.sub(r"\s+", " ", "".join(kept)).strip()[:64].strip()

slug = re.sub(r"[^A-Za-z0-9._-]+", "-", display).lower()
slug = re.sub(r"-{2,}", "-", slug)
slug = re.sub(r"\.{2,}", ".", slug)
slug = slug.strip("-._")[:64].strip("-._")

sys.stdout.write("%s\t%s\n" % (slug, display))
'

HAVE_PYTHON=0
command -v python3 >/dev/null 2>&1 && HAVE_PYTHON=1

sanitize() {
  if [ "$HAVE_PYTHON" = 1 ]; then
    printf '%s' "$1" | python3 -c "$PY_SANITIZE" 2>/dev/null && return 0
  fi
  # Pure-shell fallback so the resolver still works without python3. Byte-wise,
  # so non-ASCII collapses to "-" in the slug; the display keeps what tr leaves.
  local s slug
  s=$(printf '%s' "$1" | tr '\t\n\r' '   ' | tr -d '\000-\037\177')
  s=$(printf '%s' "$s" | tr -s ' ' | sed 's/^ *//; s/ *$//' | cut -c1-64)
  s=$(printf '%s' "$s" | sed 's/ *$//')
  slug=$(printf '%s' "$s" | tr -c 'A-Za-z0-9._-' '-' | tr '[:upper:]' '[:lower:]' | tr -s '.-')
  slug=$(printf '%s' "$slug" | sed 's/^[-._]*//; s/[-._]*$//' | cut -c1-64 | sed 's/[-._]*$//')
  printf '%s\t%s\n' "$slug" "$s"
}

emit() {  # $1 slug, $2 display
  case "$MODE" in
    slug)    printf '%s\n' "$1" ;;
    display) printf '%s\n' "$2" ;;
    both)    printf '%s%s%s\n' "$1" "$TAB" "$2" ;;
  esac
  exit 0
}

# Accept a raw candidate; emit and exit if it sanitises to a usable slug.
consider() {
  [ -n "${1:-}" ] || return 1
  local pair slug display
  pair=$(sanitize "$1") || return 1
  slug=${pair%%"$TAB"*}
  display=${pair#*"$TAB"}
  [ -n "$slug" ] || return 1
  emit "$slug" "$display"
}

if [ "$FROM_STDIN" = 1 ]; then
  consider "$(cat)"
  emit "" ""       # sanitised to nothing — the caller decides what that means
fi

# --- tier 1: the /rename title from this session's transcript ----------------
PY_TITLE='
import json, sys

name = ""
with open(sys.argv[1], "rb") as fh:
    for line in fh:
        # Cheap byte pre-filter: transcripts run to megabytes and only a few
        # lines carry a title, so do not pay for json.loads on every line.
        if b"customTitle" not in line and b"agentName" not in line:
            continue
        try:
            rec = json.loads(line.decode("utf-8", "replace"))
        except Exception:
            continue
        if not isinstance(rec, dict):
            continue
        for key in ("customTitle", "agentName"):
            val = rec.get(key)
            if isinstance(val, str) and val.strip():
                name = val        # last occurrence wins: /rename can be re-run
                break
sys.stdout.write(name)
'

transcript_title() {
  local sid="${CLAUDE_CODE_SESSION_ID:-}"
  [ -n "$sid" ] || return 1
  [ "$HAVE_PYTHON" = 1 ] || return 1
  # The id becomes a path component and a find pattern; refuse anything odd.
  case "$sid" in
    ''|*[!a-zA-Z0-9._-]*) return 1 ;;
  esac

  local root="$HOME/.claude/projects"
  [ -d "$root" ] || return 1

  local candidates proj t title
  proj=$(pwd -P 2>/dev/null | tr '/.' '--')
  candidates="$root/$proj/$sid.jsonl"
  # The session may have changed directory since it started, in which case the
  # cwd-derived project directory is not where its transcript lives.
  candidates="$candidates
$(find "$root" -maxdepth 2 -type f -name "$sid.jsonl" 2>/dev/null)"

  printf '%s\n' "$candidates" | while IFS= read -r t; do
    [ -n "$t" ] && [ -f "$t" ] || continue
    title=$(python3 -c "$PY_TITLE" "$t" 2>/dev/null) || continue
    [ -n "$title" ] || continue
    printf '%s' "$title"
    break
  done
}

consider "$(transcript_title)"

# --- tier 2: the git worktree directory --------------------------------------
consider "$(basename "$(git rev-parse --show-toplevel 2>/dev/null)" 2>/dev/null)"

# --- tier 3: the session id --------------------------------------------------
SID="$(printf '%s' "${CLAUDE_CODE_SESSION_ID:-}" | tr -cd 'a-zA-Z0-9' | cut -c1-8)"
[ -n "$SID" ] && consider "session-$SID"

# --- tier 4: the working directory, hashed -----------------------------------
# Stable across runs and distinct per directory, so two nameless sessions in
# different trees still get different identities.
if [ "$HAVE_PYTHON" = 1 ]; then
  consider "session-$(pwd -P 2>/dev/null \
    | python3 -c 'import hashlib,sys;sys.stdout.write(hashlib.sha256(sys.stdin.buffer.read()).hexdigest()[:8])' 2>/dev/null)"
fi
consider "session-$(pwd -P 2>/dev/null | cksum | cut -d' ' -f1 | cut -c1-8)"

# Unreachable in practice: cksum and cut are POSIX. Kept so the contract
# "never prints an empty name" holds even if it is.
emit "buzz-session" "buzz-session"
