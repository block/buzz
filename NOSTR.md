# Using Third-Party Nostr Clients with Buzz

Buzz is a Nostr relay that speaks NIP-29 (relay-based groups) natively. There are two ways for
third-party Nostr clients to connect:

| Path | Protocol | Connects to | Expected clients (not all verified in-repo) |
|------|----------|-------------|----------------------------------------------|
| **Direct** | NIP-29 | `buzz-relay :3000` | NIP-29 clients (e.g. Chachi, 0xchat), nak |
| **Via proxy** | NIP-28 | `buzz-proxy :4869` | NIP-28 clients (e.g. Coracle, Amethyst), nostr-tools apps |

**Direct** is simpler — no extra process, no translation layer. Use it when your client speaks
NIP-29. **Proxy** is for external guests (investors, press, partners, etc.) who use standard NIP-28
clients and don't have company credentials.

Both paths require NIP-42 authentication.

---

## Path 1: NIP-29 Direct

Connect any NIP-29 client straight to the relay.

### Quick Start

```bash
# 1. (Optional) Enable pubkey allowlist — must be set BEFORE relay startup
export BUZZ_PUBKEY_ALLOWLIST=true

# 2. Start the relay (auto-starts Docker services and runs migrations)
just relay &                         # relay on :3000

# 3. Add a pubkey to the allowlist (if enabled)
#    Insert directly — there is no CLI command for this yet.
PGPASSWORD=buzz_dev psql -h localhost -U buzz -d buzz -c \
  "INSERT INTO pubkey_allowlist (pubkey) VALUES (decode('<64-char-hex-pubkey>', 'hex'))"

# 5. Connect any NIP-29 + NIP-42 client to ws://localhost:3000
```

### What Works

| Feature | Status | Notes |
|---------|:------:|-------|
| **Group chat (kind:9)** | ✅ | Send/receive messages with `#h <channel-uuid>` tag |
| **Reactions (kind:7)** | ✅ | Standard NIP-25; channel derived from target event's `#e` tag (client `#h` ignored) |
| **Deletions (kind:5)** | ✅ | Standard NIP-09; self-authored only. `#h` optional, `#e` required |
| **User profiles (kind:0)** | ✅ | NIP-01 metadata; synced to users table (display_name, avatar, about, NIP-05). NIP-05 handles must canonicalize to this relay's domain — off-domain or invalid handles are silently cleared. If a NIP-05 handle collides with another user's (UNIQUE constraint), the handle is skipped but other profile fields (display_name, avatar, about) are still synced. |
| **Group creation (kind:9007)** | ✅ | NIP-29; include `name` tag, optional `visibility` and `channel_type` |
| **Add user (kind:9000)** | ✅ | Open: any user, subject to target's `channel_add_policy` (`owner_only`/`nobody` can block). Private: owner/admin only. Self-add bypasses agent policy but not private-channel auth. |
| **Remove user (kind:9001)** | ✅ | Self-remove allowed (with last-owner guard). Removing others: owner/admin only. |
| **Edit group metadata (kind:9002)** | ✅ | `name`/`about` tags: owner/admin. `topic`/`purpose` tags: any member. |
| **Admin delete event (kind:9005)** | ✅ | Event author can always delete own. Otherwise owner/admin required. Target must be in same channel. |
| **Group deletion (kind:9008)** | ✅ | Owner only. |
| **Leave group (kind:9022)** | ✅ | Any member. Last-owner guard prevents orphaned groups. |
| **Group metadata (kind:39000)** | ✅ | Relay-signed; always `d`, `name`, `closed` tags; `about` only if description non-empty; `private` if applicable; `hidden` for DM channels |
| **Group admins (kind:39001)** | ✅ | Relay-signed; `d` tag + `p` tags with roles (`owner`, `admin`) |
| **Group members (kind:39002)** | ✅ | Relay-signed; `d` tag + `p` tags for all members |
| **Membership notifications** | ✅ | kind:44100 (added) / kind:44101 (removed); relay-signed, global scope |
| **Presence (kind:20001)** | ✅ | Ephemeral; arbitrary status string (truncated to 128 chars); writes to Redis (`set_presence`/`clear_presence` on `"offline"`), then fan-out to local subscribers |
| **Typing indicators (kind:20002)** | ✅ | Ephemeral, not stored; published via Redis pub/sub (multi-node capable unlike presence fan-out) |
| **NIP-42 authentication** | ✅ | Proactive challenge; optional pubkey allowlist |
| **NIP-11 relay info** | ✅ | `GET /` with `Accept: application/nostr+json` |
| **Blossom media** | ✅ | `PUT /media/upload` (BUD-02), `GET /media/{sha256}.{ext}` (BUD-01) |
| **NIP-50 search** | ✅ | One-shot search REQs: `{"search":"query","kinds":[9],"#h":["<uuid>"]}` → relevance-sorted results → EOSE. Not registered as persistent subscriptions. |
| **NIP-10 threads** | ✅ | WS-submitted replies with `["e","<root>","","reply"]` tags create `thread_metadata` atomically. Visible in REST thread queries. Unknown parents rejected. |
| **NIP-17 DMs (gift wrap)** | ✅ | kind:1059 accepted with ephemeral signing keys. Stored globally (channel_id=None). Delivered via `#p`-filtered subscriptions. Not indexed in search. |
| **DM discovery** | ✅ | DM creation emits kind:39000 (with `hidden` tag) + kind:44100 membership notifications. NIP-29 clients discover DMs via standard group discovery flow. |
| **Join request (kind:9021)** | ✅ | Open channels only. Adds member, emits system message + group discovery events + kind:44100 membership notification. Private channels rejected at ingest. |
| **Edits (kind:40003)** | ⚠️ | Works on the wire but Buzz-only — no standard NIP-29 client renders these |
| **Rich content (kind:40002)** | ⚠️ | Works on the wire but Buzz-only — no standard NIP-29 client renders these |

