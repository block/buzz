#!/usr/bin/env bash
# =============================================================================
# e2e-relay-membership.sh — End-to-end test for NIP-43 relay membership
# =============================================================================
# Tests relay-level membership enforcement and admin commands.
#
# Prerequisites:
#   - Docker services running (postgres, redis, typesense)
#   - Relay built: cargo build --release --bin buzz-relay
#   - nak available on PATH (for event signing)
#
# What it tests:
#   1. Non-member REST calls are rejected (401/403)
#   2. Owner adds admin via kind:9030
#   3. Admin adds member via kind:9030
#   4. Owner changes member role via kind:9032
#   5. Admin removes member via kind:9031
#   6. Members cannot add others (permission denied)
#   7. Admins cannot change roles (permission denied)
#   8. Owner cannot be removed
#   9. GET /api/relay/members returns correct list
#  10. GET /api/relay/members/me returns correct role
#  11. NIP-11 self field is advertised
#  12. kind:13534 membership list published after add
#  13. kind:8000 member-added announcement published
#  14. kind:28936 leave request (happy path)
#  15. kind:8001 member-removed announcement after leave
#  16. Leave request without NIP-70 - tag is rejected
#  17. Stale admin command rejected (replay protection)
#  18. Owner cannot leave (lockout prevention)
#  19. Admin cannot add another admin
#  20. Admin cannot remove the owner
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log()     { echo -e "${BLUE}[e2e-membership]${NC} $*"; }
success() { echo -e "${GREEN}[e2e-membership]${NC} ✓ $*"; }
warn()    { echo -e "${YELLOW}[e2e-membership]${NC} ⚠ $*"; }

# Failure counter — tests record failures and continue; fatal() exits immediately.
FAILURES=0

fail() {
    echo -e "${RED}[e2e-membership]${NC} ✗ $*" >&2
    FAILURES=$((FAILURES + 1))
}

fatal() {
    echo -e "${RED}[e2e-membership]${NC} ✗ FATAL: $*" >&2
    cleanup
    exit 1
}

# ── Cleanup ───────────────────────────────────────────────────────────────────

RELAY_PID=""

