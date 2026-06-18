#!/usr/bin/env bash
# test-video-upload.sh — Live validation of the Blossom video upload flow.
#
# Prerequisites:
#   - Relay running at $RELAY_URL (default: http://localhost:3000)
#   - Dev mode (BUZZ_REQUIRE_AUTH_TOKEN=false) or valid API token
#   - ffmpeg, nak, curl, jq, shasum on PATH
#
# Usage:
#   ./scripts/test-video-upload.sh              # run all tests
#   RELAY_URL=http://host:3000 ./scripts/...    # custom relay URL
#   NSEC=nsec1... ./scripts/...                 # use existing key

set -euo pipefail

RELAY_URL="${RELAY_URL:-http://localhost:3000}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

PASS=0
FAIL=0

pass() { echo "  ✅ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ❌ $1"; FAIL=$((FAIL + 1)); }

# ── Dependencies ───────────────────────────────────────────────────────────────

for cmd in ffmpeg nak curl jq shasum; do
    command -v "$cmd" >/dev/null 2>&1 || { echo "Missing: $cmd"; exit 1; }
done

# ── Key generation ─────────────────────────────────────────────────────────────

if [ -z "${NSEC:-}" ]; then
    NSEC="$(nak key generate)"
    echo "Generated key: $(echo "$NSEC" | nak key public)"
fi
NPUB="$(echo "$NSEC" | nak key public)"
echo "Using pubkey: $NPUB"
echo "Relay: $RELAY_URL"
echo ""

# ── Generate test MP4 ─────────────────────────────────────────────────────────
# Minimal 1-second H.264 video with moov at front (faststart).

TEST_MP4="$TMPDIR/test.mp4"
ffmpeg -y -f lavfi -i "color=c=blue:s=320x240:d=1" \
    -c:v libx264 -profile:v baseline -pix_fmt yuv420p \
    -movflags +faststart \
    "$TEST_MP4" 2>/dev/null

FILE_SIZE=$(wc -c < "$TEST_MP4" | tr -d ' ')
SHA256=$(shasum -a 256 "$TEST_MP4" | cut -d' ' -f1)
echo "Test MP4: ${FILE_SIZE} bytes, sha256=${SHA256:0:16}..."
echo ""

# ── Helper: build Blossom auth header ──────────────────────────────────────────
# Creates a kind:24242 event with t=upload, x=<sha256>, expiration=+5min.

blossom_auth() {
    local sha256="$1"
    local now exp auth_event auth_b64

    now=$(date +%s)
    exp=$((now + 300))

    auth_event=$(nak event \
        --sec "$NSEC" \
        -k 24242 \
        -c "Upload test video" \
        -t t=upload \
        -t "x=$sha256" \
        -t "expiration=$exp" \
        2>/dev/null)

    auth_b64=$(echo -n "$auth_event" | base64 | tr -d '\n')
    echo "Nostr $auth_b64"
}

# ── Test 1: Upload MP4 ────────────────────────────────────────────────────────

echo "Test 1: Upload MP4 via PUT /media/upload"
AUTH="$(blossom_auth "$SHA256")"

UPLOAD_RESP=$(curl -s -w "\n%{http_code}" \
    -X PUT "$RELAY_URL/media/upload" \
    -H "Authorization: $AUTH" \
    -H "Content-Type: video/mp4" \
    -H "X-SHA-256: $SHA256" \
    --data-binary "@$TEST_MP4")

UPLOAD_HTTP=$(echo "$UPLOAD_RESP" | tail -1)
UPLOAD_BODY=$(echo "$UPLOAD_RESP" | sed '$d')

if [ "$UPLOAD_HTTP" = "200" ]; then
    pass "Upload returned 200"
    BLOB_URL=$(echo "$UPLOAD_BODY" | jq -r '.url // empty')
    if [ -n "$BLOB_URL" ]; then
        pass "Response contains url: ${BLOB_URL:0:60}..."
    else
        fail "Response missing url field"
    fi
    # Check duration field present
    DURATION=$(echo "$UPLOAD_BODY" | jq -r '.duration // empty')
    if [ -n "$DURATION" ]; then
        pass "Response contains duration: ${DURATION}s"
    else
        fail "Response missing duration field"
    fi
