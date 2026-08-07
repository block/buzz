#!/usr/bin/env bash
# Build fork buzz-acp (thread participation) and install outside the signed app.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -f ./bin/activate-hermit ]]; then
  # shellcheck disable=SC1091
  . ./bin/activate-hermit
fi

APP_SUPPORT="${HOME}/Library/Application Support/xyz.block.buzz.app"
BIN_DIR="${APP_SUPPORT}/agents/bin"
AGENTS_JSON="${APP_SUPPORT}/agents/managed-agents.json"
DEST="${BIN_DIR}/buzz-acp"

echo "==> cargo build --release -p buzz-acp"
cargo build --release -p buzz-acp

mkdir -p "$BIN_DIR"
cp -f "${ROOT}/target/release/buzz-acp" "$DEST"
chmod +x "$DEST"
codesign --force --sign - --timestamp=none "$DEST" 2>/dev/null || true

if ! "$DEST" --help 2>&1 | grep -q 'thread-participation'; then
  echo "error: installed binary lacks --thread-participation" >&2
  exit 1
fi

echo "==> installed $DEST"
ls -lh "$DEST"

# Point active relay-backed agents at this binary (absolute path).
if [[ -f "$AGENTS_JSON" ]]; then
  python3 - "$AGENTS_JSON" "$DEST" <<'PY'
import json, sys
from pathlib import Path
path, acp = Path(sys.argv[1]), sys.argv[2]
agents = json.loads(path.read_text())
bak = path.with_suffix(".json.bak-pre-participation-acp")
if not bak.exists():
    bak.write_text(json.dumps(agents, indent=2) + "\n")
n = 0
for a in agents:
    if a.get("relay_url"):
        if a.get("acp_command") != acp:
            a["acp_command"] = acp
            n += 1
path.write_text(json.dumps(agents, indent=2) + "\n")
pref = path.parent / "thread-participation.json"
pref.write_text(json.dumps({"enabled": True}) + "\n")
print(f"updated acp_command on {n} agent record(s); pref enabled")
PY
else
  echo "warn: no managed-agents.json at $AGENTS_JSON — start Buzz once, re-run"
fi

echo ""
echo "Done. Restart Buzz (quit + open) so agents spawn this harness."
echo "Look for: thread_participation=true in agent logs."