### What Doesn't Work

| Feature | Status | Why |
|---------|:------:|-----|
| **Create invite (kind:9009)** | ⚠️ | Accepted and stored, but side-effect handler is deferred (no-op with warning log) |
| **Group roles (kind:39003)** | ❌ | Defined in kind registry but not emitted by the relay |
| **DMs** | ⚠️ | NIP-17 gift wraps supported; NIP-04/NIP-44 not implemented. kind:10050 (DM relay list) deferred. |

### Pubkey Allowlist

When `BUZZ_PUBKEY_ALLOWLIST=true`, NIP-42 connections that authenticate with only a pubkey
(no API token) are checked against the `pubkey_allowlist` table. This lets you open the
relay to specific external Nostr identities without granting full access.

- Users with valid **API tokens** bypass the allowlist.
- **Fail-closed:** if the DB lookup fails, the connection is denied.
- Default: `false` (all authenticated pubkeys accepted).
- Auth failure returns generic `auth-required: verification failed` (no allowlist-specific message).
- Manage the allowlist via direct SQL (no CLI command yet):
  ```sql
  INSERT INTO pubkey_allowlist (pubkey) VALUES (decode('<64-char-hex-pubkey>', 'hex'));
  DELETE FROM pubkey_allowlist WHERE pubkey = decode('<64-char-hex-pubkey>', 'hex');
  SELECT encode(pubkey, 'hex'), added_at, note FROM pubkey_allowlist;
  ```

### Group Discovery

The relay emits NIP-29 group state events when channels are created, updated, or membership changes.
All discovery events include a `d` tag set to the channel UUID (NIP-29 addressable event convention):

| Kind | Tags | Content |
|------|------|---------|
| **39000** | `d=<uuid>`, `name`, `closed` (always); `about` (if description non-empty); `private` (if applicable); `hidden` (DM channels only) | Group metadata. **Note:** `closed` is always emitted per NIP-29 convention (Buzz channels require explicit membership), but open channels are still readable/writable by non-members at runtime. The tag reflects the membership model, not access enforcement. |
| **39001** | `d=<uuid>`, `p` tags with role label (`owner`, `admin`) | Admin list |
| **39002** | `d=<uuid>`, `p` tags for all members | Member list |

Events are stored **channel-scoped** so access control applies — private channel member lists are
only visible to members. Discover groups via historical REQ:

```bash
# All groups you can see
nak req -k 39000 --auth --sec <privkey> ws://localhost:3000

# Specific group's members (filter by d tag)
nak req -k 39002 --tag "d=<channel-uuid>" --auth --sec <privkey> ws://localhost:3000
```

> **Note:** Channel-scoped storage means live global subscriptions (`{kinds:[39000]}`) won't
> receive these via fan-out. Clients discover groups via historical REQ queries. Live push for
> open-channel discovery is a future enhancement.

### Membership Notifications