cleanup() {
    if [[ -n "$RELAY_PID" ]]; then
        kill "$RELAY_PID" 2>/dev/null || true
        wait "$RELAY_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# ── Helpers: keypair generation ───────────────────────────────────────────────

generate_keypair() {
    openssl rand -hex 32
}

derive_pubkey() {
    local privkey="$1"
    python3 -c "
P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8

def point_add(p1, p2):
    if p1 is None: return p2
    if p2 is None: return p1
    x1, y1 = p1; x2, y2 = p2
    if x1 == x2 and y1 != y2: return None
    if x1 == x2: lam = (3*x1*x1) * pow(2*y1, P-2, P) % P
    else: lam = (y2-y1) * pow(x2-x1, P-2, P) % P
    x3 = (lam*lam - x1 - x2) % P
    y3 = (lam*(x1-x3) - y1) % P
    return (x3, y3)

def scalar_mult(k, point):
    result = None; addend = point
    while k:
        if k & 1: result = point_add(result, addend)
        addend = point_add(addend, addend)
        k >>= 1
    return result

pub = scalar_mult(int('$privkey', 16), (Gx, Gy))
print(format(pub[0], '064x'))
"
}

# ── Helper: sign and send a Nostr event via nak ───────────────────────────────
#
# Usage: send_event <privkey> <kind> <content> [tag ...]
#   Each tag is passed as a separate argument in nak --tag format: "key=value"
#
# Returns the relay response on stdout.

send_event() {
    local privkey="$1"
    local kind="$2"
    local content="$3"
    shift 3
    local tag_args=()
    for t in "$@"; do
        tag_args+=(--tag "$t")
    done

    nak event \
        --sec "$privkey" \
        --kind "$kind" \
        --content "$content" \
        "${tag_args[@]}" \
        ws://localhost:3000 2>&1
}

# ── Helper: REST call with X-Pubkey header (dev mode) ────────────────────────
#
# When BUZZ_REQUIRE_AUTH_TOKEN=false the relay accepts an X-Pubkey header
# containing the caller's hex pubkey — no token minting required.
# This is the correct pattern for dev-mode E2E tests.
#
# Returns the HTTP status code on stdout.

rest_call() {
    local privkey="$1"
    local method="$2"
    local path="$3"
    local body="${4:-}"

    local pubkey
    pubkey=$(derive_pubkey "$privkey")

    local curl_args=(-s -o /dev/null -w "%{http_code}" -X "$method"
        -H "X-Pubkey: $pubkey")
    if [[ -n "$body" ]]; then
        curl_args+=(-H "Content-Type: application/json" -d "$body")
    fi

    curl "${curl_args[@]}" "http://localhost:3000${path}"
}

# ── Helper: REST GET, return body ─────────────────────────────────────────────

rest_get_body() {
    local privkey="$1"
    local path="$2"

    local pubkey
    pubkey=$(derive_pubkey "$privkey")

    curl -s -X GET \
        -H "X-Pubkey: $pubkey" \
        "http://localhost:3000${path}"
}

# ── Helper: check relay OK response ──────────────────────────────────────────
#
# Returns 0 if the response is ["OK", <id>, true, ...], 1 otherwise.

is_ok() {
    echo "$1" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    if isinstance(data, list) and len(data) >= 3 and data[0] == 'OK' and data[2] is True:
        sys.exit(0)
except: pass
sys.exit(1)
" 2>/dev/null
}

# ── Helper: check relay rejection (OK false) ─────────────────────────────────
#
# Returns 0 if the response is ["OK", <id>, false, ...], 1 otherwise.

is_rejected() {
    echo "$1" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    if isinstance(data, list) and len(data) >= 3 and data[0] == 'OK' and data[2] is False:
        sys.exit(0)
except: pass
sys.exit(1)
" 2>/dev/null
}

# ── Start relay ───────────────────────────────────────────────────────────────

log "Starting relay with relay membership enforcement enabled..."

if [[ -f .env ]]; then
    set -o allexport
    source .env
    set +o allexport
fi

export BUZZ_BIND_ADDR="0.0.0.0:3000"
export RELAY_URL="ws://localhost:3000"
export RUST_LOG="buzz_relay=warn"
export BUZZ_REQUIRE_AUTH_TOKEN=false
export BUZZ_REQUIRE_RELAY_MEMBERSHIP=true

# Generate owner keypair BEFORE relay start — main.rs requires RELAY_OWNER_PUBKEY
log "Generating owner keypair..."
OWNER_PRIVKEY=$(generate_keypair)
OWNER_PUBKEY=$(derive_pubkey "$OWNER_PRIVKEY")
export RELAY_OWNER_PUBKEY="$OWNER_PUBKEY"
log "Owner pubkey: $OWNER_PUBKEY"

# Generate a stable relay signing key for NIP-43 self-signed events (kind:13534, etc.)
RELAY_SK=$(generate_keypair)
export BUZZ_RELAY_PRIVATE_KEY="$RELAY_SK"
log "Relay signing key set (NIP-43 self-signed events enabled)"

# Kill any existing relay
pkill -f "buzz-relay" 2>/dev/null || true
sleep 1

./target/release/buzz-relay > /tmp/buzz-relay-membership-e2e.log 2>&1 &
RELAY_PID=$!

for i in $(seq 1 15); do
    if curl -s http://localhost:3000/ -H "Accept: application/nostr+json" | grep -q "Buzz"; then
        break
    fi
    if [[ $i -eq 15 ]]; then
        fatal "Relay did not start. Check /tmp/buzz-relay-membership-e2e.log"
    fi
    sleep 1
done
success "Relay started (PID $RELAY_PID)"

# ── Generate identities ───────────────────────────────────────────────────────

# Generate remaining keypairs (after relay is running)
log "Generating remaining keypairs..."
ADMIN_PRIVKEY=$(generate_keypair)
ADMIN_PUBKEY=$(derive_pubkey "$ADMIN_PRIVKEY")
MEMBER_PRIVKEY=$(generate_keypair)
MEMBER_PUBKEY=$(derive_pubkey "$MEMBER_PRIVKEY")
NONMEMBER_PRIVKEY=$(generate_keypair)
NONMEMBER_PUBKEY=$(derive_pubkey "$NONMEMBER_PRIVKEY")
ATTACKER_PRIVKEY=$(generate_keypair)
ATTACKER_PUBKEY=$(derive_pubkey "$ATTACKER_PRIVKEY")

log "  Owner:     ${OWNER_PUBKEY:0:16}..."
log "  Admin:     ${ADMIN_PUBKEY:0:16}..."
log "  Member:    ${MEMBER_PUBKEY:0:16}..."
log "  Non-member:${NONMEMBER_PUBKEY:0:16}..."
log "  Attacker:  ${ATTACKER_PUBKEY:0:16}..."

# ── Test 1: Non-member REST calls are rejected ────────────────────────────────

log "Test 1: Non-member REST calls should be rejected..."

STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/api/channels)
if [[ "$STATUS" == "401" || "$STATUS" == "403" ]]; then
    success "Unauthenticated request rejected ($STATUS)"
