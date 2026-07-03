# Using Third-Party Nostr Clients with Buzz

Buzz is a Nostr relay that speaks NIP-29 (relay-based groups) natively. Third-party Nostr clients connect directly to `buzz-relay` using NIP-29 and NIP-42 authentication. The old NIP-28 compatibility proxy has been removed.

## Community scope

Buzz treats the relay URL/domain as authoritative for the community. Today's
single-relay deployment has exactly one community behind that URL, so existing
NIP-29 clients keep using the same WebSocket URL, event kinds, tags, and
HTTP/media/git paths. In a multi-community deployment, each community is reached
by its own domain or subdomain; the backend resolves the community from the host
before handling AUTH, EVENT, REQ, REST, media, git, search, or workflow traffic.

The Nostr wire format does not grow a tenant tag. Client-supplied `#h` tags still
name channels/groups and are checked against the host-derived community. Events
without `#h` — profiles, gift-wrapped DMs, membership notifications, lists,
status, long-form notes, workflow/system events, and other "global" streams — are
global only inside the connected community. A pubkey can join multiple
communities and repost its profile in each one; DMs and profiles do not inherit
across community domains.

---

## NIP-29 Direct

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
| **Membership notifications** | ✅ | kind:44100 (added) / kind:44101 (removed); relay-signed, community-global scope (`channel_id=None` inside the connected community) |
| **Presence (kind:20001)** | ✅ | Ephemeral; arbitrary status string (truncated to 128 chars); writes to Redis (`set_presence`/`clear_presence` on `"offline"`), then fan-out to local subscribers. In multi-community mode presence is scoped to the connected community. |
| **Typing indicators (kind:20002)** | ✅ | Ephemeral, not stored; published via Redis pub/sub (multi-node capable unlike presence fan-out) |
| **NIP-42 authentication** | ✅ | Proactive challenge; optional pubkey allowlist |
| **NIP-11 relay info** | ✅ | `GET /` with `Accept: application/nostr+json` |
| **Blossom media** | ✅ | `PUT /media/upload` (BUD-02), `GET /media/{sha256}.{ext}` (BUD-01) |
| **NIP-50 search** | ✅ | One-shot search REQs: `{"search":"query","kinds":[9],"#h":["<uuid>"]}` → relevance-sorted results → EOSE. Not registered as persistent subscriptions. |
| **NIP-10 threads** | ✅ | WS-submitted replies with `["e","<root>","","reply"]` tags create `thread_metadata` atomically. Visible in REST thread queries. Unknown parents rejected. |
| **NIP-17 DMs (gift wrap)** | ✅ | kind:1059 accepted with ephemeral signing keys. Stored community-globally (`channel_id=None` inside the connected community). Delivered via `#p`-filtered subscriptions. Not indexed in search. |
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
| **44100** | Member added | `p` = target pubkey, `h` = channel UUID | Community-global |
| **44101** | Member removed | `p` = target pubkey, `h` = channel UUID | Community-global |

Stored community-globally (`channel_id = None` inside the connected community) so agents and clients can subscribe without knowing channel
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

## Relay Membership (NIP-43)

When `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`, every authenticated connection is checked against the
`relay_members` table. In today's single-community deployment this is the relay-wide member list; in multi-community mode the same rule is scoped to the host-derived community. Only pubkeys with a row for that community may use that community. The relay owner
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
| 9033 | Set workspace profile (icon) | `["icon", "<https-url or data:image/* URL>"]` (empty clears) |

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

A kind:9033 command similarly makes the relay store the workspace icon (per
community) and serve it in the standard NIP-11 `icon` field of its relay
information document. Clients render it in the workspace rail/switcher; anyone
can read it (`curl -H 'Accept: application/nostr+json' http://localhost:3000`),
but only admins/owners can set it. Full spec:
[docs/nips/NIP-WP.md](docs/nips/NIP-WP.md).

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

---

## Troubleshooting

### Direct Path

| Symptom | Cause | Fix |
|---------|-------|-----|
| `auth-required: verification failed` | Pubkey not in allowlist (when enabled), or NIP-42 auth failed | Add pubkey to `pubkey_allowlist` table; verify NIP-42 challenge/response |
| `invalid: channel-scoped events must include an h tag` | kind:9 sent without `#h` tag | Include `--tag "h=<channel-uuid>"` |
| `invalid: reaction target event not found` | Reaction references unknown event | Ensure the target event exists in the relay |
| No discovery events | Channel is private + you're not a member | Join the channel first |

---

## Further Reading
