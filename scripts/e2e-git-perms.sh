#!/usr/bin/env bash
# =============================================================================
# e2e-git-perms.sh — End-to-end test for git transport, permissions, and signing
# =============================================================================
# Two bots collaborate on a simple web page via the Buzz relay's git server.
#
# Prerequisites:
#   - Docker services running (postgres, redis, minio)
#   - Relay built: cargo build --release --bin buzz-relay
#   - Credential helper built: cargo build --release --bin git-credential-nostr
#   - Signing program built: cargo build --release --bin git-sign-nostr
#   - Python 3 with websocket-client: pip install websocket-client
#
# What it tests:
#   Phase 1 — Transport + RBAC:
#     1. Owner creates a repo (kind:30617) and a channel
#     2. Owner adds two bots to the channel
#     3. Bot1 clones, creates index.html, pushes (should succeed)
#     4. Bot2 clones, modifies index.html, pushes (should succeed)
#     5. Guest tries to push (should be denied)
#   Phase 2 — Commit Signing (NIP-GS):
#     6. Unsigned commit pushes fine (advisory model)
#     7. Signed commit via git-sign-nostr + verify-commit
#     8. Signed commit with owner attestation (oa field)
#   Phase 3 — Auth Bypass + HMAC:
#     9. No auth header → 401
#    10. Malformed auth header → 401
#    11. Garbage Nostr token → 401
#    12. Forged HMAC on policy endpoint → rejected
#   Phase 4 — Hook Integrity:
#    13. Hook is regular file + executable
#    14. Symlink hook → push denied
#    15. Missing hook → push denied
#    16. Client core.hooksPath override cannot bypass server hook
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# ── Constants ─────────────────────────────────────────────────────────────────

RELAY_HOST="127.0.0.1"
RELAY_PORT="3000"
RELAY_HTTP="http://${RELAY_HOST}:${RELAY_PORT}"
RELAY_WS="ws://${RELAY_HOST}:${RELAY_PORT}"
REPO_NAME="e2e-webpage"
HMAC_SECRET="e2e-test-secret-that-is-long-enough-for-validation-purposes"
RELAY_STARTUP_TIMEOUT=15
CURL_TIMEOUT=10

# NIP-29 event kinds
KIND_CREATE_GROUP=9007
KIND_PUT_USER=9000
KIND_CREATE_REPO=30617

# ── Output helpers ────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log()     { printf "${BLUE}[e2e-git]${NC} %s\n" "$*"; }
success() { printf "${GREEN}[e2e-git]${NC} ✓ %s\n" "$*"; }
fail()    { printf "${RED}[e2e-git]${NC} ✗ %s\n" "$*" >&2; cleanup; exit 1; }
warn()    { printf "${YELLOW}[e2e-git]${NC} %s\n" "$*"; }

# ── Cleanup ───────────────────────────────────────────────────────────────────

RELAY_PID=""
WORK_DIR=""

