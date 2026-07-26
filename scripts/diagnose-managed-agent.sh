#!/usr/bin/env bash
# diagnose-managed-agent.sh — why is my Buzz desktop-managed agent silent?
#
# Usage: ./scripts/diagnose-managed-agent.sh <agent-pubkey-hex> [relay-base-url]
#   agent-pubkey-hex : the agent's 64-char hex pubkey (Subby's), NOT npub
#   relay-base-url   : e.g. https://flint.communities.buzz.xyz (default: workspace relay)
#
# Checks, in order: local record -> harness log telltales -> relay membership/mention.
set -euo pipefail

PUBKEY="${1:?usage: $0 <agent-pubkey-hex> [relay-base-url]}"
RELAY="${2:-}"

APP_SUPPORT="${HOME}/Library/Application Support/xyz.block.buzz.app"
RECORD_FILE="${APP_SUPPORT}/managed-agents.json"

echo "================ BUZZ AGENT DIAGNOSTIC ================"
echo "agent pubkey : ${PUBKEY}"
echo

# 1. Local record — respond_to + auth_tag presence drive the owner gate.
echo "---- [1] local managed-agents record ----"
if [[ -f "${RECORD_FILE}" ]]; then
  if command -v jq >/dev/null 2>&1; then
    jq --arg p "${PUBKEY}" '.[] | select(.pubkey == $p)
        | {name, pubkey, respond_to, has_auth_tag: (.auth_tag != null),
           relay_url, model, provider}' "${RECORD_FILE}" 2>/dev/null       || echo "  (pubkey not found in ${RECORD_FILE})"
  else
    echo "  jq not installed; grep'ing for the pubkey:"
    grep -o ""pubkey":"[^"]*"\|"respond_to":"[^"]*"" "${RECORD_FILE}" | head
  fi
else
  echo "  no managed-agents.json at ${RECORD_FILE} (wrong bundle id? not macOS?)"
fi
echo

# 2. Harness log telltales — these lines explain a silent agent.
echo "---- [2] harness log telltales (searching app support + Logs) ----"
TELLTALES='respond-to=owner-only but no owner|BUZZ_AUTH_TAG verification failed|setup-listener mode|entering setup mode|circuit open|inbound author gate|matched no rule'
found=0
while IFS= read -r -d '' f; do
  if grep -qE "${PUBKEY}" "${f}" 2>/dev/null      || grep -qiE 'buzz-acp|managed.agent' "${f}" 2>/dev/null; then
    hits=$(grep -hE "${TELLTALES}" "${f}" 2>/dev/null | tail -n 8)
    if [[ -n "${hits}" ]]; then
      found=1
      echo "  >> ${f}"
      echo "${hits}" | sed 's/^/      /'
    fi
  fi
done < <(find "${APP_SUPPORT}" "${HOME}/Library/Logs" -type f \( -name '*.log' -o -name '*.txt' \) -print0 2>/dev/null)
[[ "${found}" == "0" ]] && echo "  (no telltale lines found — logs may be elsewhere or agent never started)"
echo

# 3. Relay-side checks.
echo "---- [3] relay checks ----"
if [[ -z "${RELAY}" ]]; then
  echo "  (skip: pass RELAY-BASE-URL as 2nd arg, e.g. https://flint.communities.buzz.xyz)"
else
  echo "  >> is the agent a MEMBER of the channel? (kinds 39002 = membership roster)"
  echo "     curl -s ${RELAY}/query -d '{"kinds":[39002],"#p":["${PUBKEY}"]}' | jq '.[].tags'"
  echo "  >> does your @mention carry the agent's exact hex p-tag? (kinds 9=stream msg, 40002=v2)"
  echo "     curl -s ${RELAY}/query -d '{"kinds":[9,40002],"#p":["${PUBKEY}"],"limit":5}' | jq '.[]|{kind,p_tags:[.tags[]|select(.[0]=="p")[1]]}'"
fi
echo
echo "================= DECISION KEY ===================="
echo "respond_to=owner-only + has_auth_tag=false  -> owner likely unresolvable: set respond-to=anyone"
echo "respond_to=owner-only + 'no owner is set'    -> the silent-drop-everything bug; restart after fix"
echo "'BUZZ_AUTH_TAG verification failed'          -> stale tag (owner re-keyed): re-create the agent"
echo "'setup-listener mode'                        -> agent not ready (missing API key / CLI login)"
echo "'circuit open'                               -> agent subprocess crash-looping (bad model/provider)"
echo "'inbound author gate — dropping'             -> you are not the resolved owner (owner-only gate)"
echo "'matched no rule'                            -> mention p-tag/kind mismatch vs subscription filter"