else
    fail "Upload returned $UPLOAD_HTTP (expected 200)"
    echo "    Body: $UPLOAD_BODY"
fi
echo ""

# ── Test 2: GET full blob ─────────────────────────────────────────────────────

echo "Test 2: GET /media/${SHA256}.mp4 (full download)"
GET_RESP=$(curl -s -o "$TMPDIR/downloaded.mp4" -w "%{http_code}" \
    "$RELAY_URL/media/${SHA256}.mp4")

if [ "$GET_RESP" = "200" ]; then
    pass "GET returned 200"
    DL_SIZE=$(wc -c < "$TMPDIR/downloaded.mp4" | tr -d ' ')
    if [ "$DL_SIZE" = "$FILE_SIZE" ]; then
        pass "Downloaded size matches ($DL_SIZE bytes)"
    else
        fail "Size mismatch: expected $FILE_SIZE, got $DL_SIZE"
    fi
else
    fail "GET returned $GET_RESP (expected 200)"
fi
echo ""

# ── Test 3: HEAD with Accept-Ranges ──────────────────────────────────────────

echo "Test 3: HEAD /media/${SHA256}.mp4 (Accept-Ranges)"
HEAD_RESP=$(curl -s -I "$RELAY_URL/media/${SHA256}.mp4")
HEAD_HTTP=$(echo "$HEAD_RESP" | head -1 | grep -o '[0-9]\{3\}')
ACCEPT_RANGES=$(echo "$HEAD_RESP" | grep -i "accept-ranges" | tr -d '\r')

if [ "$HEAD_HTTP" = "200" ]; then
    pass "HEAD returned 200"
else
    fail "HEAD returned $HEAD_HTTP (expected 200)"
fi

if echo "$ACCEPT_RANGES" | grep -qi "bytes"; then
    pass "Accept-Ranges: bytes present"
else
    fail "Accept-Ranges header missing or wrong: '$ACCEPT_RANGES'"
fi
echo ""

# ── Test 4: Range GET (206 Partial Content) ──────────────────────────────────

echo "Test 4: Range GET bytes=0-499 (206 Partial Content)"
RANGE_RESP=$(curl -s -o "$TMPDIR/range.bin" -w "%{http_code}" \
    -H "Range: bytes=0-499" \
    "$RELAY_URL/media/${SHA256}.mp4")

if [ "$RANGE_RESP" = "206" ]; then
    pass "Range GET returned 206"
    RANGE_SIZE=$(wc -c < "$TMPDIR/range.bin" | tr -d ' ')
    if [ "$RANGE_SIZE" = "500" ]; then
        pass "Received exactly 500 bytes"
    else
        fail "Expected 500 bytes, got $RANGE_SIZE"
    fi
else
    fail "Range GET returned $RANGE_RESP (expected 206)"
fi
echo ""

# ── Test 5: Range GET past EOF (416) ─────────────────────────────────────────

echo "Test 5: Range GET bytes=999999999- (416 Range Not Satisfiable)"
RANGE416_RESP=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Range: bytes=999999999-" \
    "$RELAY_URL/media/${SHA256}.mp4")

if [ "$RANGE416_RESP" = "416" ]; then
    pass "Past-EOF range returned 416"
else
    fail "Past-EOF range returned $RANGE416_RESP (expected 416)"
fi
echo ""

# ── Test 6: Content-Type spoofing rejection ──────────────────────────────────
# Send video/mp4 Content-Type but with a PNG body — should be rejected.

echo "Test 6: Content-Type spoofing (video/mp4 header, PNG body)"
PNG_FILE="$TMPDIR/fake.png"
# Minimal valid PNG (1x1 red pixel)
printf '\x89PNG\r\n\x1a\n' > "$PNG_FILE"
dd if=/dev/zero bs=100 count=1 >> "$PNG_FILE" 2>/dev/null

PNG_SHA=$(shasum -a 256 "$PNG_FILE" | cut -d' ' -f1)
SPOOF_AUTH="$(blossom_auth "$PNG_SHA")"

SPOOF_RESP=$(curl -s -w "\n%{http_code}" \
    -X PUT "$RELAY_URL/media/upload" \
    -H "Authorization: $SPOOF_AUTH" \
    -H "Content-Type: video/mp4" \
    -H "X-SHA-256: $PNG_SHA" \
    --data-binary "@$PNG_FILE")

