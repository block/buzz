#!/usr/bin/env bash
# Apply launchpad/labels.yml to the repository.
#
#   ./launchpad/sync-labels.sh              # apply to launchpad-26/buzz
#   ./launchpad/sync-labels.sh --dry-run    # show what would change
#   REPO=owner/name ./launchpad/sync-labels.sh
#
# Creates labels that are missing and updates colour/description on ones that
# exist. Does NOT delete labels that are absent from labels.yml — deleting a
# label silently strips it from every issue carrying it, so that stays manual.
set -euo pipefail

REPO="${REPO:-launchpad-26/buzz}"
DRY_RUN=""
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

FILE="$(dirname "$0")/labels.yml"
[ -f "$FILE" ] || { echo "not found: $FILE" >&2; exit 1; }

command -v gh >/dev/null || { echo "gh is required" >&2; exit 1; }

# Parse the flat "- name:/color:/description:" list without a YAML dependency.
python3 - "$FILE" <<'PY' | while IFS=$'\t' read -r name color desc; do
import re, sys
text = open(sys.argv[1]).read()
for block in re.findall(
        r'^- name:\s*"([^"]+)"\s*\n\s*color:\s*"([^"]+)"\s*\n\s*description:\s*"([^"]*)"',
        text, flags=re.M):
    print("\t".join(block))
PY
  if [ -n "$DRY_RUN" ]; then
    echo "would sync: $name ($color) — $desc"
    continue
  fi
  if gh label create "$name" --repo "$REPO" --color "$color" --description "$desc" 2>/dev/null; then
    echo "created: $name"
  else
    gh label edit "$name" --repo "$REPO" --color "$color" --description "$desc" >/dev/null
    echo "updated: $name"
  fi
done

echo
echo "Done. Labels absent from labels.yml were left alone — remove those by hand."