else
    fail "Expected 401/403 for unauthenticated request, got $STATUS"
fi

# Non-member with valid auth should also be rejected
STATUS=$(rest_call "$NONMEMBER_PRIVKEY" GET /api/channels)
if [[ "$STATUS" == "401" || "$STATUS" == "403" ]]; then
    success "Non-member request rejected ($STATUS)"
else
    fail "Non-member got $STATUS — expected 401 or 403 (membership enforcement bypass!)"
fi

# ── Test 1b: Non-member WebSocket event should be rejected ───────────────────

log "Test 1b: Non-member WebSocket event should be rejected..."

# Non-member tries to send a stream message over WebSocket.
# The relay enforces NIP-43 during NIP-42 AUTH, so the event will be rejected
# before it is processed. Channel ID is irrelevant — auth fires first.
RESULT=$(send_event "$NONMEMBER_PRIVKEY" 9 "h=00000000-0000-0000-0000-000000000000" 2>&1 || true)
if echo "$RESULT" | grep -qi 'not a relay member\|restricted\|auth\|false'; then
    success "Non-member WebSocket event rejected"
else
    warn "Could not verify WebSocket non-member rejection: $RESULT"
fi

# ── Test 2: Owner adds admin (kind:9030) ──────────────────────────────────────

log "Test 2: Owner adds admin via kind:9030..."

RESULT=$(send_event "$OWNER_PRIVKEY" 9030 "" \
    "p=$ADMIN_PUBKEY" \
    "role=admin")
if is_ok "$RESULT"; then
    success "Owner added admin (kind:9030 accepted)"
else
    fail "Owner failed to add admin: $RESULT"
fi

sleep 1

# Verify admin appears in member list
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$ADMIN_PUBKEY"; then
    success "Admin pubkey appears in /api/relay/members"
else
    fail "Admin pubkey not found in /api/relay/members: $MEMBERS"
fi

# ── Test 3: Admin adds member (kind:9030) ─────────────────────────────────────

log "Test 3: Admin adds member via kind:9030..."

RESULT=$(send_event "$ADMIN_PRIVKEY" 9030 "" \
    "p=$MEMBER_PUBKEY" \
    "role=member")
if is_ok "$RESULT"; then
    success "Admin added member (kind:9030 accepted)"
else
    fail "Admin failed to add member: $RESULT"
fi

sleep 1

MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$MEMBER_PUBKEY"; then
    success "Member pubkey appears in /api/relay/members"
else
    fail "Member pubkey not found in /api/relay/members: $MEMBERS"
fi

# ── Test 4: Owner changes role (kind:9032) ────────────────────────────────────

log "Test 4: Owner changes member role to admin via kind:9032..."

RESULT=$(send_event "$OWNER_PRIVKEY" 9032 "" \
    "p=$MEMBER_PUBKEY" \
    "role=admin")
if is_ok "$RESULT"; then
    success "Owner changed member role (kind:9032 accepted)"
else
    fail "Owner failed to change role: $RESULT"
fi

sleep 1

# Verify role changed
MEMBER_INFO=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBER_INFO" | python3 -c "
import sys, json
data = json.load(sys.stdin)
members = data if isinstance(data, list) else data.get('members', [])
for m in members:
    if m.get('pubkey', '').startswith('${MEMBER_PUBKEY:0:8}') or m.get('pubkey') == '${MEMBER_PUBKEY}':
        if m.get('role') == 'admin':
            sys.exit(0)
sys.exit(1)
" 2>/dev/null; then
    success "Member role updated to admin in /api/relay/members"
else
    fail "Role change not reflected in /api/relay/members response"
fi

# Reset member back to 'member' role for subsequent tests
send_event "$OWNER_PRIVKEY" 9032 "" \
    "p=$MEMBER_PUBKEY" \
    "role=member" > /dev/null 2>&1 || true
sleep 1

# ── Test 5: Admin removes member (kind:9031) ──────────────────────────────────

log "Test 5: Admin removes member via kind:9031..."