SPOOF_HTTP=$(echo "$SPOOF_RESP" | tail -1)

if [ "$SPOOF_HTTP" = "415" ] || [ "$SPOOF_HTTP" = "400" ]; then
    pass "Spoofed upload rejected with $SPOOF_HTTP"
else
    fail "Spoofed upload returned $SPOOF_HTTP (expected 400 or 415)"
fi
echo ""

# ── Test 7: Idempotent re-upload ─────────────────────────────────────────────

echo "Test 7: Idempotent re-upload (same file, same hash)"
REUP_AUTH="$(blossom_auth "$SHA256")"
REUP_RESP=$(curl -s -w "\n%{http_code}" \
    -X PUT "$RELAY_URL/media/upload" \
    -H "Authorization: $REUP_AUTH" \
    -H "Content-Type: video/mp4" \
    -H "X-SHA-256: $SHA256" \
    --data-binary "@$TEST_MP4")

REUP_HTTP=$(echo "$REUP_RESP" | tail -1)
if [ "$REUP_HTTP" = "200" ]; then
    pass "Re-upload returned 200 (idempotent)"
else
    fail "Re-upload returned $REUP_HTTP (expected 200)"
fi
echo ""

# ── Test 8: Poster frame upload + imeta validation ───────────────────────────
# Upload a JPEG poster frame, then verify the server accepts an imeta tag
# that links the video and poster via the NIP-71 `image` field.

echo "Test 8: Upload poster frame (JPEG image)"
POSTER_JPG="$TMPDIR/poster.jpg"
ffmpeg -y -ss 0.5 -i "$TEST_MP4" -vframes 1 -vf "scale=640:-2" -q:v 2 \
    "$POSTER_JPG" 2>/dev/null

if [ ! -s "$POSTER_JPG" ]; then
    # Fallback: first frame (video may be too short for 0.5s seek)
    ffmpeg -y -i "$TEST_MP4" -vframes 1 -vf "scale=640:-2" -q:v 2 \
        "$POSTER_JPG" 2>/dev/null
fi

POSTER_SIZE=$(wc -c < "$POSTER_JPG" | tr -d ' ')
POSTER_SHA=$(shasum -a 256 "$POSTER_JPG" | cut -d' ' -f1)
echo "  Poster: ${POSTER_SIZE} bytes, sha256=${POSTER_SHA:0:16}..."

POSTER_AUTH="$(blossom_auth "$POSTER_SHA")"
POSTER_RESP=$(curl -s -w "\n%{http_code}" \
    -X PUT "$RELAY_URL/media/upload" \
    -H "Authorization: $POSTER_AUTH" \
    -H "Content-Type: image/jpeg" \
    -H "X-SHA-256: $POSTER_SHA" \
    --data-binary "@$POSTER_JPG")

POSTER_HTTP=$(echo "$POSTER_RESP" | tail -1)
POSTER_BODY=$(echo "$POSTER_RESP" | sed '$d')

if [ "$POSTER_HTTP" = "200" ]; then
    pass "Poster upload returned 200"
    POSTER_URL=$(echo "$POSTER_BODY" | jq -r '.url // empty')
    if [ -n "$POSTER_URL" ]; then
        pass "Poster has url: ${POSTER_URL:0:60}..."
    else
        fail "Poster response missing url"
    fi
    # Poster should have dim but NOT duration
    POSTER_DIM=$(echo "$POSTER_BODY" | jq -r '.dim // empty')
    POSTER_DUR=$(echo "$POSTER_BODY" | jq -r '.duration // empty')
    if [ -n "$POSTER_DIM" ]; then
        pass "Poster has dim: $POSTER_DIM"
    else
        fail "Poster missing dim"
    fi
    if [ -z "$POSTER_DUR" ]; then
        pass "Poster correctly omits duration"
    else
        fail "Poster should not have duration, got: $POSTER_DUR"
    fi
else
    fail "Poster upload returned $POSTER_HTTP (expected 200)"
    echo "    Body: $POSTER_BODY"
fi
echo ""

# ── Test 9: GET poster frame ─────────────────────────────────────────────────

