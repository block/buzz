#!/bin/bash
# Startup script for Antigravity ACP Harness + buzz-acp bridge

export BUZZ_RELAY_URL="${BUZZ_RELAY_URL:-ws://192.168.1.80:3001}"
export BUZZ_PRIVATE_KEY="${BUZZ_PRIVATE_KEY:-c12b840d5b3bcd6526b1f6aa5464c54410591db62933e425a2ef98627bbee7e8}"
export GEMINI_API_KEY="${GEMINI_API_KEY:-}"
export HARNESS_STUB_RESPONSE="${HARNESS_STUB_RESPONSE:-}"

mkdir -p /tmp/buzz-acp-logs

echo "Starting Antigravity ACP bridge..."
echo "Relay: $BUZZ_RELAY_URL"
echo "Public Key: 8d703e67060b90bb06c45e713bcd23aa12bcd25edcfb374d8d870090ded22bb9"

exec /home/bntt/buzz/target/release/buzz-acp \
  --relay-url "$BUZZ_RELAY_URL" \
  --private-key "$BUZZ_PRIVATE_KEY" \
  --agent-command python3 \
  --agent-args /home/bntt/buzz-acp-antigravity/buzz_acp_antigravity.py \
  --respond-to anyone \
  --subscribe mentions