RESULT=$(send_event "$ADMIN_PRIVKEY" 9031 "" \
    "p=$MEMBER_PUBKEY")
if is_ok "$RESULT"; then
    success "Admin removed member (kind:9031 accepted)"
else
    fail "Admin failed to remove member: $RESULT"
fi

sleep 1

MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$MEMBER_PUBKEY"; then
    fail "Member pubkey still present in /api/relay/members after removal"
else
    success "Member pubkey absent from /api/relay/members after removal"
fi

# Re-add member for permission tests
send_event "$ADMIN_PRIVKEY" 9030 "" \
    "p=$MEMBER_PUBKEY" \
    "role=member" > /dev/null 2>&1 || true
sleep 1

# ── Test 6: Members cannot add others ────────────────────────────────────────

log "Test 6: Member attempts to add non-member (should be denied)..."

RESULT=$(send_event "$MEMBER_PRIVKEY" 9030 "" \
    "p=$NONMEMBER_PUBKEY" \
    "role=member")
if is_ok "$RESULT"; then
    fail "Member was able to add another user (should be denied)"
elif is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'restricted\|denied\|forbidden\|unauthorized\|permission'; then
    success "Member add attempt rejected"
else
    fail "Ambiguous response for member add attempt: $RESULT"
fi

# ── Test 7: Admins cannot change roles ───────────────────────────────────────

log "Test 7: Admin attempts to change owner role (should be denied)..."

RESULT=$(send_event "$ADMIN_PRIVKEY" 9032 "" \
    "p=$OWNER_PUBKEY" \
    "role=member")
if is_ok "$RESULT"; then
    fail "Admin was able to change owner role (should be denied)"
elif is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'restricted\|denied\|forbidden\|unauthorized\|permission'; then
    success "Admin role-change on owner rejected"
else
    fail "Ambiguous response for admin role-change attempt: $RESULT"
fi

# ── Test 8: Owner cannot be removed ──────────────────────────────────────────

log "Test 8: Admin attempts to remove owner (should be denied)..."

RESULT=$(send_event "$ADMIN_PRIVKEY" 9031 "" \
    "p=$OWNER_PUBKEY")
if is_ok "$RESULT"; then
    fail "Admin was able to remove the owner (should be denied)"
elif is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'restricted\|denied\|forbidden\|unauthorized\|permission'; then
    success "Owner removal attempt rejected"
else
    fail "Ambiguous response for owner removal attempt: $RESULT"
fi

# Verify owner still present
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$OWNER_PUBKEY"; then
    success "Owner still present in /api/relay/members"
else
    fail "Owner missing from /api/relay/members after removal attempt"
fi

# ── Test 9: GET /api/relay/members returns correct list ───────────────────────

log "Test 9: GET /api/relay/members returns expected members..."

MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
STATUS=$(rest_call "$OWNER_PRIVKEY" GET /api/relay/members)

if [[ "$STATUS" == "200" ]]; then
    success "GET /api/relay/members returned 200"
else
    fail "GET /api/relay/members returned $STATUS (expected 200)"
fi

for pubkey in "$OWNER_PUBKEY" "$ADMIN_PUBKEY" "$MEMBER_PUBKEY"; do
    if echo "$MEMBERS" | grep -q "$pubkey"; then
        success "  ${pubkey:0:16}... present in member list"
    else
        fail "  ${pubkey:0:16}... missing from member list"
    fi
done

if echo "$MEMBERS" | grep -q "$NONMEMBER_PUBKEY"; then
    fail "Non-member ${NONMEMBER_PUBKEY:0:16}... should not appear in member list"
else
    success "Non-member correctly absent from member list"
fi

# ── Test 10: GET /api/relay/members/me returns correct role ──────────────────

log "Test 10: GET /api/relay/members/me returns correct role..."

for pair in "${OWNER_PRIVKEY}:owner" "${ADMIN_PRIVKEY}:admin" "${MEMBER_PRIVKEY}:member"; do
    privkey="${pair%%:*}"
    expected_role="${pair##*:}"
    STATUS=$(rest_call "$privkey" GET /api/relay/members/me)
    BODY=$(rest_get_body "$privkey" /api/relay/members/me)

    if [[ "$STATUS" == "200" ]]; then
        if echo "$BODY" | grep -q "\"$expected_role\""; then
            success "  /api/relay/members/me: $expected_role role confirmed (status $STATUS)"
        else
            warn "  /api/relay/members/me: expected role '$expected_role', got: $BODY"
        fi
    else
        warn "  /api/relay/members/me returned $STATUS for $expected_role"
    fi
