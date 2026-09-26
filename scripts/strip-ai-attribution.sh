#!/usr/bin/env bash
# Remove AI-assistant attribution from a commit message in place.
#
# Buzz does not carry AI attribution in its history. The rule was enforced only
# at review time, which caught it late: a reviewer had to notice the trailer,
# and the author then had to rewrite already-pushed history to remove it. This
# strips it at the point it is introduced instead.
#
# What it removes:
#   - `Co-authored-by:` trailers naming an AI assistant or an assistant's
#     noreply address.
#   - `Generated with <tool>` footers, with or without a markdown link.
#
# What it deliberately leaves alone:
#   - `Co-authored-by:` trailers naming a human. Pair-programming attribution is
#     legitimate and this must never eat it.
#   - `Signed-off-by:` trailers. DCO is required; the sibling `signoff` hook
#     command adds one, and dropping it would fail the DCO Check.
#
# Usage: strip-ai-attribution.sh <commit-msg-file>
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <commit-msg-file>" >&2
  exit 2
fi

file="$1"
[ -f "$file" ] || exit 0

# Matched case-insensitively: Git trailer keys are case-insensitive in practice
# and tools emit both `Co-Authored-By` and `Co-authored-by`.
#
# The assistant patterns are matched on the *identity*, not on the word "AI":
# a human legitimately named e.g. "Claudia" must not be stripped, so the
# patterns require either a known assistant name in the name position or a
# known assistant noreply address.
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

sed -E \
  -e '/^[[:space:]]*co-authored-by:[[:space:]]*(claude|chatgpt|copilot|cursor|codex|devin|gemini)\b/Id' \
  -e '/^[[:space:]]*co-authored-by:.*(noreply@anthropic\.com|users\.noreply\.github\.com\/copilot|noreply@openai\.com)/Id' \
  -e '/^[[:space:]]*(🤖[[:space:]]*)?generated with[[:space:]]/Id' \
  "$file" >"$tmp"

# Collapse the blank-line runs the deletions leave behind, then drop leading and
# trailing blanks. This is the same normalization Git's own default message
# cleanup performs, applied here so the result is identical no matter which
# order a Git version runs cleanup and this hook in.
awk '
  /^[[:space:]]*$/ { blank = 1; next }
  { if (blank && seen) print ""; blank = 0; seen = 1; print }
' "$tmp" >"$file"