The relay emits relay-signed notifications when members are added or removed:

| Kind | Meaning | Tags | Scope |
|------|---------|------|-------|
| **44100** | Member added | `p` = target pubkey, `h` = channel UUID | Global |
| **44101** | Member removed | `p` = target pubkey, `h` = channel UUID | Global |

Stored globally (`channel_id = None`) so agents and clients can subscribe without knowing channel
UUIDs in advance. Client-submitted kind:44100/44101 events are rejected — only the relay keypair
may sign these.

> **Subscription constraint:** Global REQs that can match p-gated kinds (44100, 44101, 1059) **must**
> include a `#p` filter where **all** `#p` values match the authenticated pubkey. The relay rejects
> subscriptions that omit `#p` or include other pubkeys (prevents eavesdropping on others' membership
> changes and DMs). Error: `restricted: p-gated events require #p matching your pubkey`.

```bash
nak req -k 44100 -k 44101 --tag "p=<your-hex-pubkey>" \
  --auth --sec <privkey> ws://localhost:3000
```

### Sending Messages

```bash
# Send a kind:9 message
nak event -k 9 -c "Hello from NIP-29!" --tag "h=<channel-uuid>" \
  --auth --sec <privkey> ws://localhost:3000

# Subscribe to channel messages
nak req -k 9 --tag "h=<channel-uuid>" --stream \
  --auth --sec <privkey> ws://localhost:3000

# React to a message (#h optional but recommended; channel derived from #e target)
nak event -k 7 -c "+" --tag "h=<channel-uuid>" --tag "e=<message-event-id>" \
  --auth --sec <privkey> ws://localhost:3000

# Delete a message (#h optional; #e required; must be self-authored)
nak event -k 5 -c "reason" --tag "h=<channel-uuid>" --tag "e=<message-event-id>" \
  --auth --sec <privkey> ws://localhost:3000

# Create a group
nak event -k 9007 --tag "name=my-channel" --tag "visibility=open" \
  --auth --sec <privkey> ws://localhost:3000

# Search messages (NIP-50)
nak req -k 9 --tag "h=<channel-uuid>" --search "search query" -l 20 \
  --auth --sec <privkey> ws://localhost:3000

# Reply to a message (NIP-10 threading)
nak event -k 9 -c "Reply text" --tag "h=<channel-uuid>" \
  --tag "e=<parent-event-id>;;reply" \
  --auth --sec <privkey> ws://localhost:3000

# Fetch gift-wrapped DMs (NIP-17)
nak req -k 1059 --tag "p=<your-hex-pubkey>" \
  --auth --sec <privkey> ws://localhost:3000
```

### Tested Clients (Direct)

| Client | Platform | Evidence | Notes |
|--------|----------|:--------:|-------|
| **BuzzTestClient** | Rust (repo) | Automated E2E | Full NIP-29 flow: discovery (39000/39001/39002), kind:9 send/receive, reactions, deletions, h-tag enforcement |
| **E2E nostr interop** | Rust (repo) | Automated E2E | NIP-50 search (3 tests), NIP-10 threads (3 tests), NIP-17 gift wraps (3 tests), DM discovery (1 test) |
| **nak** | CLI | Manual (verified) | kind:9 send/recv, NIP-50 search, NIP-10 thread replies, group discovery |

**Not verified in-repo** (anecdotal / expected based on NIP-29 support):
- **Chachi** (Web/Mobile) — NDK-based; NIP-29 native
- **0xchat** (Mobile) — NIP-29 native

---

## Path 2: NIP-28 via buzz-proxy

For clients that speak NIP-28 (kind:40/41/42) but not NIP-29, **buzz-proxy** translates between
the two protocols in real time. Events are re-signed with deterministic shadow keys so each
external user maps to a consistent identity on the relay.

### Quick Start

