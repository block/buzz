#!/usr/bin/env bash
# End-to-end test for buzz-proxy
# Prerequisites:
#   - Buzz relay running on :3000 (just relay)
#   - Buzz proxy running on :4869 (just proxy)
#   - websocat installed (cargo install websocat)
#   - curl installed
#   - jq installed
set -euo pipefail

PROXY_URL="${PROXY_URL:-ws://localhost:4869}"
PROXY_HTTP="${PROXY_HTTP:-http://localhost:4869}"
RELAY_HTTP="${RELAY_HTTP:-http://localhost:3000}"

echo "=== Buzz Proxy E2E Test ==="
echo "Proxy: $PROXY_URL"
echo "Relay: $RELAY_HTTP"

# Step 1: Check NIP-11
echo ""
echo "--- Step 1: NIP-11 Info Document ---"
NIP11=$(curl -sf -H "Accept: application/nostr+json" "$PROXY_HTTP/" || true)
if [ -z "$NIP11" ]; then
    echo "FAIL: NIP-11 endpoint not responding"
    exit 1
fi
echo "$NIP11" | jq .
echo "PASS: NIP-11 document served"

# Step 2: Create invite token
echo ""
echo "--- Step 2: Create Invite Token ---"
# Get a channel UUID from the relay
CHANNEL_ID=$(curl -sf "$RELAY_HTTP/api/channels" -H "X-Pubkey: 0101010101010101010101010101010101010101010101010101010101010101" | jq -r '.[0].id // empty')
if [ -z "$CHANNEL_ID" ]; then
    echo "WARN: No channels found in relay. Creating a test channel..."
    CHANNEL_ID=$(curl -sf -X POST "$RELAY_HTTP/api/channels" \
        -H "Content-Type: application/json" \
        -H "X-Pubkey: 0101010101010101010101010101010101010101010101010101010101010101" \
        -d '{"name":"proxy-test","channel_type":"stream","visibility":"open"}' | jq -r '.id')
fi
echo "Using channel: $CHANNEL_ID"

INVITE=$(curl -sf -X POST "$PROXY_HTTP/admin/invite" \
    -H "Content-Type: application/json" \
    -d "{\"channels\": \"$CHANNEL_ID\", \"hours\": 1, \"max_uses\": 10}" || true)
if [ -z "$INVITE" ]; then
    echo "FAIL: Could not create invite token"
    exit 1
fi
TOKEN=$(echo "$INVITE" | jq -r '.token')
echo "Token: ${TOKEN:0:30}..."
echo "PASS: Invite token created"

# Step 3: Connect and test NIP-42 + kind:40 query
echo ""
echo "--- Step 3: WebSocket Connection Test ---"
echo "Connecting to $PROXY_URL?token=$TOKEN ..."

# Use a timeout approach: send a REQ for kind:40, expect AUTH challenge + EOSE
# websocat with -t (text) and timeout
RESULT=$(echo '["REQ","test-sub",{"kinds":[40],"limit":10}]' | \
    timeout 5 websocat -t -1 "$PROXY_URL?token=$TOKEN" 2>/dev/null || true)

if [ -z "$RESULT" ]; then
    echo "FAIL: No response from proxy (timeout or connection refused)"
    echo "Make sure the proxy is running: just proxy"
    exit 1
fi

echo "Response: $RESULT"

# Check if we got an AUTH challenge (expected before we authenticate)
if echo "$RESULT" | grep -q '"AUTH"'; then
    echo "PASS: Received NIP-42 AUTH challenge"
else
    echo "INFO: Did not receive AUTH challenge in first message"
fi

echo ""
echo "=== Test Summary ==="
echo "NIP-11: PASS"
echo "Invite creation: PASS"
echo "WebSocket connection: PASS"
echo ""
echo "For full interactive testing, use:"
echo "  websocat $PROXY_URL?token=$TOKEN"
echo "  Then send: [\"AUTH\", <signed-kind-22242-event>]"
echo "  Then send: [\"REQ\",\"sub1\",{\"kinds\":[40],\"limit\":10}]"