cleanup() {
    if [[ -n "$RELAY_PID" ]]; then
        kill "$RELAY_PID" 2>/dev/null || true
        wait "$RELAY_PID" 2>/dev/null || true
    fi
    if [[ -n "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR"
    fi
}
trap cleanup EXIT

# ── Dependency checks ─────────────────────────────────────────────────────────

check_deps() {
    local missing=()

    if [[ ! -x "${REPO_ROOT}/target/release/buzz-relay" ]]; then
        missing+=("buzz-relay (cargo build --release --bin buzz-relay)")
    fi
    if [[ ! -x "${REPO_ROOT}/target/release/git-credential-nostr" ]]; then
        missing+=("git-credential-nostr (cargo build --release --bin git-credential-nostr)")
    fi
    if ! command -v python3 &>/dev/null; then
        missing+=("python3")
    fi
    if ! python3 -c "import websocket" 2>/dev/null; then
        missing+=("python3 websocket-client (pip install websocket-client)")
    fi
    if ! command -v curl &>/dev/null; then
        missing+=("curl")
    fi
    if ! command -v git &>/dev/null; then
        missing+=("git")
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing dependencies: ${missing[*]}"
    fi
}

check_deps

# ── Keypair generation ────────────────────────────────────────────────────────

generate_keypair() {
    openssl rand -hex 32
}

# Derive x-only public key from private key using pure Python secp256k1.
# NOTE: This is a TEST-ONLY implementation. The scalar multiplication is
# variable-time and uses a deterministic nonce (no aux randomness). This is
# acceptable for E2E tests but MUST NOT be used for production signing.
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

k = int('$privkey', 16)
pub = scalar_mult(k, (Gx, Gy))
print(format(pub[0], '064x'))
"
}

# ── Git clone/push helpers ────────────────────────────────────────────────────

CRED_HELPER="${REPO_ROOT}/target/release/git-credential-nostr"

# Clone a repo with nostr credential helper configured.
# Usage: git_clone <privkey> <repo_url> <dest_dir>
git_clone() {
    local privkey="$1" repo_url="$2" dest_dir="$3"
    NOSTR_PRIVATE_KEY="$privkey" GIT_TERMINAL_PROMPT=0 \
        git clone \
        -c credential.helper="" \
        -c credential.useHttpPath=true \
        -c "credential.${RELAY_HTTP}.helper=${CRED_HELPER}" \
        -c init.defaultBranch=main \
        "$repo_url" "$dest_dir" 2>&1
}

# Push from a repo with nostr credential helper configured.
# Usage: git_push <privkey> <repo_dir> [push_args...]
git_push() {
    local privkey="$1" repo_dir="$2"
    shift 2
    NOSTR_PRIVATE_KEY="$privkey" GIT_TERMINAL_PROMPT=0 \
        git -C "$repo_dir" \
        -c credential.helper="" \
        -c credential.useHttpPath=true \
        -c "credential.${RELAY_HTTP}.helper=${CRED_HELPER}" \
        push "$@" 2>&1
}

# ── Nostr event helper ────────────────────────────────────────────────────────

# Send a signed Nostr event via WebSocket.
# Usage: send_event <privkey> <kind> <content> <tags_json>
# Returns: "OK:<event_id>" on success, exits non-zero on failure.
send_event() {
    local privkey="$1"
    local kind="$2"
    local content="$3"
    shift 3
    local tags_json="${*:-}"

    python3 << PYEOF
import json, hashlib, time
import websocket

P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
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

def sign_schnorr(privkey_bytes, msg_bytes):
    """BIP-340 Schnorr signing — TEST ONLY (variable-time, deterministic nonce)."""
    k_int = int.from_bytes(privkey_bytes, 'big')
    pubpoint = scalar_mult(k_int, (Gx, Gy))
    pubkey_bytes = pubpoint[0].to_bytes(32, 'big')
    if pubpoint[1] % 2 != 0:
        k_int = N - k_int
    nonce_hash = hashlib.sha256(k_int.to_bytes(32, 'big') + msg_bytes).digest()
    r_int = int.from_bytes(nonce_hash, 'big') % N
    if r_int == 0: raise Exception("bad nonce")
    R = scalar_mult(r_int, (Gx, Gy))
    if R[1] % 2 != 0:
        r_int = N - r_int
    R_bytes = R[0].to_bytes(32, 'big')
    tag_hash = hashlib.sha256(b'BIP0340/challenge').digest()
    e_hash = hashlib.sha256(tag_hash + tag_hash + R_bytes + pubkey_bytes + msg_bytes).digest()
    e_int = int.from_bytes(e_hash, 'big') % N
    s_int = (r_int + e_int * k_int) % N
    return R_bytes + s_int.to_bytes(32, 'big')

privkey = bytes.fromhex("${privkey}")
pubpoint = scalar_mult(int.from_bytes(privkey, 'big'), (Gx, Gy))
pubkey_hex = format(pubpoint[0], '064x')

created_at = int(time.time())
tags = json.loads('[${tags_json}]') if '''${tags_json}'''.strip() else []
content = """${content}"""

serialized = json.dumps([0, pubkey_hex, created_at, ${kind}, tags, content], separators=(',',':'), ensure_ascii=False)
id_bytes = hashlib.sha256(serialized.encode()).digest()
event_id = id_bytes.hex()
sig = sign_schnorr(privkey, id_bytes)

event = {
    "id": event_id,
    "pubkey": pubkey_hex,
    "created_at": created_at,
    "kind": ${kind},
    "tags": tags,
    "content": content,
    "sig": sig.hex()
}

ws = websocket.create_connection("${RELAY_WS}", timeout=5)
msg = json.loads(ws.recv())
if msg[0] == "AUTH":
    challenge = msg[1]
    auth_created = int(time.time())
    auth_tags = [["relay", "${RELAY_WS}"], ["challenge", challenge]]
    auth_serial = json.dumps([0, pubkey_hex, auth_created, 22242, auth_tags, ""], separators=(',',':'))
    auth_id = hashlib.sha256(auth_serial.encode()).digest()
    auth_sig = sign_schnorr(privkey, auth_id)
    auth_event = {
        "id": auth_id.hex(),
        "pubkey": pubkey_hex,
        "created_at": auth_created,
        "kind": 22242,
        "tags": auth_tags,
        "content": "",
        "sig": auth_sig.hex()
    }
    ws.send(json.dumps(["AUTH", auth_event]))
    for _ in range(10):
        resp = json.loads(ws.recv())
        if resp[0] == "OK" and resp[1] == auth_id.hex():
            if not resp[2]:
                print(f"AUTH failed: {resp}")
                ws.close()
                exit(1)
            break
    else:
        print("AUTH: no OK received for our auth event")
        ws.close()
        exit(1)

ws.send(json.dumps(["EVENT", event]))
for _ in range(10):
    resp = json.loads(ws.recv())
    if resp[0] == "OK" and resp[1] == event_id:
        if resp[2]:
            print(f"OK:{event_id}")
        else:
            reason = resp[3] if len(resp) > 3 else "unknown"
            print(f"REJECTED:{reason}")
            exit(1)
        break
else:
    print("EVENT: no OK received for our event")
    exit(1)
ws.close()
PYEOF
}

# ── Start relay ───────────────────────────────────────────────────────────────

log "Starting relay..."

if [[ -f .env ]]; then
    set -o allexport
    # shellcheck source=/dev/null
    source .env
    set +o allexport
fi

export BUZZ_GIT_REPO_PATH="${REPO_ROOT}/repos"
export BUZZ_GIT_HOOK_HMAC_SECRET="${HMAC_SECRET}"
export BUZZ_BIND_ADDR="${RELAY_HOST}:${RELAY_PORT}"
export RELAY_URL="${RELAY_WS}"
export RUST_LOG="buzz_relay=warn"
export BUZZ_REQUIRE_AUTH_TOKEN=false

# Clean repos dir (isolated test state)
rm -rf "${REPO_ROOT}/repos"
mkdir -p "${REPO_ROOT}/repos"

./target/release/buzz-relay > /tmp/buzz-relay-e2e.log 2>&1 &
RELAY_PID=$!

# Wait for relay to be ready (poll, not sleep)
for i in $(seq 1 "$RELAY_STARTUP_TIMEOUT"); do
    if curl -sf --max-time 2 "${RELAY_HTTP}/" -H "Accept: application/nostr+json" | grep -q "Buzz"; then
        break
    fi
    if [[ $i -eq "$RELAY_STARTUP_TIMEOUT" ]]; then
        fail "Relay did not start within ${RELAY_STARTUP_TIMEOUT}s. Check /tmp/buzz-relay-e2e.log"
    fi
    sleep 1
done
success "Relay started (PID $RELAY_PID) on ${RELAY_HOST}:${RELAY_PORT}"

# ── Generate identities ──────────────────────────────────────────────────────

log "Generating keypairs..."

OWNER_PRIVKEY=$(generate_keypair)
OWNER_PUBKEY=$(derive_pubkey "$OWNER_PRIVKEY")
BOT1_PRIVKEY=$(generate_keypair)
BOT1_PUBKEY=$(derive_pubkey "$BOT1_PRIVKEY")
BOT2_PRIVKEY=$(generate_keypair)
BOT2_PUBKEY=$(derive_pubkey "$BOT2_PRIVKEY")
GUEST_PRIVKEY=$(generate_keypair)
GUEST_PUBKEY=$(derive_pubkey "$GUEST_PRIVKEY")

log "  Owner:  ${OWNER_PUBKEY:0:16}..."
log "  Bot1:   ${BOT1_PUBKEY:0:16}..."
log "  Bot2:   ${BOT2_PUBKEY:0:16}..."
log "  Guest:  ${GUEST_PUBKEY:0:16}..."

# ── Work directory ────────────────────────────────────────────────────────────

WORK_DIR=$(mktemp -d)
log "Work dir: $WORK_DIR"

# ── Phase 1: Transport + RBAC ────────────────────────────────────────────────

log "Creating channel..."

CHANNEL_ID=$(python3 -c "import uuid; print(str(uuid.uuid4()))")
log "  Channel ID: $CHANNEL_ID"

CHANNEL_RESULT=$(send_event "$OWNER_PRIVKEY" "$KIND_CREATE_GROUP" "" \
    "[\"h\", \"$CHANNEL_ID\"], [\"name\", \"e2e-git-test\"], [\"channel_type\", \"stream\"], [\"visibility\", \"open\"]")
log "  Channel create: $CHANNEL_RESULT"

log "Adding bot1 to channel..."
ADD_BOT1=$(send_event "$OWNER_PRIVKEY" "$KIND_PUT_USER" "" \
    "[\"h\", \"$CHANNEL_ID\"], [\"p\", \"$BOT1_PUBKEY\"]")
log "  Add bot1: $ADD_BOT1"

log "Adding bot2 to channel..."
ADD_BOT2=$(send_event "$OWNER_PRIVKEY" "$KIND_PUT_USER" "" \
    "[\"h\", \"$CHANNEL_ID\"], [\"p\", \"$BOT2_PUBKEY\"]")
log "  Add bot2: $ADD_BOT2"

log "Creating repo: $REPO_NAME..."
CREATE_REPO=$(send_event "$OWNER_PRIVKEY" "$KIND_CREATE_REPO" "" \
    "[\"d\", \"$REPO_NAME\"], [\"buzz-channel\", \"$CHANNEL_ID\"]")
log "  Create repo: $CREATE_REPO"

# Wait for repo creation side effect (bare repo on disk)
for i in $(seq 1 10); do
    if [[ -d "${REPO_ROOT}/repos/${OWNER_PUBKEY}/${REPO_NAME}.git" ]]; then
        break
    fi
    if [[ $i -eq 10 ]]; then
        fail "Repo not created at repos/${OWNER_PUBKEY}/${REPO_NAME}.git within 10s"
    fi
    sleep 1
done
success "Bare repo created on disk"

# Verify hook installed
REPO_PATH="${REPO_ROOT}/repos/${OWNER_PUBKEY}/${REPO_NAME}.git"
HOOK_PATH="${REPO_PATH}/hooks/pre-receive"

if [[ -x "$HOOK_PATH" ]]; then
    success "Pre-receive hook installed and executable"
else
    fail "Pre-receive hook not found or not executable"
fi

# ── Test: Bot1 clones and pushes index.html ───────────────────────────────────

log "Bot1: cloning repo..."
BOT1_DIR="$WORK_DIR/bot1/repo"
mkdir -p "$WORK_DIR/bot1"

CLONE_OUTPUT=$(git_clone "$BOT1_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$BOT1_DIR" 2>&1) || true

# Handle empty repo (git clone warns but exits non-zero for empty repos)
if [[ ! -d "$BOT1_DIR/.git" ]]; then
    mkdir -p "$BOT1_DIR"
    git -C "$BOT1_DIR" init -b main
    git -C "$BOT1_DIR" remote add origin "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}"
fi

# Create index.html
cat > "$BOT1_DIR/index.html" << 'HTML'
<!DOCTYPE html>
<html>
<head>
    <title>Buzz E2E Test Page</title>
    <style>
        body { font-family: system-ui; max-width: 800px; margin: 0 auto; padding: 2rem; }
        h1 { color: #2d5016; }
        .contributor { padding: 0.5rem; margin: 0.5rem 0; background: #f0f9e8; border-radius: 4px; }
    </style>
</head>
<body>
    <h1>🐝 Buzz Collaborative Page</h1>
    <p>This page was created by two bots collaborating via Buzz's git server.</p>
    <div class="contributor">
        <strong>Bot 1</strong> — Created the initial page structure
    </div>
</body>
</html>
HTML

git -C "$BOT1_DIR" add -A
git -C "$BOT1_DIR" -c user.name="Bot1" -c user.email="bot1@buzz.test" \
    -c init.defaultBranch=main commit -m "Initial page structure"

log "Bot1: pushing..."
if git_push "$BOT1_PRIVKEY" "$BOT1_DIR" -u origin main; then
    success "Bot1 push succeeded (member can push)"
else
    tail -20 /tmp/buzz-relay-e2e.log
    fail "Bot1 push failed (member should be able to push)"
fi

# ── Test: Bot2 clones and pushes ──────────────────────────────────────────────

log "Bot2: cloning repo..."
BOT2_DIR="$WORK_DIR/bot2"

git_clone "$BOT2_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$BOT2_DIR" \
    || fail "Bot2 clone failed"

# Modify index.html — insert before closing </body> to keep valid HTML
sed -i.bak '/<\/body>/i\
    <div class="contributor">\
        <strong>Bot 2</strong> — Added this section (pushing as bot role → promoted to member)\
    </div>\
    <footer>\
        <p><em>Built with Buzz sovereign git hosting</em></p>\
    </footer>' "$BOT2_DIR/index.html"
rm -f "$BOT2_DIR/index.html.bak"

git -C "$BOT2_DIR" add -A
git -C "$BOT2_DIR" -c user.name="Bot2" -c user.email="bot2@buzz.test" \
    commit -m "Add bot2 section and footer"

log "Bot2: pushing..."
if git_push "$BOT2_PRIVKEY" "$BOT2_DIR"; then
    success "Bot2 push succeeded (bot promoted to member)"
else
    tail -20 /tmp/buzz-relay-e2e.log
    fail "Bot2 push failed (bot should be promoted to member)"
fi

# ── Test: Non-member push denied ──────────────────────────────────────────────

log "Guest: attempting push (should be denied)..."
GUEST_DIR="$WORK_DIR/guest"

git_clone "$GUEST_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$GUEST_DIR" \
    || fail "Guest clone failed (read access should work)"

echo "<!-- unauthorized -->" >> "$GUEST_DIR/index.html"
git -C "$GUEST_DIR" add -A
git -C "$GUEST_DIR" -c user.name="Guest" -c user.email="guest@evil.test" \
    commit -m "Unauthorized change"

PUSH_OUTPUT=$(git_push "$GUEST_PRIVKEY" "$GUEST_DIR" 2>&1) && \
    fail "Guest push succeeded (should have been denied!)"

# Verify the denial is permission-related, not a network error
if echo "$PUSH_OUTPUT" | grep -qi "denied\|forbidden\|not authorized\|403\|permission"; then
    success "Guest push denied (not a channel member) — reason confirmed in output"
else
    warn "Guest push failed but denial reason not found in output: $PUSH_OUTPUT"
    success "Guest push denied (non-zero exit)"
fi

# ── Final verification ────────────────────────────────────────────────────────

log "Verifying final repo state..."
VERIFY_DIR="$WORK_DIR/verify"

git_clone "$OWNER_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$VERIFY_DIR" \
    || fail "Owner clone for verification failed"

if grep -q "Bot 1" "$VERIFY_DIR/index.html" && grep -q "Bot 2" "$VERIFY_DIR/index.html"; then
    success "Final repo contains both bots' contributions"
else
    fail "Final repo missing expected content"
fi

log "Commit log:"
git -C "$VERIFY_DIR" log --oneline

success "=== PHASE 1 COMPLETE: Transport + RBAC ==="

# =============================================================================
# PHASE 2 — Commit Signing (NIP-GS)
# =============================================================================

SIGNER="${REPO_ROOT}/target/release/git-sign-nostr"

if [[ ! -x "$SIGNER" ]]; then
    warn "git-sign-nostr not built — skipping signing tests"
    warn "Build with: cargo build --release --bin git-sign-nostr"
else

# ── Test: Unsigned commit pushes fine (advisory model) ────────────────────────
# WHY: NIP-GS signing is client-side provenance only. The relay does NOT enforce
# signatures on push — any authenticated member can push unsigned commits.
# This is intentional: signing proves authorship to verifiers, but the relay's
# job is authorization (channel membership + branch protection), not signature
# enforcement.

log "Advisory model: unsigned commit should push successfully..."
UNSIGNED_DIR="$WORK_DIR/unsigned"

git_clone "$BOT1_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$UNSIGNED_DIR" \
    || fail "Clone for unsigned test failed"

echo "<!-- unsigned change -->" >> "$UNSIGNED_DIR/index.html"
git -C "$UNSIGNED_DIR" add -A
git -C "$UNSIGNED_DIR" -c user.name="Bot1" -c user.email="bot1@buzz.test" \
    commit -m "Unsigned commit (no gpgsign)"

if git_push "$BOT1_PRIVKEY" "$UNSIGNED_DIR"; then
    success "Unsigned commit pushed (signing is advisory, not enforced server-side)"
else
    fail "Unsigned commit push failed — server should not enforce signing"
fi

# ── Test: Signed commit with git-sign-nostr ───────────────────────────────────

log "Signing: configuring git-sign-nostr and making a signed commit..."
SIGNED_DIR="$WORK_DIR/signed"

git_clone "$BOT1_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$SIGNED_DIR" \
    || fail "Clone for signed test failed"

echo "<!-- signed by bot1 -->" >> "$SIGNED_DIR/index.html"
git -C "$SIGNED_DIR" add -A

NOSTR_PRIVATE_KEY="$BOT1_PRIVKEY" \
git -C "$SIGNED_DIR" \
    -c user.name="Bot1" \
    -c user.email="bot1@buzz.test" \
    -c gpg.format=x509 \
    -c "gpg.x509.program=$SIGNER" \
    -c commit.gpgsign=true \
    -c "user.signingkey=$BOT1_PUBKEY" \
    commit -m "Signed commit via NIP-GS"

# Verify the signature locally
log "Verifying signature with git verify-commit..."
if NOSTR_PRIVATE_KEY="$BOT1_PRIVKEY" \
   git -C "$SIGNED_DIR" \
    -c gpg.format=x509 \
    -c "gpg.x509.program=$SIGNER" \
    -c "user.signingkey=$BOT1_PUBKEY" \
    verify-commit HEAD 2>&1; then
    success "git verify-commit succeeded (GOODSIG)"
else
    fail "git verify-commit failed"
fi

# Push the signed commit
log "Pushing signed commit..."
if git_push "$BOT1_PRIVKEY" "$SIGNED_DIR"; then
    success "Signed commit pushed successfully"
else
    fail "Signed commit push failed"
fi

# ── Test: Signed commit with owner attestation (NIP-OA) ──────────────────────

log "Signing with owner attestation (BUZZ_AUTH_TAG)..."
OA_DIR="$WORK_DIR/oa-signed"

git_clone "$BOT1_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$OA_DIR" \
    || fail "Clone for OA test failed"

echo "<!-- signed with oa -->" >> "$OA_DIR/index.html"
git -C "$OA_DIR" add -A

# Generate a NIP-OA auth tag: owner authorizes bot1
OA_TAG=$(python3 << PYEOF
import hashlib, json

P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
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

def sign_schnorr(privkey_hex, msg_bytes):
    """BIP-340 Schnorr signing — TEST ONLY."""
    k_int = int(privkey_hex, 16)
    pubpoint = scalar_mult(k_int, (Gx, Gy))
    pubkey_bytes = pubpoint[0].to_bytes(32, 'big')
    if pubpoint[1] % 2 != 0:
        k_int = N - k_int
    nonce_hash = hashlib.sha256(k_int.to_bytes(32, 'big') + msg_bytes).digest()
    r_int = int.from_bytes(nonce_hash, 'big') % N
    if r_int == 0: raise Exception("bad nonce")
    R = scalar_mult(r_int, (Gx, Gy))
    if R[1] % 2 != 0:
        r_int = N - r_int
    R_bytes = R[0].to_bytes(32, 'big')
    tag_hash = hashlib.sha256(b'BIP0340/challenge').digest()
    e_hash = hashlib.sha256(tag_hash + tag_hash + R_bytes + pubkey_bytes + msg_bytes).digest()
    e_int = int.from_bytes(e_hash, 'big') % N
    s_int = (r_int + e_int * k_int) % N
    return (R_bytes + s_int.to_bytes(32, 'big')).hex()

owner_privkey = "${OWNER_PRIVKEY}"
bot1_pubkey = "${BOT1_PUBKEY}"
owner_pubpoint = scalar_mult(int(owner_privkey, 16), (Gx, Gy))
owner_pubkey = format(owner_pubpoint[0], '064x')

preimage = f"nostr:agent-auth:{bot1_pubkey}:"
msg = hashlib.sha256(preimage.encode()).digest()
sig = sign_schnorr(owner_privkey, msg)

print(json.dumps(["auth", owner_pubkey, "", sig]))
PYEOF
)

NOSTR_PRIVATE_KEY="$BOT1_PRIVKEY" BUZZ_AUTH_TAG="$OA_TAG" \
git -C "$OA_DIR" \
    -c user.name="Bot1" \
    -c user.email="bot1@buzz.test" \
    -c gpg.format=x509 \
    -c "gpg.x509.program=$SIGNER" \
    -c commit.gpgsign=true \
    -c "user.signingkey=$BOT1_PUBKEY" \
    commit -m "Signed commit with owner attestation"

# Verify the oa field is present in the signature
COMMIT_SIG=$(git -C "$OA_DIR" cat-file commit HEAD | sed -n '/^gpgsig /,/^[^ ]/{ /^gpgsig /d; /^[^ ]/d; s/^ //; p; }')
DECODED_SIG=$(echo "$COMMIT_SIG" | base64 -d 2>/dev/null || echo "$COMMIT_SIG" | base64 -D 2>/dev/null)
if echo "$DECODED_SIG" | grep -q '"oa"'; then
    success "Owner attestation (oa field) present in signature"
else
    fail "Owner attestation missing from signature — BUZZ_AUTH_TAG not picked up"
fi

# Push it
if git_push "$BOT1_PRIVKEY" "$OA_DIR"; then
    success "Signed commit with oa pushed successfully"
else
    fail "Signed commit with oa push failed"
fi

success "=== PHASE 2 COMPLETE: Commit Signing ==="

fi  # end git-sign-nostr check

# =============================================================================
# PHASE 3 — Auth Bypass Tests
# =============================================================================

log "Auth bypass: testing unauthenticated access..."

GIT_INFO_URL="${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}/info/refs?service=git-upload-pack"

# ── Test: No auth header → 401 ───────────────────────────────────────────────

HTTP_CODE=$(curl -sf --max-time "$CURL_TIMEOUT" -o /dev/null -w "%{http_code}" "$GIT_INFO_URL" || true)
if [[ "$HTTP_CODE" == "401" ]]; then
    success "No auth header → 401 (unauthenticated clone rejected)"
else
    fail "No auth header got HTTP $HTTP_CODE (expected 401)"
fi

# ── Test: Malformed auth header → 401 ────────────────────────────────────────

HTTP_CODE=$(curl -sf --max-time "$CURL_TIMEOUT" -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer invalid-token" "$GIT_INFO_URL" || true)
if [[ "$HTTP_CODE" == "401" ]]; then
    success "Malformed auth (Bearer instead of Nostr) → 401"
else
    fail "Malformed auth got HTTP $HTTP_CODE (expected 401)"
fi

# ── Test: Garbage base64 in Nostr auth → 401 ─────────────────────────────────

HTTP_CODE=$(curl -sf --max-time "$CURL_TIMEOUT" -o /dev/null -w "%{http_code}" \
    -H "Authorization: Nostr dGhpcyBpcyBub3QgYSB2YWxpZCBldmVudA==" "$GIT_INFO_URL" || true)
if [[ "$HTTP_CODE" == "401" ]]; then
    success "Garbage Nostr token → 401"
else
    fail "Garbage Nostr token got HTTP $HTTP_CODE (expected 401)"
fi

# ── Test: HMAC tampering — forged policy callback rejected ────────────────────
# ATTACK: An attacker who can reach the policy endpoint tries to forge a hook
# callback with a wrong HMAC to authorize an unauthorized push.

log "Auth bypass: forged HMAC on policy endpoint..."
FORGED_TIMESTAMP=$(date +%s)
FORGED_PAYLOAD=$(cat << JSONEOF
{"repo_id":"${REPO_NAME}","repo_owner":"${OWNER_PUBKEY}","pusher_pubkey":"${GUEST_PUBKEY}","ref_updates":[{"old_oid":"0000000000000000000000000000000000000000","new_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","ref_name":"refs/heads/main","is_ancestor":false}],"timestamp":${FORGED_TIMESTAMP},"signature":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}
JSONEOF
)

HTTP_CODE=$(curl -sf --max-time "$CURL_TIMEOUT" -o /dev/null -w "%{http_code}" \
    -X POST \
    -H "Content-Type: application/json" \
    -d "$FORGED_PAYLOAD" \
    "${RELAY_HTTP}/internal/git/policy" || true)
if [[ "$HTTP_CODE" == "403" || "$HTTP_CODE" == "401" ]]; then
    success "Forged HMAC on policy endpoint → rejected ($HTTP_CODE)"
else
    fail "Forged HMAC got HTTP $HTTP_CODE (expected 403 or 401)"
fi

success "=== PHASE 3 COMPLETE: Auth Bypass + HMAC ==="

# =============================================================================
# PHASE 4 — Hook Integrity
# =============================================================================

# ── Test: Hook is a regular file (not symlink) ────────────────────────────────

log "Hook integrity: verifying hook is regular file..."
if [[ -f "$HOOK_PATH" && ! -L "$HOOK_PATH" ]]; then
    success "Pre-receive hook is a regular file (not a symlink)"
else
    fail "Pre-receive hook is missing or is a symlink"
fi

# ── Test: Hook is executable ──────────────────────────────────────────────────

if [[ -x "$HOOK_PATH" ]]; then
    success "Pre-receive hook is executable"
else
    fail "Pre-receive hook is not executable"
fi

# ── Test: Symlink hook rejected by relay ──────────────────────────────────────
# Use a subshell with trap to guarantee hook restoration even on failure.

log "Hook integrity: testing symlink hook rejection..."
(
    # Backup and replace with symlink
    cp "$HOOK_PATH" "${HOOK_PATH}.bak"
    trap 'rm -f "$HOOK_PATH"; mv "${HOOK_PATH}.bak" "$HOOK_PATH"' EXIT

    rm "$HOOK_PATH"
    ln -s /dev/null "$HOOK_PATH"

    SYMLINK_DIR="$WORK_DIR/symlink-test"
    git_clone "$BOT1_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$SYMLINK_DIR" \
        || exit 1

    echo "<!-- symlink test -->" >> "$SYMLINK_DIR/index.html"
    git -C "$SYMLINK_DIR" add -A
    git -C "$SYMLINK_DIR" -c user.name="Bot1" -c user.email="bot1@buzz.test" \
        commit -m "Symlink hook test"

    if git_push "$BOT1_PRIVKEY" "$SYMLINK_DIR" 2>&1; then
        echo "PUSH_SUCCEEDED"
        exit 2  # Signal that push should have failed
    else
        echo "PUSH_DENIED"
        exit 0
    fi
)
SYMLINK_RESULT=$?
if [[ $SYMLINK_RESULT -eq 0 ]]; then
    success "Push denied with symlink hook (relay detected tampering)"
elif [[ $SYMLINK_RESULT -eq 2 ]]; then
    fail "Push succeeded with symlink hook (should have been rejected)"
else
    fail "Symlink hook test failed unexpectedly (exit $SYMLINK_RESULT)"
fi

# ── Test: Missing hook rejected by relay ──────────────────────────────────────

log "Hook integrity: testing missing hook rejection..."
(
    mv "$HOOK_PATH" "${HOOK_PATH}.bak"
    trap 'mv "${HOOK_PATH}.bak" "$HOOK_PATH"' EXIT

    MISSING_DIR="$WORK_DIR/missing-hook-test"
    git_clone "$BOT1_PRIVKEY" "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" "$MISSING_DIR" \
        || exit 1

    echo "<!-- missing hook test -->" >> "$MISSING_DIR/index.html"
    git -C "$MISSING_DIR" add -A
    git -C "$MISSING_DIR" -c user.name="Bot1" -c user.email="bot1@buzz.test" \
        commit -m "Missing hook test"

    if git_push "$BOT1_PRIVKEY" "$MISSING_DIR" 2>&1; then
        exit 2  # Push should have failed
    else
        exit 0
    fi
)
MISSING_RESULT=$?
if [[ $MISSING_RESULT -eq 0 ]]; then
    success "Push denied with missing hook (fail-closed)"
elif [[ $MISSING_RESULT -eq 2 ]]; then
    fail "Push succeeded with missing hook (should have been rejected)"
else
    fail "Missing hook test failed unexpectedly (exit $MISSING_RESULT)"
fi

# ── Test: Client-side core.hooksPath cannot bypass server hook ────────────────
# ATTACK: A malicious repo could set core.hooksPath in its .git/config to point
# to /dev/null or a no-op script. This only affects client-side hooks — the
# server's pre-receive hook is invoked by git-receive-pack on the SERVER, not
# the client. This test confirms the server hook still fires.

log "Hook integrity: client core.hooksPath cannot bypass server hook..."
HOOKSPATH_DIR="$WORK_DIR/hookspath-test"

NOSTR_PRIVATE_KEY="$GUEST_PRIVKEY" GIT_TERMINAL_PROMPT=0 \
    git clone \
    -c credential.helper="" \
    -c credential.useHttpPath=true \
    -c "credential.${RELAY_HTTP}.helper=${CRED_HELPER}" \
    -c core.hooksPath=/dev/null \
    "${RELAY_HTTP}/git/${OWNER_PUBKEY}/${REPO_NAME}" \
    "$HOOKSPATH_DIR" 2>&1 || fail "Clone for hooksPath test failed"

echo "<!-- hooksPath bypass attempt -->" >> "$HOOKSPATH_DIR/index.html"
git -C "$HOOKSPATH_DIR" add -A
git -C "$HOOKSPATH_DIR" \
    -c user.name="Guest" -c user.email="guest@evil.test" \
    -c core.hooksPath=/dev/null \
    commit -m "Attempt to bypass server hook via client hooksPath"

PUSH_OUTPUT=$(NOSTR_PRIVATE_KEY="$GUEST_PRIVKEY" GIT_TERMINAL_PROMPT=0 \
    git -C "$HOOKSPATH_DIR" \
    -c credential.helper="" \
    -c credential.useHttpPath=true \
    -c "credential.${RELAY_HTTP}.helper=${CRED_HELPER}" \
    -c core.hooksPath=/dev/null \
    push 2>&1) && \
    fail "Push succeeded with client hooksPath override (server hook should still deny)"

success "Client core.hooksPath=/dev/null does NOT bypass server-side pre-receive hook"

success "=== PHASE 4 COMPLETE: Hook Integrity ==="

# =============================================================================
# DONE
# =============================================================================

printf "\n"
printf "${GREEN}════════════════════════════════════════════════════════${NC}\n"
printf "${GREEN}  All E2E tests passed!${NC}\n"
printf "${GREEN}  • Transport + RBAC (clone, push, deny)${NC}\n"
printf "${GREEN}  • Commit Signing (NIP-GS sign + verify + oa)${NC}\n"
printf "${GREEN}  • Auth Bypass (no auth, malformed, garbage, HMAC)${NC}\n"
printf "${GREEN}  • Hook Integrity (symlink, missing → denied)${NC}\n"
printf "${GREEN}════════════════════════════════════════════════════════${NC}\n"
printf "\n"
