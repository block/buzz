#!/usr/bin/env bash
# grab-emoji.sh — Download custom Slack emoji and upload to Buzz
#
# Usage:
#   SLACK_TOKEN=xoxp-... ./scripts/grab-emoji.sh shipitparrot partyparrot tada
#
# Env:
#   SLACK_TOKEN  — Slack user token (xoxp-...) with emoji:read scope
#
# Output:
#   name → buzz_url      on success
#   name → ERROR: reason on failure (script continues to next emoji)

set -euo pipefail

CACHE_FILE="${HOME}/.cache/slack-emoji-list.json"
CACHE_TTL=86400  # 24 hours in seconds

# ── Preflight checks ──────────────────────────────────────────────────────────

if [[ $# -eq 0 ]]; then
  echo "Usage: SLACK_TOKEN=xoxp-... $0 <emoji-name> [emoji-name ...]" >&2
  exit 1
fi

if [[ -z "${SLACK_TOKEN:-}" ]]; then
  echo "ERROR: SLACK_TOKEN is not set. Export your xoxp- Slack token." >&2
  exit 1
fi

if ! command -v buzz &>/dev/null; then
  echo "ERROR: 'buzz' not found in PATH. Install the Buzz CLI and retry." >&2
  exit 1
fi

if ! command -v curl &>/dev/null; then
  echo "ERROR: 'curl' not found in PATH." >&2
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "ERROR: 'jq' not found in PATH." >&2
  exit 1
fi

# ── Cache management ──────────────────────────────────────────────────────────

_cache_is_fresh() {
  [[ -f "$CACHE_FILE" ]] || return 1
  local mtime now age
  # macOS stat uses -f %m; GNU stat uses -c %Y
  if stat --version &>/dev/null 2>&1; then
    mtime=$(stat -c %Y "$CACHE_FILE")
  else
    mtime=$(stat -f %m "$CACHE_FILE")
  fi
  now=$(date +%s)
  age=$(( now - mtime ))
  (( age < CACHE_TTL ))
}

_refresh_cache() {
  mkdir -p "$(dirname "$CACHE_FILE")"
  local response
  response=$(curl -sf \
    -H "Authorization: Bearer ${SLACK_TOKEN}" \
    "https://slack.com/api/emoji.list") || {
    echo "ERROR: network failure fetching emoji list from Slack" >&2
    return 1
  }

  local ok
  ok=$(echo "$response" | jq -r '.ok')
  if [[ "$ok" != "true" ]]; then
    local err
    err=$(echo "$response" | jq -r '.error // "unknown error"')
    echo "ERROR: Slack API returned error: $err" >&2
    return 1
  fi

  echo "$response" > "$CACHE_FILE"
}

if ! _cache_is_fresh; then
  _refresh_cache || exit 1
fi

# ── Emoji resolution ──────────────────────────────────────────────────────────

# Returns the URL for an emoji name, resolving one level of aliasing.
# Prints the URL to stdout; returns 1 if not found.
_resolve_url() {
  local name="$1"
  local value
  value=$(jq -r --arg n "$name" '.emoji[$n] // empty' "$CACHE_FILE")

  if [[ -z "$value" ]]; then
    return 1
  fi

  if [[ "$value" == alias:* ]]; then
    local target="${value#alias:}"
    value=$(jq -r --arg n "$target" '.emoji[$n] // empty' "$CACHE_FILE")
    if [[ -z "$value" ]]; then
      return 1
    fi
  fi

  echo "$value"
}

# ── Per-emoji processing ──────────────────────────────────────────────────────

for emoji_name in "$@"; do
  # Resolve URL
  emoji_url=$(_resolve_url "$emoji_name") || {
    echo "${emoji_name} → ERROR: emoji not found in workspace"
    continue
  }

  # Determine extension from URL (default to .png if none found)
  ext="${emoji_url##*.}"
  # Strip any query string from extension
  ext="${ext%%\?*}"
  case "$ext" in
    png|gif|jpg|jpeg|webp) ;;
    *) ext="png" ;;
  esac

  # Download to temp file
  tmp_file=$(mktemp "/tmp/slack-emoji-XXXXXX.${ext}")
  trap 'rm -f "$tmp_file"' EXIT

  if ! curl -sf -o "$tmp_file" "$emoji_url"; then
    echo "${emoji_name} → ERROR: failed to download emoji from ${emoji_url}"
    rm -f "$tmp_file"
    trap - EXIT
    continue
  fi

  # Upload to Buzz
  upload_output=$(buzz upload file "$tmp_file" 2>&1) || {
    echo "${emoji_name} → ERROR: buzz upload failed — ${upload_output}"
    rm -f "$tmp_file"
    trap - EXIT
    continue
  }

  # Extract URL from BlobDescriptor output
  # buzz upload file prints a multi-line BlobDescriptor; the url field is first
  buzz_url=$(echo "$upload_output" | grep -E '^url:' | awk '{print $2}' | head -1)
  if [[ -z "$buzz_url" ]]; then
    # Fallback: try JSON parse if output is JSON
    buzz_url=$(echo "$upload_output" | jq -r '.url // empty' 2>/dev/null || true)
  fi

  if [[ -z "$buzz_url" ]]; then
    echo "${emoji_name} → ERROR: could not parse Buzz URL from upload output"
  else
    echo "${emoji_name} → ${buzz_url}"
  fi

  rm -f "$tmp_file"
  trap - EXIT
done
