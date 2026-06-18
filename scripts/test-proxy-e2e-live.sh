#!/usr/bin/env bash
# Live e2e test for buzz-proxy — requires relay on :3000 and proxy on :4869
set -euo pipefail

PROXY_URL="ws://0.0.0.0:4869"
PROXY_HTTP="http://localhost:4869"
ADMIN_SECRET="test-admin-secret"

PASS=0
FAIL=0

ok()   { PASS=$((PASS+1)); echo "  ✅ $1"; }
fail() { FAIL=$((FAIL+1)); echo "  ❌ $1"; }

echo "═══ buzz-proxy e2e tests ═══"
echo ""

# ── Test 1: NIP-11 ──────────────────────────────────────────────────────
echo "1. NIP-11 relay info"
NIP11=$(curl -sf "$PROXY_HTTP" -H "Accept: application/nostr+json")
if echo "$NIP11" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d['name']=='buzz-proxy'" 2>/dev/null; then
    ok "NIP-11 returns valid relay info"
else
    fail "NIP-11 check failed"
fi

# ── Test 2: Register guest ──────────────────────────────────────────────
echo "2. Guest registration (pubkey-based)"
GUEST_SK=$(nak key generate)
GUEST_PK=$(nak key public "$GUEST_SK")
CHANNEL_UUID="6f953bb4-f761-4bc2-98de-392470c2b897"

REG=$(curl -sf -X POST "$PROXY_HTTP/admin/guests" \
    -H "Authorization: Bearer $ADMIN_SECRET" \
    -H "Content-Type: application/json" \
    -d "{\"pubkey\": \"$GUEST_PK\", \"channels\": \"$CHANNEL_UUID\"}")
if echo "$REG" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d['pubkey']=='$GUEST_PK'" 2>/dev/null; then
    ok "Guest registered: $GUEST_PK"
else
    fail "Guest registration failed: $REG"
fi

# ── Test 3: List guests ─────────────────────────────────────────────────
echo "3. List guests"
LIST=$(curl -sf "$PROXY_HTTP/admin/guests" -H "Authorization: Bearer $ADMIN_SECRET")
if echo "$LIST" | python3 -c "import sys,json; d=json.load(sys.stdin); assert len(d['guests'])>=1" 2>/dev/null; then
    ok "Guest list returns registered guests"
else
    fail "Guest list failed: $LIST"
fi

# ── Test 4: Admin auth required ─────────────────────────────────────────
echo "4. Admin auth enforcement"
NOAUTH=$(curl -s -o /dev/null -w "%{http_code}" "$PROXY_HTTP/admin/guests")
if [ "$NOAUTH" = "401" ]; then
    ok "Admin endpoint rejects unauthenticated requests"
else
    fail "Expected 401, got $NOAUTH"
fi

# ── Test 5: WebSocket AUTH challenge ────────────────────────────────────
echo "5. WebSocket NIP-42 challenge"
CHALLENGE=$(echo "" | timeout 3 websocat -t -1 "$PROXY_URL" 2>/dev/null | head -1)
if echo "$CHALLENGE" | python3 -c "import sys,json; d=json.loads(sys.stdin.read()); assert d[0]=='AUTH'" 2>/dev/null; then
    ok "Proxy sends NIP-42 AUTH challenge on connect"
else
    fail "No AUTH challenge received: $CHALLENGE"
fi

# ── Test 6: Full NIP-42 handshake (pubkey-based, no token) ─────────────
echo "6. Full NIP-42 auth handshake (pubkey-based)"

# Use a Python script to do the full handshake on a single WebSocket connection.
RESPONSE=$(uv run python3 -c "
import asyncio, json, subprocess, sys

async def test():
    import websockets
    async with websockets.connect('$PROXY_URL') as ws:
        # 1. Receive AUTH challenge
        msg = json.loads(await ws.recv())
        assert msg[0] == 'AUTH', f'Expected AUTH, got {msg[0]}'
        challenge = msg[1]

        # 2. Create signed auth event with nak
        auth_json = subprocess.check_output([
            'nak', 'event', '--sec', '$GUEST_SK', '-k', '22242',
            '--tag', 'relay=$PROXY_URL',
            '--tag', f'challenge={challenge}',
            '-c', ''
        ], text=True).strip()

        # 3. Send AUTH response
        await ws.send(json.dumps(['AUTH', json.loads(auth_json)]))

        # 4. Read OK
        ok_msg = json.loads(await ws.recv())
        print(json.dumps(ok_msg))

asyncio.run(test())
" 2>&1) || true

echo "  Response: $RESPONSE"

if echo "$RESPONSE" | python3 -c "import sys,json; d=json.loads(sys.stdin.read()); assert d[0]=='OK' and d[2]==True" 2>/dev/null; then
    ok "NIP-42 auth succeeded (pubkey-based, no token)"
else
    # Fall back: try without websockets library (use websocat with named pipe)
    fail "NIP-42 auth handshake: $RESPONSE"
fi

# ── Test 7: Create invite token ─────────────────────────────────────────
echo "7. Invite token creation"
INVITE=$(curl -sf -X POST "$PROXY_HTTP/admin/invite" \
    -H "Authorization: Bearer $ADMIN_SECRET" \
    -H "Content-Type: application/json" \
    -d "{\"channels\": \"$CHANNEL_UUID\", \"hours\": 1, \"max_uses\": 5}")
TOKEN=$(echo "$INVITE" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)
if [ -n "$TOKEN" ]; then
    ok "Invite token created: ${TOKEN:0:30}..."
else
    fail "Invite token creation failed: $INVITE"
fi

# ── Test 8: Revoke guest ────────────────────────────────────────────────
echo "8. Guest revocation"
REVOKE=$(curl -sf -X DELETE "$PROXY_HTTP/admin/guests" \
    -H "Authorization: Bearer $ADMIN_SECRET" \
    -H "Content-Type: application/json" \
    -d "{\"pubkey\": \"$GUEST_PK\"}")
if echo "$REVOKE" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d['revoked']==True" 2>/dev/null; then
    ok "Guest revoked"
else
    fail "Guest revocation failed: $REVOKE"
fi

# ── Summary ─────────────────────────────────────────────────────────────
echo ""
echo "═══ Results: $PASS passed, $FAIL failed ═══"
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