echo "Test 9: GET poster frame"
POSTER_GET_RESP=$(curl -s -o "$TMPDIR/poster_dl.jpg" -w "%{http_code}" \
    "$RELAY_URL/media/${POSTER_SHA}.jpg")

if [ "$POSTER_GET_RESP" = "200" ]; then
    pass "GET poster returned 200"
    POSTER_DL_SIZE=$(wc -c < "$TMPDIR/poster_dl.jpg" | tr -d ' ')
    if [ "$POSTER_DL_SIZE" = "$POSTER_SIZE" ]; then
        pass "Poster download size matches ($POSTER_DL_SIZE bytes)"
    else
        fail "Poster size mismatch: expected $POSTER_SIZE, got $POSTER_DL_SIZE"
    fi
else
    fail "GET poster returned $POSTER_GET_RESP (expected 200)"
fi
echo ""

# ── Test 10: Video + poster blobs coexist and are independently retrievable ──
# The server links video and poster purely through the imeta tag at message
# send time. Here we verify the prerequisite: both blobs exist, have correct
# Content-Types, and are independently addressable.

echo "Test 10: Video + poster blobs coexist"
VIDEO_HEAD=$(curl -s -o /dev/null -w "%{http_code}" -I "$RELAY_URL/media/${SHA256}.mp4")
POSTER_HEAD=$(curl -s -o /dev/null -w "%{http_code}" -I "$RELAY_URL/media/${POSTER_SHA}.jpg")

if [ "$VIDEO_HEAD" = "200" ] && [ "$POSTER_HEAD" = "200" ]; then
    pass "Both video and poster blobs exist (HEAD 200)"
else
    fail "Blob existence check failed: video=$VIDEO_HEAD poster=$POSTER_HEAD"
fi

# Verify poster is an image (not video) by checking Content-Type
POSTER_CT=$(curl -s -I "$RELAY_URL/media/${POSTER_SHA}.jpg" | grep -i "content-type" | tr -d '\r' | awk '{print $2}')
if echo "$POSTER_CT" | grep -qi "image/jpeg"; then
    pass "Poster Content-Type is image/jpeg"
else
    fail "Poster Content-Type should be image/jpeg, got: $POSTER_CT"
fi

# Verify video and poster have different hashes (independent blobs)
if [ "$SHA256" != "$POSTER_SHA" ]; then
    pass "Video and poster have distinct content hashes"
else
    fail "Video and poster hashes should differ"
fi
echo ""

# ── Test 11: Poster sidecar has correct metadata ─────────────────────────────
# The server writes a JSON sidecar for every uploaded blob. Verify the poster
# sidecar exists and has image MIME type (the server's verify_imeta_blobs
# checks this at message send time).

echo "Test 11: Poster sidecar metadata"
# The bare-hash GET resolves via sidecar — if it returns 200 with image/jpeg
# content-type, the sidecar is correctly configured.
POSTER_BARE_RESP=$(curl -s -D "$TMPDIR/poster_bare_headers.txt" -o /dev/null -w "%{http_code}" \
    "$RELAY_URL/media/${POSTER_SHA}")
if [ "$POSTER_BARE_RESP" = "200" ]; then
    pass "Poster bare-hash GET resolves via sidecar (200)"
else
    fail "Poster bare-hash GET returned $POSTER_BARE_RESP (expected 200)"
fi

# Verify Content-Type on the bare-hash response
POSTER_BARE_CT=$(grep -i "content-type" "$TMPDIR/poster_bare_headers.txt" | tr -d '\r' | awk '{print $2}')
if echo "$POSTER_BARE_CT" | grep -qi "image/jpeg"; then
    pass "Poster bare-hash Content-Type is image/jpeg"
else
    fail "Poster bare-hash Content-Type should be image/jpeg, got: $POSTER_BARE_CT"
fi

# Note: Full imeta image validation (accept/reject at message send time) is
# covered by Rust unit tests: test_imeta_image_poster_frame_accepted,
# test_imeta_image_video_url_rejected, test_imeta_image_thumbnail_url_rejected.
echo ""

# ── Summary ───────────────────────────────────────────────────────────────────

echo "════════════════════════════════════════"
echo "  Results: $PASS passed, $FAIL failed"
echo "════════════════════════════════════════"

[ "$FAIL" -eq 0 ] && exit 0 || exit 1
