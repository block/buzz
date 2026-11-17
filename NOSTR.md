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

# 4. Connect any NIP-29 + NIP-42 client to ws://localhost:3000
```

### What Works

| Feature | Status | Notes |
|---------|:------:|-------|
| **Group chat (kind:9)** | ✅ | Send/receive messages with `#h <channel-uuid>` tag |
| **Reactions (kind:7)** | ✅ | Standard NIP-25; channel derived from target event's `#e` tag (client `#h` ignored) |
| **Deletions (kind:5)** | ✅ | Standard NIP-09; self-authored only. `#h` optional, `#e` required |
| **User profiles (kind:0)** | ✅ | NIP-01 metadata; synced to users table (display_name, avatar, about, NIP-05). NIP-05 handles must canonicalize to this relay's domain — off-domain or invalid handles are silently cleared. If a NIP-05 handle collides with another user's (UNIQUE constraint), the handle is skipped but other profile fields (display_name, avatar, about) are still synced. |
| **Group creation (kind:9007)** | ✅ | NIP-29; include an `h` tag with the new channel id and a `name` tag, optional `visibility`, `channel_type`, label `t` and `unlisted` tags. The relay refuses a 9007 without `h`. It refuses a listed channel (open, not unlisted, stream or forum) whose name another listed channel already has, ignoring case and including archived ones: `invalid: an open channel named "<name>" already exists`. |
| **Add user (kind:9000)** | ✅ | Open: any user, subject to target's `channel_add_policy` (`owner_only`/`nobody` can block). Private: owner/admin only. Self-add bypasses agent policy but not private-channel auth. A guest cannot add anyone. A new member with no role tag gets the channel's `join_role`; a non-member adding themselves may ask for `join_role` or a lower role. One event may name up to 100 people: one `p` tag each, with an optional role as the `p` tag's third element (NIP-29), falling back to the event's `role` tag. Each person is checked and added on their own. When the event names several people, the OK message is `members: ` followed by JSON listing the non-empty `added`, `updated` (role changed), `already_member` and `rejected` (`[{"reason", "pubkeys"}]`) results. The event is refused only when it is malformed or nobody it names may be added; then one person keeps the single-add error, and several get `invalid: members: {…}`. The event and its membership changes are stored together, so an `error:` rejection stores nothing and the same event can be resent; a resent stored event returns `duplicate:` without results. Re-adding someone at their current role posts no second `member_joined` notice. |
| **Remove user (kind:9001)** | ✅ | Self-remove allowed (with last-owner guard). Removing others: owner/admin, or an active member removing their own agent. Only a channel owner may remove another channel owner. |
| **Edit group metadata (kind:9002)** | ✅ | `name`/`about`/label `t`/`visibility`/`ttl`/`archived`/`join_role`/`unlisted` tags: owner/admin. `topic`/`purpose` tags: any member except a guest. `auto_join` tag: relay owner/admin (see [Default channels](#default-channels)). `parent` and `parent_access` tags: without other metadata tags, each with its own rule (see [Channel parents](#channel-parents)). A relay operator may also send a 9002 that changes only `name` on a listed channel, archived ones included, whatever its channel role, so that duplicate names can be repaired; the channel stays archived. The relay refuses a `name`, `visibility` or `unlisted` change that leaves a listed channel with a name another listed channel already has: `invalid: an open channel named "<name>" already exists`. The relay refuses a private-to-open `visibility` change while the channel has, or once had, a member who is neither the sender nor an agent the sender owns (NIP-OA); removed members count, because their posts stay in its history: `invalid: a private channel with other members cannot be made open; create a new open channel instead`. Buzz applies 9002 as a partial update: an absent tag leaves that setting unchanged. |
| **Admin delete event (kind:9005)** | ✅ | Event author can always delete own. Otherwise owner/admin required. Target must be in same channel. |
| **Group deletion (kind:9008)** | ✅ | Owner only. |
| **Leave group (kind:9022)** | ✅ | Any member. Last-owner guard prevents orphaned groups. |
| **Group metadata (kind:39000)** | ✅ | Relay-signed; always `d`, `name`, `t`, `P`, `created_at`, `join_role`; `public` or `private`; `restricted` and `closed` per the access flags below; `unlisted` when the channel is unlisted; `about` only if description non-empty; `hidden` for DM channels and private system channels; `auto_join` while the channel is a default channel; `parent` while the channel has a parent; `parent_access` when it is not `member` |
| **Unlisted channels** | ✅ | kind:9002 `["unlisted", "true"\|"false"]`, owner/admin, or at creation on kind:9007. An unlisted open channel is left out of every read that does not name it, for non-members: channel lists, kind:39000 without `#d`, messages, COUNT, NIP-50 search and artifact queries. Naming it by `#h` (or `#d` on 39000–39003) reads it, and kind:9021 joins it, as for any open channel. See [Unlisted channels](#unlisted-channels). |
| **Group admins (kind:39001)** | ✅ | Relay-signed; `d`, channel type `t` and creator `P` tags + `p` tags with roles (`owner`, `admin`) |
| **Group members (kind:39002)** | ✅ | Relay-signed; `d`, channel type `t` and creator `P` tags + `p` tags for all members |
| **Membership notifications** | ✅ | kind:44100 (added) / kind:44101 (removed); relay-signed, community-global scope (`channel_id=None` inside the connected community) |
| **Presence (kind:20001)** | ✅ | Ephemeral; arbitrary status string (truncated to 128 chars); writes to Redis (`set_presence`/`clear_presence` on `"offline"`), then fan-out to local subscribers. In multi-community mode presence is scoped to the connected community. |
| **Agent presence (kind:20003)** | ✅ | Ephemeral; an owner's presence for the agents it owns, named by `p` tags (at most 256); `online`/`away` set and `offline` clears each agent's presence as its own kind:20001 would. Not fanned out. |
| **Channel access changed (kind:20004)** | ✅ | Ephemeral, relay-signed and relay-only: tells a channel's live subscribers to ask again what they may do there (see [Channel access](#channel-access)). A client may not send it. |
| **Viewer access changed (kind:20005)** | ✅ | Ephemeral, relay-signed, relay-only and `#p`-gated: tells one pubkey, when its community timeout or ban changes, to ask again what it may do in every channel (see [Channel access](#channel-access)). A client may not send it. |
| **Typing indicators (kind:20002)** | ✅ | Ephemeral, not stored; published via Redis pub/sub (multi-node capable unlike presence fan-out) |
| **NIP-42 authentication** | ✅ | Proactive challenge; optional pubkey allowlist |
| **NIP-11 relay info** | ✅ | `GET /` with `Accept: application/nostr+json` |
| **Blossom media** | ✅ | `PUT /media/upload` (BUD-02), `GET /media/{sha256}.{ext}` (BUD-01) |
| **NIP-50 search** | ✅ | One-shot search REQs: `{"search":"query","kinds":[9],"#h":["<uuid>"]}` → relevance-sorted results → EOSE. Not registered as persistent subscriptions. HTTP `POST /query` also accepts `search_sort: "newest"` (advertised as `buzz-search-sort` in NIP-11 `supported_extensions`) for newest-first results, paged with `page`. |
| **NIP-10 threads** | ✅ | WS-submitted replies with `["e","<root>","","reply"]` tags create `thread_metadata` atomically. Visible in REST thread queries. Unknown parents rejected. |
| **NIP-17 DMs (gift wrap)** | ✅ | kind:1059 accepted with ephemeral signing keys. Stored community-globally (`channel_id=None` inside the connected community). Delivered via `#p`-filtered subscriptions. Not indexed in search. |
| **DM discovery** | ✅ | DM creation emits kind:39000 (with `hidden` tag) + kind:44100 membership notifications. NIP-29 clients discover DMs via standard group discovery flow. |
| **Join request (kind:9021)** | ✅ | Open channels only. Adds the joiner with the channel's `join_role` (`member` by default), emits system message + group discovery events + kind:44100 membership notification. Private channels rejected at ingest. The same body joins new community members to [default channels](#default-channels). |
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

### Announce channels

Roles have the same permissions in every channel. A channel's settings
decide only who can join and which role a new person gets:

- `visibility` (`open` | `private`) decides who can find, join and read.
- `join_role` (`member` | `guest`) is the role a person gets when nobody
  picks one: a self-join (kind:9021, or a kind:9000 self-add), or a kind:9000
  add with no `role` tag. The default, `member`, keeps today's behavior.

An open channel with `join_role = guest` is an **announce channel**: anyone
can find, read and join it, but people who join are guests and cannot write.

What each role can do:

| Action | Guest | Member or bot | Admin / owner |
|--------|-------|---------------|---------------|
| Read, leave | Yes | Yes | Yes |
| Post, reply, edit, delete own, react, artifacts, canvas, forum, pins, typing, workflow messages, huddles | No | Yes | Yes |
| Topic, purpose (kind:9002) | No | Yes | Yes |
| Add people (kind:9000) | No | Yes, as member, guest or bot | Yes; only owners can grant owner |
| Push to the channel's git repositories | No | Yes | Yes |
| Name, about, visibility, TTL, `join_role`, `unlisted` | No | No | Yes |

A non-member may write only where joining would give a role that can write:
an open channel with `join_role = member`. A refused write gets
`restricted: only members can post in this channel`.

Other rules:

- A non-member adding themselves with kind:9000 may ask for `join_role` or a
  lower role. A higher role is refused.
- Any member who can write (member, bot, owner or admin) can make a guest a
  member with kind:9000 (`role` `member`). This is the same for a person and
  an agent. Owners and admins can make other role changes, except that only
  owners may grant ownership or change an owner's role. Admins cannot promote
  themselves or anyone else to owner. Only owners may remove other owners;
  owning an agent does not override its channel-owner role. The last-owner
  guard still applies to owner demotion and departure.
- The human owner of an agent can edit (kind:40003) and delete (kind:9005)
  that agent's messages, also as a guest or a non-member. This is existing
  behavior. It lets a person correct or remove what their agent posted; it
  does not let them post.
- Owners and admins change the setting with kind:9002
  `["join_role", "guest"]` or `["join_role", "member"]`. A missing tag means
  "no change". Changing it does not change anyone's role: existing members
  can still write after a change to `guest`, and existing guests stay
  read-only after a change to `member`. A stored value the relay does not
  know is treated as `guest`.

kind:39000 publishes the NIP-29 access flags from these settings, and an
explicit `["join_role", ...]` tag so clients can tell what joining gives:

| `visibility` | `join_role` | `private` | `public` | `restricted` | `closed` |
|--------------|-------------|-----------|----------|--------------|----------|
| open | `member` | | ✓ | | |
| open | `guest` | | ✓ | ✓ | |
| private | either | ✓ | | ✓ | ✓ |

### Unlisted channels

An **unlisted** open channel works like a link-shared document: anyone in
the community who has its id (for example from a shared link) can read it
and join it, but it does not show up for people who are not members.

- Owners and admins set it with kind:9002 `["unlisted", "true"]` and clear
  it with `["unlisted", "false"]`. A missing tag means "no change". It can be
  sent in the same event as `["visibility", "open"]`, so a private channel
  becomes open and unlisted in one step.
- kind:9007 also takes `["unlisted", "true"|"false"]`, so a channel can
  start unlisted; a missing tag means listed. Any other value is refused
  before the event is stored, as on kind:9002.
- For a non-member, an unlisted open channel, of any type, is left out of
  every read that does not name it: a filter without `#h` (including
  kind:39000 discovery without `#d`, and the system-channel opt-in by `#P`
  or `#t:["system"]`), COUNT, NIP-50 search, artifact queries and the
  channel list. A filter that names it by `#h`, or by `#d`
  on a filter whose kinds are all 39000–39003, reads it under the ordinary
  open-channel rules. This is the same "named-only" rule that applies to
  system channels ([NIP-CL](docs/nips/NIP-CL.md)).
- Joining (kind:9021) and posting work as on any open channel; `join_role`
  still decides what a joiner gets.
- Members see it everywhere, as before.
- The flag is stored on its own and matters only while the channel is open.
  Private channels are already invisible to non-members. A private channel
  can be marked unlisted ahead of time, so opening it later never lists it.
- kind:39000 carries a bare `["unlisted"]` flag whenever the setting is on,
  whatever the visibility. It does not add NIP-29 `hidden`: the metadata
  stays readable by id.
- Relays that support it advertise `buzz-unlisted-channels` in NIP-11
  `supported_extensions`. An older relay applies `["visibility", "open"]`
  and ignores `unlisted`, so a client must check for it before opening a
  channel as unlisted.
- Unlisted is not an access control. Anyone with the id can read the channel.

### Channel parents

A stream or forum can sit under one **parent** channel, as a project's
channels sit under the project. Anyone who can read the parent can read
the child, without joining it.

- Set it with kind:9002 `["parent", "<channel id>"]` and clear it with
  `["parent", ""]`. It may ride only with `parent_access`: a 9002 with
  `parent` and any other metadata tag is refused. Setting takes a direct owner or admin role in the
  child and a role that may post in the new parent. Clearing takes a direct
  owner or admin role in the child or in its current parent.
- Only streams and forums take part. A channel with a parent may have a TTL
  and auto-archive; it still reads and grants as before once archived. A
  project, a channel with children and no parent, may not: the relay refuses
  a TTL on it, filing a channel under a channel that has a TTL and no parent,
  and clearing the parent of a channel that has a TTL and children. A tree is at most three channels
  deep (channel, parent, grandparent) and has no loops; the relay refuses a
  change that breaks either. A new parent must not be archived.
- **Reading.** A private child is readable by its members and by anyone who
  can read its parent or grandparent: a member of it, or anyone when it is
  open. Open children are readable by everyone anyway. Every read path
  follows this: named and implicit REQs, live delivery, COUNT, NIP-50
  search, artifact queries, kind:39000–39003 and huddles. Children of a
  listed open parent are in the implicit scope; children of an unlisted open
  parent are readable only by name, as the parent is.
- **Writing.** The closest grant wins. Walking channel, parent, grandparent,
  the first level where the person has a member row, or that is open, gives
  the role: the row's role, or the open channel's `join_role`. A role that
  comes from a parent caps at `member`; owner and admin powers come only from
  a row in the channel itself. So under an announce parent, a non-member of
  the parent reads the children as a guest.
- **What a channel takes from its parents.** kind:9002
  `["parent_access", "member"|"guest"|"none"]` caps what the walk above may
  bring down to the channel. `member`, the default, is the cap above.
  `guest` makes everyone who comes through a parent a guest: they read but
  cannot post. `none` stops the walk: only the channel's own member rows
  (and, while it is open, its own `join_role`) grant anything, so the
  parent's people do not read it, list it, count it, search it, sync its
  artifacts or see its metadata. Each step up passes the cap of the channel
  it leaves, so a `none` channel also cuts its children off from the
  channels above it, and a `guest` one caps them at guest. A direct row
  still decides, including for someone outside the parent. Only a direct
  owner or admin of the channel may change it; it may ride with `parent`,
  so a channel can be filed and narrowed in one event, but with no other
  metadata tag, and at most once. It matters only while the channel has a
  parent and is private.
- Access through a parent never makes a member row. Membership lists,
  kind:39002 and agent subscriptions are unchanged; an agent listens only
  where it is a member.
- An archived parent still grants access; a deleted one grants nothing.
- Removal from a parent, a parent or `parent_access` change and a visibility
  change take effect at once, and live subscriptions of people who lost access close
  with `restricted: channel access revoked`.
- kind:39000 carries `["parent", "<channel id>"]` while the channel has one,
  and `["parent_access", <value>]` while it is not `member`.
- Relays that support it advertise `buzz-channel-parents` in NIP-11
  `supported_extensions`, and `buzz-channel-parent-access` for
  `parent_access`. A relay with the first but not the second ignores a
  `parent_access` tag beside `parent` and files the channel unnarrowed, so a
  client checks for it before filing a narrowed channel.

### Artifact conversations

A NIP-AR artifact, such as a task, can have a **conversation**: a channel
for its comments. It belongs to the artifact, not to anyone who posts in it.

- **Address.** The conversation's channel ID is derived from the artifact's
  `d` alone, so every client computes the same one: take the first 16 bytes
  of `SHA-256("nip-ar-conversation" || 0x00 || UTF-8(d))`, then set the
  RFC 9562 version (8) and variant bits (byte 6 = `(b & 0x0f) | 0x80`,
  byte 8 = `(b & 0x3f) | 0x80`). It does not change when the artifact moves.
  Fixture: `d` `8c1f3d2e-4b5a-4c6d-9e7f-0a1b2c3d4e5f` gives
  `3f3e3cd7-3b56-81d9-8dd2-8d8b5b1f6d57`. Version-8 channel IDs are
  reserved for conversations: an ordinary kind:9007 at one is refused.
- **Opening.** kind:9007 with `["h", <derived id>]` and
  `["artifact", <d>]`. Anyone who may post in the artifact's home may send
  it; it needs the `messages:write` scope. The relay checks that `h` is the
  derived address and that the artifact exists, is not deleted and lives in
  that home. It then creates a private channel of type `artifact`, filed
  under the home (`parent`), with no member rows: nobody owns it. Other
  tags are ignored. Opening one that already exists is accepted and writes
  nothing. The home must be a live, unarchived stream or forum with at most
  one parent, so the conversation is at most the third level; otherwise the
  relay refuses with `invalid: comments aren't available here`. An
  artifact cannot itself live in a conversation.
- **Comments** are kind:9 events with the conversation's `h`. Who reads and
  posts comes only from the home, through [Channel parents](#channel-parents):
  the home's readers read, a view-only (`guest`) home's people read but do
  not post, and a home's `parent_access` applies as it does to any child.
  Comments close to new posts while the home is archived. Only the home
  counts: an archived project above it still grants access and does not
  archive its channels, so it does not close their comments either, just as
  it does not close posting in those channels.
- **Only comments.** A conversation accepts only kind:9 messages, kind:40003
  edits, kind:7 reactions, kind:5 and kind:9005 deletions, and the opening
  kind:9007. Every other kind with its `h`, including people (9000, 9001,
  9021, 9022), settings (9002), deletion (9008), a canvas, and any kind added
  later, is refused with `invalid: an artifact's conversation only holds
  comments`. The home's direct owners and admins may delete comments with
  kind:9005; authors may delete their own.
- **Moves.** When the artifact moves, the conversation moves with it in the
  same transaction: its parent becomes the new home, the old home's readers
  lose it at once, and their live subscriptions close. A move to a home that
  could not hold a conversation is refused with
  `invalid: this artifact's comments can't move there`.
- **Delete and restore.** Deleting the artifact closes its conversation to
  everyone: nothing in it is readable and nothing can be posted, and opening
  one is refused. Restoring the artifact reopens it.
- **Hidden.** `artifact` is not a legacy channel type, so conversations are
  left out of reads that name no channel ([NIP-CL](docs/nips/NIP-CL.md)).
  Read them by naming the channel.
- Relays that support conversations advertise `buzz-artifact-conversations`
  in NIP-11 `supported_extensions`. They also include artifacts in
  auto-archiving channels in incremental artifact sync.

### Channel access

A client asks what its signer may do in named channels, and is told when
to ask again. Conversations, channels read through parents and open
channels the signer has not joined all get the same answer.

- **Asking.** `POST /query` with exactly
  `[{"access":"channels","ids":[<channel id>, ...]}]`: 1 to 100 distinct
  lowercase channel IDs. The NIP-98 proof must carry a `payload` tag. Any
  other shape is refused with 400.
- **Listing.** Without `ids`, `[{"access":"channels"}]` lists the
  channels the signer can reach, ordered by channel ID: the channels they
  joined, listed open channels, and the children and grandchildren of
  both that the parents' grants reach. These are the channels a read that
  names no channel covers, so conversations and unlisted channels stay
  out. It is paged: optional `"limit"` (1 to 100, default 100) and
  `"after":<channel id>`, and no other keys. The answer is
  `{"channels":[<entry>, ...],"next":<channel id or null>}`. A page looks
  at up to `limit` candidates after `after` and leaves out the ones the
  signer cannot read, so it can hold fewer than `limit` and still have a
  `next`. To read the next page, send `next` as `after`; `null` means
  there are no more. Walking every page lists each channel once.
- **Answer.** For `ids`, a JSON array with one entry per readable channel,
  in request order:
  `{"id":<channel id>,"read":true,"post":<bool>,"joined":<bool>,"role":<role>,"archived":<bool>}`.
  `joined` is whether the signer has a member row. `role` is the one they
  act with: their row's (`owner`, `admin`, `member` or `guest`; a bot acts
  as `member`), else an open channel's `join_role`, else what the parents
  give, capped at `member`. `archived` is whether the channel itself is
  archived. `post` is whether a kind:9 from them would be accepted now:
  false for a `guest` role, an archived channel, a conversation whose home
  is archived, and a community timeout of the signer or of the agent's
  owner, by the same check the post itself meets. While the signer is
  timed out, every entry also carries `"timed_out_until":<unix seconds>`;
  ask again then. A banned signer, or an agent whose owner is banned, is
  refused with 403 and the reason, as every request from them is. A channel left out cannot be read:
  it is private to them, deleted or missing. If any channel cannot be
  answered, the whole request fails with 503 and is retried; a channel is
  only ever left out of a complete answer. The answer is a snapshot, and
  every post is still checked when it arrives.
- **Who else can read.** `POST /query` with exactly
  `[{"access":"readers","channel":<channel id>,"pubkeys":[<hex pubkey>, ...]}]`:
  one lowercase channel ID and 1 to 100 distinct lowercase hex pubkeys,
  with no other keys. The NIP-98 proof must carry a `payload` tag. Any
  other shape is refused with 400. The answer is a JSON array with one
  entry per pubkey, in request order: `{"pubkey":<hex pubkey>,"read":<bool>}`.
  Nobody named is left out, so `false` always means "cannot read".
  Reading is judged by the check a task's assignees meet
  ([NIP-AR](docs/nips/NIP-AR.md)): a member row, an open channel, or what
  the parents give under `parent_access`; a conversation follows its home.
  Someone banned from the community, or an agent whose owner is, reads
  nothing and is answered `false`, as every request from them is refused.
  A timeout doesn't change reading: a timed-out person is answered `true`
  where they read, and may ask.
  - **Who may ask.** Only a signer who can read the channel. Otherwise the
    request is refused with 403 and
    `{"error":"not-allowed: the signer cannot read this channel"}`. A
    missing or deleted channel is refused the same way, so the answer
    can't show whether a private channel exists. A banned signer is
    refused before the question is answered, with the ban's own
    `blocked:` reason, like every request from them. There are no partial
    answers: a refusal is never a list of `false`s.
  - **What it discloses, by design.** It answers only for a channel the
    asker can already read, and it says only whether each person reads
    it, never how or why not. The relay never states the reason for
    someone else. Two things can still be inferred:
    - Someone who can read a channel can learn that a named person reads
      it through a parent the asker isn't in, such as a private project.
    - An asker who also sees a person in the channel's member list, but
      is told they can't read it, can infer that the person is banned.
  - If any person cannot be answered, the whole request fails with 503 and
    is retried. The answer is a snapshot: after a kind:20004 for the
    channel, ask again.
  - Relays that support this advertise `buzz-channel-readers` in NIP-11
    `supported_extensions`.
- **Ask again.** After a committed change to who may read or post in a
  channel, the relay sends an ephemeral kind:20004, signed by the relay,
  with empty content and only `["h", <channel id>]`, to that channel and to
  every private channel under it, conversations included. A subscription
  receives it like any event, so include kind 20004 in the channel's
  filter. It goes only to subscribers who can still read the channel;
  those who cannot have their subscriptions closed. It is sent for a
  member added, removed, leaving, joining or changing role; a change to
  visibility, `join_role`, archiving, `parent` or `parent_access`; and an
  artifact's move, delete or restore; a channel's deletion; and a
  huddle's auto-archive or auto-join. It carries no answer: ask again. It
  is best effort, so a client also asks again when it reconnects and when
  a post is refused.
- **Ask again everywhere.** When a pubkey's community restriction changes
  (a timeout or ban starts or is lifted, by any route: a moderation
  command, the admin API or a resolved report), or an agent's owner is
  recorded, the relay sends an ephemeral kind:20005, signed by the relay,
  with empty content and only `["p", <pubkey>]`, to that pubkey and to each
  agent it owns. Subscribe with `{"kinds":[20005],"#p":[<own pubkey>]}`;
  the kind is `#p`-gated, so it reaches no one else. It carries no answer:
  ask again about every channel. A timeout that runs out sends nothing;
  `timed_out_until` says when to ask.
- Relays that support this advertise `buzz-channel-access` in NIP-11
  `supported_extensions`. Without it, a client cannot learn whether it may
  post in a channel it has not joined.

### Default channels

A community's owners and admins can mark a channel as a **default channel**:
one the relay joins every new community member into. The setting is
`auto_join` on the channel.

- Only a *relay* owner or admin (the NIP-43 roster role, not a channel role)
  can change it, with kind:9002 `["auto_join", "true"]` or
  `["auto_join", "false"]`. The actor need not be a member of the channel.
  A missing tag means "no change".
- It is accepted only on a `stream` channel the event leaves open, and `true`
  also needs it unarchived. `false` is refused on a private channel too: the
  actor need not be a member, so neither value may write into a channel they
  cannot read. The relay decides this in the same transaction as the other
  kind:9002 changes, so one event may open a channel and flag it. Making a
  flagged channel private or archiving it is allowed and leaves the stored
  value in place; to take a private channel out of the defaults, clear the
  flag before making it private or once it is open again.
- kind:39000 carries a bare `["auto_join"]` tag only while the setting is
  effective: set, open and unarchived. What the event says is what the relay
  does, so a client never has to combine the flag with the access flags.
- Nothing is a default channel until someone asks for it: new channels and
  existing channels start with `auto_join` off. `buzz channels update
  --channel <uuid> --auto-join true` sets it.

Every change is announced by one kind:40099 `auto_join_changed` system
message in the channel.

When someone becomes a community member (an invite claim or a kind:9030 add)
the relay joins them to every effective default channel as a self-join: the
body of kind:9021, so the channel's `join_role` applies and the channel gets
its `member_joined` kind:40099, kind:39002 and kind:44100. There is no cap on
how many channels that is: whatever carries the kind:39000 tag is joined, and
the work a new member costs grows with the number of channels an owner or
admin flags. On a community that requires relay membership, the relay does
this once, at that moment, and never again on login or reconnect, so a later
kind:9022 leave stands. A kind:9030 add is
made by someone else, so the new member's `channel_add_policy` applies first,
as it would to that actor's kind:9000: if it refuses them, no default channel
is joined. The claim response waits for the joins, and a claimer who
disconnects before it arrives does not cut them short. A channel that fails
to join, or an add the member's policy refuses, is logged and counted
(`buzz_auto_join_failures_total`), not retried: the kind:39000 tag is the
record a client reconciles against. That includes a channel archived or made
private while the join runs: the membership write decides both again under
the channel's locks, so the member is not added and nothing is announced.
`buzz-admin add-member` writes the roster row only and
does not join default channels.

A community that does not require relay membership has no roster insert to
join people at, so there the relay joins a person when it admits their
WebSocket AUTH, after the OK, to every effective default channel they have
never been a member of. "Never a member" is the channel's own membership
history: a kind:9022 leave or a kind:9001 removal keeps the row, so the
person is not joined back, and a new device or another relay pod finds the
row and does nothing. The check runs again under the channel's membership
lock, so two admissions of the same person join them once. A channel flagged
later is joined on each person's next admission. Agents are skipped: an AUTH
with a NIP-OA owner, a NIP-FI agent token, or a stored agent owner joins
nothing. A membership write that fails leaves no row, so the next admission
tries that channel again.

### Group Discovery

The relay emits NIP-29 group state events when channels are created, updated, or membership changes.
[NIP-CL](docs/nips/NIP-CL.md) defines the channel identity tags on these events and how clients
use them to find channels.
All discovery events include a `d` tag set to the channel UUID (NIP-29 addressable event convention):

| Kind | Tags | Content |
|------|------|---------|
| **39000** | `d=<uuid>`, `name`, `t=<channel_type>`, `P=<creator>`, `created_at=<unix seconds>`, `join_role` (always); `public` or `private`; `restricted` (private channels, and `join_role=guest`); `closed` (private channels only); `unlisted` (unlisted channels); `about` (if description non-empty); `hidden` (DM channels and private system channels); `auto_join` (open, unarchived channels with the setting on, see [Default channels](#default-channels)) | Group metadata. The access flags follow the channel settings (see [Announce channels](#announce-channels)). |
| **39001** | `d=<uuid>`, `t=<channel_type>`, `P=<creator>`, `p` tags with role label (`owner`, `admin`) | Admin list |
| **39002** | `d=<uuid>`, `t=<channel_type>`, `P=<creator>`, `p` tags for all members | Member list |

On all three kinds the channel type `t` tag comes first, then one `t` tag
per channel label, then `P`. Clients read the first `t` value as the
channel type. `P` is
the public key that signed the channel's kind:9007. The relay writes it
from the database, and it does not change when ownership moves. Uppercase
`P` follows NIP-22, NIP-34 and NIP-72, where it names the author of the root
object; lowercase `p` already lists members and DM participants on these
kinds.

kind:39000 also carries `["created_at", <unix seconds>]`: the time the
channel was created. The event's own `created_at` is the time of the last
edit, so use the tag to find the older of two channels.

**Channel labels.** A label groups channels across channel types, for
example `["t", "workspace"]`. Set labels with `["t", <label>]` tags on
kind:9007. On kind:9002 the `t` tags replace the whole set, and
`["t", ""]` with no other `t` tag clears it; a kind:9002 without `t` tags
leaves the labels as they are. Only an owner or admin may change labels.
The relay refuses a label set with more than 8 distinct labels, a label
that is not 1-64 characters from `a-z`, `0-9`, `.`, `:` and `-`, or a
label equal to a channel type name (`stream`, `forum`, `dm`, `workflow`,
`system`).

**System channels.** A channel with `channel_type` `system` exists only for
access control, for example to hold config that only an agent and its owner
can read. It is an ordinary channel in every other way, but queries leave it
out unless they ask for it:

- A filter that does not name it, for example a REQ with no `#h`, a COUNT,
  a NIP-50 search, or kind:39002 `#p:[me]`, does not return it. Such a
  filter returns only the legacy channel types (`stream`, `forum`, `dm`,
  `workflow`), so a channel type added later is also left out.
- A filter that names it with `#h`, or with `#d` on kinds 39000–39003, reads
  it under the ordinary rules: the reader is a member, or it is open. A
  channel of a type added later is read the same way.
- A filter on kinds 39000–39003 with `#t:["system"]` or any `#P` also
  returns the system channels the reader can read, then matches as usual.
  It does not return channels of a type added later.
  For example, `{kinds:[39000], #P:[<creator>], #t:[<label>]}` finds one
  creator's labeled channel, system or not. Values in one tag filter
  combine with OR, so `#t:["system", <label>]` does not mean "a system
  channel with this label".

Push notifications use the same rule: a lease filter pushes events from a
system channel only when its `#h` names that channel. Every kind:44100/44101
membership notification carries its channel type as `t`, and a filter without
`#t` sees only the legacy types, so system channel notices reach only a filter
with `#t:["system"]` (see NIP-CL).

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
> open-channel discovery is a future enhancement. A group-state filter that names channels by
> `#d` (`{kinds:[39000], "#d":[<channel-uuid>]}`) does receive later replacements for the named
> channels it may read, after EOSE.

### Membership Notifications

The relay emits relay-signed notifications when members are added or removed:

| Kind | Meaning | Tags | Scope |
|------|---------|------|-------|
| **44100** | Member added | `p` = target pubkey, `h` = channel UUID | Community-global |
| **44101** | Member removed | `p` = target pubkey, `h` = channel UUID | Community-global |

Each notice also has `t` = the channel type (`stream`, `forum`, `dm`,
`workflow`, `system`). A filter with `#t` gets only the types it names. A
filter without `#t` gets only `stream`, `forum`, `dm` and `workflow`, and so
does an untagged notice stored before the tag existed (NIP-CL).

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

### Workflow Owner Attribution

A workflow message is a kind:9 event that the relay signs and marks with
`["buzz:workflow", "true"]`. Its first `p` tag is `["p", <owner>, "", "owner"]`,
which names the workflow's owner for display. This tag is attribution, not a
mention, so `#p` filters do not match it. This is a deliberate exception to
NIP-01, applied to live subscriptions, history and push leases alike, so a
bounded `#p:[me]` read returns the messages that really mention you. A
workflow that mentions its owner also carries a plain `["p", <owner>]`, which
`#p` matches as usual. To find a workflow's messages by owner, read the tags
of the channel's messages: `buzz:workflow-owner` also names the owner. A
marked `p` tag on any other event, for example a kind:39002 member list role,
matches `#p` normally. See `docs/buzz-push.md` for the push rules.

### Workflow Mentions

A workflow message may carry `["buzz:workflow-mention", <pubkey>]` tags. Each
one names a channel member whom the workflow's owner named in the stored text of
the step, as opposed to text substituted into it when the step ran. The relay adds
a plain `["p", <pubkey>]` for every member that the rendered text mentions, and a
`buzz:workflow-mention` tag only for members that the stored text also names.

A `buzz:workflow-mention` tag is the relay's statement that the message may
direct the named agent on the owner's behalf. An agent acts on a workflow
message only if all of these hold:

- the message is signed by the relay key that the active community advertises;
- a `buzz:workflow-mention` tag names the agent;
- the workflow principal that `buzz:workflow-owner` names passes the receiving
  client's rule for whom the agent accepts work from: its owner, or an agent
  that its owner has attested.

`buzz:workflow-owner` is attribution. It does not by itself authorize anything.
A plain `p` tag on a workflow message notifies the member but does not direct
an agent.

A relay may leave out `buzz:workflow-mention` tags under a policy. The message
still posts with its `p` tags, and an agent that no tag names does not act on
it. A relay's NIP-11 document MUST carry `workflow_agent_mentions`: `"any"`
when every run may carry the tags, or `"owner"` when only runs the workflow's
owner or a timer started, on a definition of only `send_message` and `delay`
steps, carry them; a client MUST treat a missing field as an older relay whose
policy is unknown. For Buzz's owner-only agent mentions, see
[docs/operator-workflow-agent-mentions.md](docs/operator-workflow-agent-mentions.md).

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

# Subscribe to reactions to channel messages — include #h for live delivery (see note below)
nak req -k 7 --tag "h=<channel-uuid>" --stream \
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

> **Note:** The relay derives a reaction's channel from its `#e` target (client `#h` is
> ignored for channel determination). Reactions to channel-scoped events are therefore
> channel-scoped. Live fan-out keeps channel-scoped and global subscriptions strictly
> separate, which means a kinds-only subscription (`{"kinds":[7]}`) receives none of
> those reactions — subscribe with `{"kinds":[7],"#h":["<channel-uuid>"]}` instead.
> `#h` matching works whether or not the signed reaction carries an `h` tag: explicit
> `h` tags are matched directly, and tagless reactions match via their stored channel.

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
# Rewrite the kind:13534 member list from the stored members (repairs a missing or stale list)
# Repairs only the community RELAY_URL maps to; on a multi-community relay, run it once per community host
docker compose exec relay buzz-admin republish-members
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

- [nostr-protocol/nips](https://github.com/nostr-protocol/nips) — the upstream NIP specifications (NIP-01, NIP-29, NIP-42, and the other NIPs referenced throughout this guide).
- [`docs/nips/`](docs/nips/) — Buzz's own NIP extension documents.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — event kinds, wire protocol, and relay internals.