```bash
# 1. Start infrastructure + relay (see Path 1)

# 2. Generate proxy server key and derive its pubkey
export BUZZ_PROXY_SERVER_KEY=$(openssl rand -hex 32)
PROXY_PUBKEY=$(echo $BUZZ_PROXY_SERVER_KEY | nak key public)

# 3. Mint a proxy API token (required until proxy is migrated to NIP-98 auth)
export BUZZ_PROXY_API_TOKEN=$(curl -s -X POST http://localhost:3000/api/tokens \
  -H "Authorization: Nostr <base64-nip98-event>" \
  -H "Content-Type: application/json" \
  -d '{"name":"proxy"}' | jq -r .token)

# 4. Get the relay's public key (needed for attribution trust)
#    This is the pubkey of the relay's signing keypair. If BUZZ_RELAY_PRIVATE_KEY
#    is set, derive it: echo $BUZZ_RELAY_PRIVATE_KEY | nak key public
#    If not set, the relay generates a random keypair at startup — check relay logs.
export BUZZ_RELAY_PUBKEY=<relay-hex-pubkey>

# 5. Start the proxy
export BUZZ_UPSTREAM_URL=ws://localhost:3000
export BUZZ_PROXY_SALT=$(openssl rand -hex 32)
export BUZZ_PROXY_ADMIN_SECRET=$(openssl rand -hex 16)
cargo run -p buzz-proxy             # proxy on :4869

# 6. Register a guest
curl -X POST http://localhost:4869/admin/guests \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BUZZ_PROXY_ADMIN_SECRET" \
  -d '{"pubkey": "<guest-hex-pubkey>", "channels": "<channel-uuid>"}'

# 7. Connect any NIP-28 + NIP-42 client to ws://localhost:4869
```

### What Works

| Feature | Status | Notes |
|---------|:------:|-------|
| **NIP-11 relay info** | ✅ | Standard relay info document at `GET /` |
| **NIP-42 authentication** | ✅ | Proactive challenge + reactive-auth compatible |
| **Channel discovery (kind:40)** | ✅ | Synthesized from Buzz REST API at startup; served locally. Content uses channel UUID as `name` for ID stability; human-readable name is in kind:41. **Snapshot — new channels created after proxy start require restart. Renames do NOT affect kind:40 (UUID-anchored).** |
| **Channel metadata (kind:41)** | ✅ | Name, description (picture always empty — no channel-picture source in proxy path); synthesized at startup, served locally. **Snapshot — new channels AND renames require restart to update local kind:41.** |
| **Channel messages (kind:42)** | ✅ | Translated to/from Buzz kind:9 |
| **Inbound kind:1** | ✅ | Text notes (kind:1) accepted and translated to kind:9, same as kind:42 |
| **Message editing (kind:41)** | ✅ | Bidirectional: inbound kind:41 → kind:40003; outbound kind:40003 → kind:41. **Note:** inbound kind:41 is always treated as a message edit, never as a channel metadata update. Standard NIP-28 channel-metadata writes are not supported. |
| **Reactions (kind:7)** | ✅ | Bidirectional; inbound channel scope verified against allowed channels. **Constraint:** target must already be known to the proxy's ID mapping cache (populated by prior fetch, outbound delivery, or inbound publish). Error if unknown: `reaction target is unknown to the proxy; fetch the message first`. |
| **Deletions (kind:5)** | ⚠️ | **Outbound only** — standard kind:5 events stored on the relay are translated for clients. Admin deletions (kind:9005) and REST-API deletes soft-delete without emitting kind:5, so proxy clients won't see those. Inbound kind:5 blocked by proxy policy (not yet implemented). |
| **Real-time streaming** | ✅ | Live event delivery via open subscriptions |
| **Multi-channel access** | ✅ | Guests can be granted access to multiple channels |
| **Shadow identity** | ✅ | Each guest gets a deterministic shadow keypair |

> **kind:41 dual semantics:** A `REQ` for kind:41 returns both locally-synthesized channel metadata
> (startup snapshot) AND upstream edit events (kind:40003 translated to kind:41). Clients may see
> two different event types under the same kind number. Inbound kind:41 is always treated as a
> message edit (→ kind:40003), never as a channel metadata update.

### What Doesn't Work

