# buzz-proxy

NIP-28 compatibility proxy for [Buzz](../../VISION.md). Lets standard Nostr clients (Coracle, Amethyst, nak, etc.) connect to a Buzz relay using familiar NIP-28 channel events.

```
Client (NIP-28)  ←→  buzz-proxy :4869  ←→  buzz-relay :3000
```

**Supported NIPs:** NIP-01, NIP-11, NIP-28, NIP-42

**Not supported (MVP):** NIP-29 group navigation, NIP-50 search, DMs

---

## Quick Start

### 1. Start infrastructure and relay

```bash
just relay    # starts Docker services + migrations automatically
```

### 2. Mint a proxy API token

The token's `--pubkey` must match the public key derived from `BUZZ_PROXY_SERVER_KEY`.

```bash
# Derive the public key from your server key
nak key public <BUZZ_PROXY_SERVER_KEY>

# Mint the token with that pubkey
cargo run -p buzz-admin -- mint-token \
  --name "buzz-proxy" \
  --scopes "proxy:submit,channels:read,messages:read" \
  --pubkey <derived-pubkey>
```

### 3. Configure environment

```bash
export BUZZ_UPSTREAM_URL=ws://localhost:3000
export BUZZ_PROXY_BIND_ADDR=0.0.0.0:4869
export BUZZ_PROXY_SERVER_KEY=<hex nsec from step 2>
export BUZZ_PROXY_SALT=$(openssl rand -hex 32)
export BUZZ_PROXY_API_TOKEN=<api token from step 2>
export BUZZ_PROXY_ADMIN_SECRET=<secret for admin API>
```

> Put these in `.env` at the repo root — `just` loads it automatically.

### 4. Start the proxy

```bash
just proxy    # Proxy on :4869
```

### 5. Register a guest (recommended)

Register a guest by their Nostr public key. They can then connect with any NIP-42-capable client — no token needed.

```bash
curl -X POST http://localhost:4869/admin/guests \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <BUZZ_PROXY_ADMIN_SECRET>" \
  -d '{"pubkey":"<guest-hex-pubkey>","channels":"<channel-uuid1>,<channel-uuid2>"}'
```

### 6. Connect (pubkey-based)

Just add the proxy as a relay in your Nostr client:

```
ws://localhost:4869
```

The client handles NIP-42 authentication automatically. No token in the URL.

### Alternative: Invite tokens (for ad-hoc sharing)

```bash
curl -X POST http://localhost:4869/admin/invite \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <BUZZ_PROXY_ADMIN_SECRET>" \
  -d '{"channels":"<channel-uuid>","hours":24,"max_uses":10}'
```

Connect with the token in the URL:

```
ws://localhost:4869?token=<invite_token>
```

---

## Environment Variables

| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `BUZZ_UPSTREAM_URL` | ✅ | — | WebSocket URL of the Buzz relay |
| `BUZZ_PROXY_SERVER_KEY` | ✅ | — | Hex nsec for the proxy server keypair |
| `BUZZ_PROXY_SALT` | ✅ | — | Hex 32-byte salt for shadow key derivation (keep stable) |
| `BUZZ_PROXY_API_TOKEN` | ✅ | — | Buzz API token with `proxy:submit`, `channels:read`, and `messages:read` scopes |
| `BUZZ_PROXY_BIND_ADDR` | ❌ | `0.0.0.0:4869` | Listen address |
| `BUZZ_PROXY_ADMIN_SECRET` | ❌ | — | Bearer secret for `POST /admin/invite` (unset = dev mode, no auth) |
| `RUST_LOG` | ❌ | `buzz_proxy=info` | Log level |

---

## API Endpoints

### `GET /`
- With `Accept: application/nostr+json` → NIP-11 relay info document
- With `Upgrade: websocket` → WebSocket connection (requires `?token=<invite>`)

### `POST /admin/invite`

Create an invite token. Requires `Authorization: Bearer <BUZZ_PROXY_ADMIN_SECRET>` (if secret is set).

**Request:**
```json
{
  "channels": "<uuid1>,<uuid2>",
  "hours": 24,
  "max_uses": 10
}
```

**Response:**
```json
{
  "token": "buzz_invite_<uuid>",
  "channels": ["<uuid1>", "<uuid2>"],
  "expires_at": "2026-03-12T22:00:00Z",
  "max_uses": 10
}
```

---

## Recommended Clients

| Client | Platform | NIP-28 | NIP-42 | Notes |
|--------|----------|:------:|:------:|-------|
| **Coracle** | Web | ✅ | ✅ | Best UI — renders kind:42 in chat |
| **Nostrudel** | Web | ✅ | ✅ | Good NIP-28 support |
| **Amethyst** | Android | ✅ | ✅ | NIP-28 public chat works |
| **nak** | CLI | ✅ | ✅ | Best for scripting/testing |
| **websocat** | CLI | ✅ | — | Raw protocol debugging |
| **Damus** | iOS | ❌ | ✅ | No NIP-28 UI |
| **Primal** | All | ❌ | ❌ | Incompatible (caching relay) |

---

## Testing with nak

```bash
# Install
go install github.com/fiatjaf/nak@latest

# List channels
nak req -k 40 -l 10 --auth "ws://localhost:4869?token=<invite_token>"

# Subscribe to messages
nak req -k 42 --tag "e=<kind40_event_id>" -l 20 --auth "ws://localhost:4869?token=<invite_token>"

# Post a message
nak event -k 42 -c "Hello!" --tag "e=<kind40_event_id>" --sec <nsec> "ws://localhost:4869?token=<invite_token>"
```

## E2E Test Script

```bash
# Prerequisites: relay + proxy running, websocat + curl + jq installed
./scripts/test-proxy-e2e.sh
```

---

## How It Works

- **Kind translation:** kind:42 ↔ kind:9, `#e(event_id)` ↔ `#h(uuid)`
- **Shadow keys:** Each external pubkey gets a deterministic shadow keypair (HMAC-SHA256 of salt + pubkey). The relay sees the shadow key; the external user's real key is never exposed.
- **Channel map:** Loaded at startup from the relay's REST API. kind:40/41 events are synthesized locally — never forwarded upstream.
- **Invite tokens:** In-memory, scoped to specific channels, time-limited, use-limited. Lost on proxy restart.
- **`proxy:submit` scope:** Allows the proxy's API token to submit shadow-signed events through the relay's pubkey enforcement.
- **Pre-auth buffering:** REQ messages sent before NIP-42 auth completes are buffered (max 20 msgs / 64 KiB) and replayed after auth.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `"error: invite token not found"` | Token expired, used up, or proxy restarted | Create new token via `POST /admin/invite` |
| `"auth-required: authentication timeout"` | Client didn't respond to AUTH challenge within 30s | Use a NIP-42-capable client |
| `"error: channel not found"` | Channel created after proxy started | Restart proxy to refresh channel map |
| Connection drops immediately | Relay not running or wrong `BUZZ_UPSTREAM_URL` | Check `just relay` is running |
| No messages appearing | Wrong kind:40 event ID in subscription | Re-query kind:40 to get correct event ID |
| Startup fails: "failed to initialize channel map" | Can't reach relay REST API | Check relay health and API token scopes |

---

For the full guide including client-specific setup, architecture details, and extended troubleshooting, see [GUIDES/NOSTR_CLIENT_GUIDE.md](../../GUIDES/NOSTR_CLIENT_GUIDE.md).