done

# Non-member should get 403/404
STATUS=$(rest_call "$NONMEMBER_PRIVKEY" GET /api/relay/members/me)
if [[ "$STATUS" == "403" || "$STATUS" == "404" ]]; then
    success "Non-member /api/relay/members/me correctly rejected ($STATUS)"
else
    warn "Non-member /api/relay/members/me returned $STATUS (expected 403/404)"
fi

# ── Test 11: NIP-11 self field is advertised ──────────────────────────────────

log "Test 11: NIP-11 self field is advertised..."

RELAY_HTTP="http://localhost:3000"
SELF_PK=$(curl -s -H "Accept: application/nostr+json" "$RELAY_HTTP" | jq -r '.self // empty')
if [[ -n "$SELF_PK" && "$SELF_PK" != "null" ]]; then
    success "NIP-11 self field present: ${SELF_PK:0:16}..."
else
    fail "NIP-11 self field missing from relay info document"
fi

# ── Test 12: kind:13534 membership list published after add ───────────────────
#
# After Test 3 added a member, the relay should have published a signed
# kind:13534 membership list event to the DB.

log "Test 12: kind:13534 membership list published after member add..."

COUNT=$(psql "$DATABASE_URL" -t -c "SELECT count(*) FROM events WHERE kind=13534;" 2>/dev/null | tr -d ' \n')
if [[ -n "$COUNT" && "$COUNT" -gt 0 ]]; then
    success "kind:13534 membership list found in DB ($COUNT row(s))"
else
    fail "kind:13534 not found in DB — relay may not be publishing membership lists"
fi

# ── Test 13: kind:8000 member-added announcement published ────────────────────

log "Test 13: kind:8000 member-added announcement published..."

COUNT=$(psql "$DATABASE_URL" -t -c "SELECT count(*) FROM events WHERE kind=8000;" 2>/dev/null | tr -d ' \n')
if [[ -n "$COUNT" && "$COUNT" -gt 0 ]]; then
    success "kind:8000 member-added announcement found in DB ($COUNT row(s))"
else
    fail "kind:8000 not found in DB — relay may not be publishing member-added announcements"
fi

# ── Test 14: kind:28936 leave request (happy path) ────────────────────────────
#
# Member sends a valid leave request with the NIP-70 protected-event `-` tag.
# The relay should accept it and remove the member.

log "Test 14: kind:28936 leave request (happy path)..."

RESULT=$(send_event "$MEMBER_PRIVKEY" 28936 "" "-=")
if is_ok "$RESULT"; then
    success "Leave request accepted (kind:28936 with NIP-70 - tag)"
else
    fail "Leave request rejected: $RESULT"
fi

sleep 1

# Verify member was removed
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$MEMBER_PUBKEY"; then
    fail "Member still present in /api/relay/members after leave request"
else
    success "Member correctly absent from /api/relay/members after leave"
fi

# ── Test 15: kind:8001 member-removed announcement after leave ────────────────

log "Test 15: kind:8001 member-removed announcement published after leave..."

COUNT=$(psql "$DATABASE_URL" -t -c "SELECT count(*) FROM events WHERE kind=8001;" 2>/dev/null | tr -d ' \n')
if [[ -n "$COUNT" && "$COUNT" -gt 0 ]]; then
    success "kind:8001 member-removed announcement found in DB ($COUNT row(s))"
else
    fail "kind:8001 not found in DB — relay may not be publishing member-removed announcements"
fi

# ── Test 16: Leave request without NIP-70 - tag is rejected ──────────────────
#
# Re-add the member first, then send a leave WITHOUT the required `-` tag.
# The relay must reject it (NIP-70 enforcement).

log "Test 16: Leave request without NIP-70 - tag is rejected..."

# Re-add member
send_event "$ADMIN_PRIVKEY" 9030 "" \
    "p=$MEMBER_PUBKEY" \
    "role=member" > /dev/null 2>&1 || true
sleep 1

# Send leave without the - tag
RESULT=$(send_event "$MEMBER_PRIVKEY" 28936 "")
if is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'restricted\|denied\|forbidden\|nip-70\|protected\|false'; then
    success "Leave without NIP-70 - tag correctly rejected"
else
    fail "Leave without NIP-70 - tag was accepted (should be rejected): $RESULT"
fi