| Feature | Status | Why |
|---------|:------:|-----|
| **Channel creation (kind:40 write)** | ❌ | Channels created via REST API or NIP-29 kind:9007 (direct path) |
| **Inbound deletions (kind:5)** | ❌ | Blocked by proxy policy; not yet implemented |
| **DMs (NIP-04/NIP-44)** | ❌ | Proxy only handles NIP-28 channel events |
| **User profiles (kind:0)** | ❌ | Profiles managed via REST API or kind:0 (direct path) |
| **NIP-10 reply threading** | ⚠️ | Threading works on direct path; proxy preserves `#e` tags but does not translate thread metadata |
| **NIP-50 search** | ❌ | Available on direct path only (ws://relay:3000); not proxied |
| **File uploads (NIP-94/96)** | ❌ | Use Blossom on the relay directly (Path 1) |
| **Relay lists / Outbox (NIP-65)** | ❌ | Single-relay architecture |

### Channel UUIDs vs Event IDs

Buzz identifies channels by UUID. NIP-28 clients identify channels by the event ID of the
kind:40 creation event. The proxy translates automatically, but you need the event ID to subscribe.

The synthesized kind:40 uses the channel UUID as the `name` field in content (for deterministic
event ID stability across restarts). The human-readable channel name is in kind:41 metadata:

```bash
# Get kind:40 (UUID in content.name) and kind:41 (human-readable name)
nak req -k 40 -k 41 --auth --sec <privkey> ws://localhost:4869
```

### Proxy Authentication

Two methods, both using NIP-42:

**Pubkey-based (primary)** — register a guest's hex pubkey with channel access:

```bash
# Register
curl -X POST http://localhost:4869/admin/guests \
  -H "Authorization: Bearer $BUZZ_PROXY_ADMIN_SECRET" \
  -H "Content-Type: application/json" \
  -d '{"pubkey": "<hex>", "channels": "<uuid1>,<uuid2>"}'

# List
curl http://localhost:4869/admin/guests \
  -H "Authorization: Bearer $BUZZ_PROXY_ADMIN_SECRET"

# Revoke
curl -X DELETE http://localhost:4869/admin/guests \
  -H "Authorization: Bearer $BUZZ_PROXY_ADMIN_SECRET" \
  -H "Content-Type: application/json" \
  -d '{"pubkey": "<hex>"}'
```

> **Private channels:** The proxy authenticates upstream using its own server key via NIP-42.
> `GET /api/channels` and relay REQ filters only return channels accessible to that identity.
> For the proxy to expose a private channel, the proxy's server pubkey must itself be a member
> of that channel. Guest registration alone is not sufficient for private channels.

**Invite tokens (secondary)** — for ad-hoc sharing with expiry and use limits:

```bash
# Create
curl -X POST http://localhost:4869/admin/invite \
  -H "Authorization: Bearer $BUZZ_PROXY_ADMIN_SECRET" \
  -H "Content-Type: application/json" \
  -d '{"channels": "<uuid1>,<uuid2>", "max_uses": 5, "hours": 48}'

# Connect: ws://localhost:4869?token=<invite_token>
```

### Connecting with Coracle (expected, not verified in-repo)

1. Open **https://coracle.social**.
2. Note your hex pubkey from **Settings → Account**.
3. Register it: `POST /admin/guests` with your pubkey and channel UUIDs.
4. **Settings → Relays → Add Relay** → `ws://localhost:4869`
5. Coracle should handle NIP-42 auth automatically. Channels should appear under **Public Channels**.

For remote access, tunnel with ngrok: `ngrok http 4869` → use `wss://<subdomain>.ngrok.io`.

### Connecting with nak (Proxy)

```bash
# Discover channels
nak req -k 40 -l 10 --auth --sec <privkey> ws://localhost:4869

# Read messages from a specific channel
nak req -k 42 --tag "e=<kind40-event-id>" -l 10 --auth --sec <privkey> ws://localhost:4869

# Send a message
nak event -k 42 -c "Hello!" --tag e=<kind40-event-id> \
  --auth --sec <privkey> ws://localhost:4869

# Stream live from a specific channel
nak req -k 42 --tag "e=<kind40-event-id>" --stream --auth --sec <privkey> ws://localhost:4869
```

### Connecting with nostr-tools v2.23

```javascript
import { Relay } from 'nostr-tools/relay'
import { finalizeEvent } from 'nostr-tools/pure'
import { channelMessageEvent } from 'nostr-tools/nip28'

const relay = new Relay('ws://localhost:4869', { websocketImplementation: WebSocket })
relay.onauth = async (template) => finalizeEvent(template, secretKey)
await relay.connect()

const event = channelMessageEvent({
  channel_create_event_id: '<kind:40 event ID>',
  relay_url: 'ws://localhost:4869',
  content: 'Hello from nostr-tools!',
  created_at: Math.floor(Date.now() / 1000),
}, secretKey)
await relay.publish(event)
```

Test script: `scripts/test-proxy-nostr-tools.mjs`.

### Connecting with nostr-sdk v0.44 (Python)

```python
import nostr_sdk

keys = nostr_sdk.Keys.parse("<hex-privkey>")
signer = nostr_sdk.NostrSigner.keys(keys)
client = nostr_sdk.ClientBuilder().signer(signer).build()
client.automatic_authentication(True)

await client.add_relay(nostr_sdk.RelayUrl.parse("ws://localhost:4869"))
await client.connect()

builder = nostr_sdk.EventBuilder.channel_msg(channel_eid, relay_url, "Hello from Python!")
await client.send_event_builder(builder)
```

Test script: `scripts/test-proxy-nostr-sdk-python.py`.

### Tested Clients (Proxy)

| Client | Platform | Evidence | Notes |
|--------|----------|:--------:|-------|
| **nak** | CLI | Manual (anecdotal) | Auth, discovery, metadata, send, receive, streaming |
| **nostr-tools v2.23** | JS | Standalone script | `scripts/test-proxy-nostr-tools.mjs` |
| **nostr-sdk v0.44** | Python | Standalone script | `scripts/test-proxy-nostr-sdk-python.py` |

**Not verified in-repo** (anecdotal / expected based on NIP-28 + NIP-42 support):
- **Coracle** (Web) — expected best GUI; renders kind:42 in chat UI
- **Amethyst** (Android) — NIP-28 public chat view
- **Nostrudel** (Web) — good NIP-28 support

### Clients That Won't Work (anecdotal)

| Client | Why |
|--------|-----|
| **Damus** | NIP-42 works but no NIP-28 channel UI (anecdotal) |
| **Primal** | Caching relay infrastructure — doesn't connect directly (anecdotal) |
| **Clients without NIP-42** | Both relay and proxy require authentication |

---

## Architecture

```
                          NIP-29 (direct)
┌──────────────────┐ ◄──────────────────────────► ┌──────────────────┐
│  NIP-29 Client   │   kind:9, kind:7, kind:5     │  Buzz Relay    │
│  (Chachi, 0xchat,│   kind:9000/01/02/05/07/08   │  :3000           │
│   nak)           │   #h(uuid), NIP-42            │                  │
└──────────────────┘                               │  kind:39000/1/2  │
                                                   │  kind:44100/44101│
                                                   │  Blossom media   │
┌──────────────────┐        ┌────────────────┐     │  /media/upload   │
│  NIP-28 Client   │◄──────►│  buzz-proxy    │◄───►│                  │
│  (Coracle, nak,  │ NIP-28 │  :4869         │ WS  └──────────────────┘
│   nostr-tools)   │        │                │ +REST
└──────────────────┘        │ kind:42↔kind:9 │ (/api/channels,
                            │ kind:41↔40003  │  /api/events)
                            │ kind:1→kind:9  │
                            │ kind:7 (bidir) │
                            │ kind:5 (out)   │
                            │ #e(id)↔#h(uuid)│
                            │ shadow keys    │
                            └────────────────┘
```

**Direct path:** Clients speak kind:9 natively. No translation, no shadow keys, no proxy. The relay
handles NIP-42 auth, channel scoping via `#h` tags, group discovery (kind:39000–39002), membership
notifications (kind:44100/44101), NIP-29 admin commands (kind:9000, 9001, 9002, 9005, 9007, 9008,
9021, 9022; plus deferred 9009), and standard deletions/reactions (kind:5/7).

**Proxy path:** Translates kind:42 ↔ kind:9 (also accepts kind:1 inbound), kind:41 ↔ kind:40003
(edits), kind:7 (reactions, bidirectional), and kind:5 (deletions, outbound only — standard kind:5
events only; admin/REST deletions do not surface as NIP-28 delete events). Re-signs events with
deterministic shadow keys (HMAC-SHA256 of salt + pubkey). Channel discovery (kind:40) is synthesized
locally from Buzz's REST API at startup and never forwarded upstream. Channel metadata (kind:41)
is dual-sourced: local snapshot metadata plus upstream edit events (kind:40003 → kind:41).

---

## Proxy Environment Variables

| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `BUZZ_UPSTREAM_URL` | ✅ | — | WebSocket URL of the relay |
| `BUZZ_PROXY_API_TOKEN` | ✅ | — | Relay API token for REST calls (required until proxy is migrated to NIP-98 auth) |
| `BUZZ_PROXY_SERVER_KEY` | ✅ | — | Hex-encoded 32-byte secret key (raw hex, not bech32 `nsec`) |
| `BUZZ_PROXY_SALT` | ✅ | — | Hex 32-byte salt for shadow keys (keep stable and secret) |
| `BUZZ_RELAY_PUBKEY` | ✅ | — | Hex-encoded 64-char relay public key (for attribution trust) |
| `BUZZ_PROXY_BIND_ADDR` | ❌ | `0.0.0.0:4869` | Listen address |
| `BUZZ_PROXY_RELAY_URL` | ❌ | derived from bind addr | Public WebSocket URL for NIP-42 relay-tag validation. Set if behind a reverse proxy. |
| `BUZZ_PROXY_ADMIN_SECRET` | ❌ | — | Bearer secret for `/admin/*` (unset = no auth, dev mode) |
| `RUST_LOG` | ❌ | `buzz_proxy=info,tower_http=info` | Log level |

---

## Relay Membership (NIP-43)

When `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`, every authenticated connection is checked against the
`relay_members` table. Only pubkeys with a row in that table may use the relay. The relay owner
is bootstrapped automatically from `RELAY_OWNER_PUBKEY` on startup.

### CLI: Managing Members

Use `buzz-admin` — the operator CLI shipped in the relay image — to manage relay membership.
In a Docker Compose deployment, use `run.sh`:

```bash
# Add a member (accepts bech32 npub or 64-char hex; default role: member)
./run.sh add-member npub1abc...
./run.sh add-member <64-char-hex-pubkey>
./run.sh add-member npub1abc... --role admin

# Remove a member
./run.sh remove-member npub1abc...
./run.sh remove-member npub1abc... --role member   # only removes if role matches

# List all members
./run.sh list-members
```

Or invoke `buzz-admin` directly inside the container:

```bash
docker compose exec relay buzz-admin add-member --pubkey npub1abc...
docker compose exec relay buzz-admin add-member --pubkey npub1abc... --role admin
docker compose exec relay buzz-admin remove-member --pubkey npub1abc...
docker compose exec relay buzz-admin list-members
```

**Exit codes:**

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Validation error (bad pubkey, bad role, usage error) |
| 2 | Not found (remove: member does not exist) |
| 3 | Cannot remove relay owner (use `RELAY_OWNER_PUBKEY` to change owner) |
| 4 | Role mismatch (`--role` check failed) |
| 5 | DB/Redis/internal error |

**Required environment variables for member management:**

| Variable | Notes |
|----------|-------|
| `DATABASE_URL` | Postgres connection string |
| `REDIS_URL` | Redis connection string |
| `BUZZ_RELAY_PRIVATE_KEY` | Hex private key — required to sign kind:13534 events |

### NIP-43 Admin Events (WebSocket)

Relay membership can also be managed over WebSocket using NIP-43 admin events. These require
the sender to be authenticated (NIP-42) as the relay owner or an admin.

| Kind | Action | Required tags |
|------|--------|---------------|
| 9030 | Add member | `["p", "<hex-pubkey>"]`, optional `["role", "member\|admin"]` |
| 9031 | Remove member | `["p", "<hex-pubkey>"]`, optional `["role", "member\|admin"]` |
| 9032 | Change role | `["p", "<hex-pubkey>"]`, `["role", "member\|admin"]` |

Example using `nak`:

```bash
# Add a member (owner or admin must sign)
nak event -k 9030 \
  --tag "p=<target-hex-pubkey>" \
  --tag "role=member" \
  --auth --sec <owner-or-admin-privkey> \
  ws://localhost:3000

# Remove a member
nak event -k 9031 \
  --tag "p=<target-hex-pubkey>" \
  --auth --sec <owner-or-admin-privkey> \
  ws://localhost:3000

# Change a member's role to admin
nak event -k 9032 \
  --tag "p=<target-hex-pubkey>" \
  --tag "role=admin" \
  --auth --sec <owner-or-admin-privkey> \
  ws://localhost:3000
```

After each add/remove/role-change, the relay publishes a kind:13534 membership list event
(relay-signed, NIP-70 protected) that clients can subscribe to:

```bash
# Subscribe to the live membership roster
nak req -k 13534 --auth --sec <privkey> ws://localhost:3000
```

### Known Limitations

1. **CLI intentionally does not emit kind 8000/8001 deltas** — `publish_nip43_delta` is
   in-process-only (no Redis hop), so a sidecar call stores but never pushes. The 13534 list
   snapshot is the authoritative roster and rides Redis to live clients. Do not wire a delta call
   that passes in-process tests and silently no-ops in the deployed `compose exec` path.

2. **The `custom_created_at = max(now, newest_existing_13534 + 1s)` bump defeats same-second
   domination for serial invocations; it does NOT serialize concurrent CLI processes** — two
   near-simultaneous adds can read the same newest timestamp and collide on the bumped second.
   `run.sh` serialization is the guard against parallel adds (e.g. `xargs -P`). When adding
   multiple members in a loop, add `sleep 1` between invocations.

---

## Relay Environment Variables (NIP-29 relevant)

| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `BUZZ_PUBKEY_ALLOWLIST` | ❌ | `false` | Enable pubkey allowlist for NIP-42 pubkey-only auth |
| `BUZZ_RELAY_PRIVATE_KEY` | ❌ | random | Hex secret key for relay signing (discovery events, system messages) |
| `BUZZ_REQUIRE_AUTH_TOKEN` | ❌ | `false` | Require authenticated NIP-42 for all connections |

---

## Security Notes

### Direct Path
- **Pubkey allowlist is fail-closed.** DB errors deny the connection.
- **API token users bypass the allowlist.** The allowlist only gates pubkey-only NIP-42.
- **kind:9 requires `#h` tag.** Messages without a channel-scoped `#h` tag are rejected.
- **kind:7 derives channel from target.** Reactions look up the target event's channel via `#e` — client-supplied `#h` tags are ignored. Reactions to unknown events are rejected (fail-closed).
- **kind:5 uses `#h` if present, but doesn't require it.** Deletions validate author-match against target events via `#e` tags. Only self-authored events can be deleted (admin deletions use kind:9005).
- **Client-submitted kind:44100/44101 rejected.** Membership notifications can only be signed by the relay keypair.

### Proxy Path
- **Event pubkey verification.** Inbound events must have a `pubkey` matching the authenticated NIP-42 identity. Spoofed pubkeys are rejected.
- **Inbound kind:5 blocked by proxy policy.** Not yet implemented. The relay's deletion handler does perform author-match validation, but the proxy-side translation path for inbound deletions has not been built.
- **Shadow keys use HMAC-SHA256.** Proper domain separation; salt must be kept secret.
- **Guest registry is in-memory.** Lost on proxy restart. Re-register guests after restarts.
- **Invite tokens are in-memory.** Lost on proxy restart. Default `max_uses` is 10.
- **Revocation is not session-aware.** Removing a guest doesn't disconnect active sessions.
- **Admin secret uses hash-then-compare.** No timing oracle on the bearer token check.

---

## Troubleshooting

### Direct Path

| Symptom | Cause | Fix |
|---------|-------|-----|
| `auth-required: verification failed` | Pubkey not in allowlist (when enabled), or NIP-42 auth failed | Add pubkey to `pubkey_allowlist` table; verify NIP-42 challenge/response |
| `invalid: channel-scoped events must include an h tag` | kind:9 sent without `#h` tag | Include `--tag "h=<channel-uuid>"` |
| `invalid: reaction target event not found` | Reaction references unknown event | Ensure the target event exists in the relay |
| No discovery events | Channel is private + you're not a member | Join the channel first via REST API |

### Proxy Path

| Symptom | Cause | Fix |
|---------|-------|-----|
| `restricted: pubkey not registered and no invite token provided` | Pubkey not registered, no token | Register guest or create invite token |
| `error: token invalid: invite token not found` | Token doesn't exist (proxy restarted or mistyped) | Create new invite token |
| `error: token invalid: invite token expired` | Token past expiry time | Create new invite token |
| `error: token invalid: invite token exhausted` | Token reached `max_uses` limit | Create new invite token with higher limit |
| `auth-required: authentication timeout` | Client didn't respond to NIP-42 within 30s | Use a NIP-42-capable client |
| No messages after auth | Unresolved `#e` filter silently returns zero events | Re-query `nak req -k 40` for correct kind:40 event ID |
| Guest still has access after revoke | Active sessions not terminated | Restart proxy to cut all sessions |
| Proxy startup fails | Can't reach relay REST API or missing env vars | Check relay is running; verify all required env vars (especially `BUZZ_RELAY_PUBKEY`) |

---

## Further Reading

- [`crates/buzz-proxy/README.md`](crates/buzz-proxy/README.md) — proxy crate internals, shadow key derivation, subscription namespacing. **Note:** some auth/buffering details in that README may be stale; this document is the authoritative reference for proxy behavior.