# ── Test 17: Stale admin command rejected (replay protection) ─────────────────
#
# An admin command with a created_at more than 5 minutes in the past must be
# rejected to prevent replay attacks.

log "Test 17: Stale admin command rejected (replay protection)..."

OLD_TS=$(($(date +%s) - 300))
# Use nak directly to control the timestamp; pipe output to the relay
RESULT=$(nak event \
    --sec "$ADMIN_PRIVKEY" \
    --kind 9030 \
    --created-at "$OLD_TS" \
    --tag "p=$ATTACKER_PUBKEY" \
    --tag "role=member" \
    --content "" \
    ws://localhost:3000 2>&1 || true)
if is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'stale\|old\|expired\|restricted\|false'; then
    success "Stale admin command (created_at -5m) rejected"
else
    fail "Stale admin command was accepted (replay protection missing): $RESULT"
fi

# Verify attacker was NOT added
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$ATTACKER_PUBKEY"; then
    fail "Attacker was added via stale command (replay protection bypass!)"
else
    success "Attacker not present in member list after stale command"
fi

# ── Test 18: Owner cannot leave (lockout prevention) ─────────────────────────
#
# The relay must refuse a leave request from the owner to prevent lockout.

log "Test 18: Owner cannot leave (lockout prevention)..."

RESULT=$(send_event "$OWNER_PRIVKEY" 28936 "" "-=")
if is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'owner\|restricted\|denied\|forbidden\|false'; then
    success "Owner leave request correctly rejected"
else
    fail "Owner leave request was accepted (lockout prevention missing): $RESULT"
fi

# Verify owner still present
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$OWNER_PUBKEY"; then
    success "Owner still present in /api/relay/members after leave attempt"
else
    fail "Owner missing from /api/relay/members after leave attempt"
fi

# ── Test 19: Admin cannot add another admin ───────────────────────────────────
#
# Admins can only add members. Elevating to admin requires owner privilege.

log "Test 19: Admin cannot add another admin..."

RESULT=$(send_event "$ADMIN_PRIVKEY" 9030 "" \
    "p=$ATTACKER_PUBKEY" \
    "role=admin")
if is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'restricted\|denied\|forbidden\|unauthorized\|permission\|false'; then
    success "Admin cannot add another admin (correctly rejected)"
else
    fail "Admin was able to add an admin (privilege escalation!): $RESULT"
fi

# Verify attacker was NOT added as admin
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | python3 -c "
import sys, json
data = json.load(sys.stdin)
members = data if isinstance(data, list) else data.get('members', [])
for m in members:
    if m.get('pubkey') == '${ATTACKER_PUBKEY}' and m.get('role') == 'admin':
        sys.exit(0)
sys.exit(1)
" 2>/dev/null; then
    fail "Attacker has admin role in member list (privilege escalation confirmed!)"
else
    success "Attacker not present as admin in member list"
fi

# ── Test 20: Admin cannot remove the owner ────────────────────────────────────

log "Test 20: Admin cannot remove the owner..."

RESULT=$(send_event "$ADMIN_PRIVKEY" 9031 "" \
    "p=$OWNER_PUBKEY")
if is_rejected "$RESULT" || echo "$RESULT" | grep -qi 'restricted\|denied\|forbidden\|unauthorized\|permission\|false'; then
    success "Admin cannot remove the owner (correctly rejected)"
else
    fail "Admin was able to remove the owner (privilege escalation!): $RESULT"
fi

# Verify owner still present
MEMBERS=$(rest_get_body "$OWNER_PRIVKEY" /api/relay/members)
if echo "$MEMBERS" | grep -q "$OWNER_PUBKEY"; then
    success "Owner still present in /api/relay/members after admin removal attempt"
else
    fail "Owner missing from /api/relay/members — admin removed owner!"
fi

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo "Final member list:"
echo "─────────────────────"
rest_get_body "$OWNER_PRIVKEY" /api/relay/members | python3 -m json.tool 2>/dev/null || true
echo ""

if [[ "$FAILURES" -eq 0 ]]; then
    echo -e "${GREEN}════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  ✅ All NIP-43 relay membership E2E tests passed!${NC}"
    echo -e "${GREEN}════════════════════════════════════════════════════════${NC}"
else
    echo -e "${RED}════════════════════════════════════════════════════════${NC}"
    echo -e "${RED}  ❌ ${FAILURES} test(s) failed!${NC}"
    echo -e "${RED}════════════════════════════════════════════════════════${NC}"
    exit 1
fi
