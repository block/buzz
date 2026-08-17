---
title: "NIP-PL — Push Leases (full normative draft)"
tags: [nostr, nip, push-notifications, buzz, draft]
status: draft
created: 2026-07-02
---

NIP-PL
======

Push Leases
-----------

`draft` `optional` `relay`

**Depends on**: NIP-01, NIP-11, NIP-40 (expiration), NIP-42 (authentication), NIP-44 (encryption). Interacts with NIP-46 (remote signers) and NIP-59 (gift wrap, never decrypted by executors).

## Abstract

This NIP defines the **push lease**: a stored, installation-scoped, expiring authorization asking a **push executor** (usually the user's relay) to keep a constrained Nostr filter active after the client's socket closes, and to *wake* a specific application installation through a platform push transport (APNs, FCM, optionally UnifiedPush) when the filter matches.

The push payload is a **wake signal** authored entirely by the configured transport service: a fixed reconnect instruction, never relay-supplied bytes, event ids, event content, URLs, ciphertext, or extensible custom data. On wake, the client reconnects and fetches authoritative events over normal `REQ`. Push delivery is lossy and best-effort — duplicates and omissions are both possible; the relay remains the single source of truth. Platform transports are execution profiles for the lease, not the protocol's content plane.

A lease is a `kind:30350` addressable event: `d` is a random per-origin installation id, `expiration` is public and mandatory, and everything else — transport endpoint, subscriptions, priority classes — is NIP-44-encrypted to the executor's advertised key.

## Motivation

Nostr is pull-based. Mobile operating systems terminate background sockets within seconds, so reliable notification requires a server-side component that watches on the client's behalf and wakes it through the platform's push channel.

Prior art models the *transport artifact* as the protocol object: notepush registers raw APNs device tokens against a bespoke HTTP API; the NIP-9a draft (kind:30390) registers an arbitrary HTTP callback URL that receives full event JSON. Both put platform plumbing at the center and push semantics at the edge. This NIP inverts that: the protocol object is the *authorization* — a signed, expiring, revocable filter, the thing Nostr already has language for. Which vendor executes the wake is a profile detail.

The design goals, in order: (1) the push path must not become a shadow feed — no event content transits Apple or Google; (2) notification must be structurally non-amplifying — a lease that can only match a narrow, authenticated slice of the stream cannot be weaponized into a firehose; (3) installations are sovereign — independently created, replaced, and revoked, with no cross-device coupling; (4) multi-tenant executors preserve community isolation on the push path exactly as relays do on the read path.

## Non-Goals

This NIP does not define durable message delivery, delivery receipts, or acknowledgement semantics. Duplicate wakes are valid and harmless; clients deduplicate fetched events by id.

This NIP defines exactly one notification meaning: reconnect to locally configured relays. Rich previews and relay-supplied notification content are out of scope and MUST NOT transit the push transport.

This NIP does not define read state (see NIP-RS), reminders (see NIP-ER), or notification preferences as service-side flags — preferences are expressed as subscriptions and classes inside the lease.

Executors never decrypt the NIP-44 or NIP-59 payloads of the events they match. (The executor necessarily decrypts *lease* content, which is encrypted to it.)

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **installation**: one install of one application on one device. Each `(installation, origin)` pair is identified by a lease `d` value.
- **push lease (lease)**: the `kind:30350` addressable event authorizing wakes for one installation.
- **executor**: the logical component that stores leases, matches events, and sends platform pushes. It is trusted by and operates for the origin, holds the descriptor's private decryption keys, and shares the origin's read-authorization state. It is usually the user's relay; it MAY be deployed as a separate process holding the app's transport credentials, but that separation is deployment topology, not a protocol boundary — **this NIP defines no protocol by which an untrusted third party can act as an executor.**
- **origin**: the canonical origin identifier the descriptor advertises for a relay/community; the tenant key (see Acceptance and Origin Binding).
- **wake signal**: the fixed, transport-authored reconnect payload defined in Wake Delivery. It contains no relay-supplied application data.
- **subscription**: one `{filter, class, ignore?, suppress?}` entry inside a lease.
- **priority class**: one of `silent`, `default`, `time_sensitive`, `urgent`.
- **transport profile**: the APNs/FCM/UnifiedPush-specific execution rules for a lease.

## The Lease Event

`kind:30350` is an addressable event keyed by `(pubkey, 30350, d)` per NIP-01.

```jsonc
{
  "kind": 30350,
  "pubkey": "<installation owner>",
  "created_at": 1769990000,
  "tags": [
    ["d", "<random-installation-id>"],
    ["expiration", "<unix-seconds>"],
    ["exec", "<executor-key-id>"],
    ["alt", "Push lease"]
  ],
  "content": "<nip44-ciphertext to the executor's advertised pubkey>"
}
```

- `d` MUST be generated from at least 128 bits of randomness by the installation, and MUST be distinct per origin — cross-origin unlinkability is a guarantee of this NIP, not a nicety. It MUST NOT contain or be derived from a hardware identifier, advertising identifier, APNs token, FCM registration token, UnifiedPush endpoint, or other transport identifier. Reinstalling the application MUST create a new `d`; transport-token rotation within the same installation MUST retain `d` and replace the existing lease.
- `expiration` (NIP-40) is REQUIRED and MUST satisfy `now − allowed_skew < expiration ≤ now + max_lease_ttl` at acceptance (`invalid: lease ttl too long` / `invalid: lease already expired`; `max_lease_ttl` descriptor-advertised, default 30 days; RECOMMENDED `allowed_skew` 15 minutes). The executor MUST stop matching once it passes. Inactive (tombstone) replacements carry a public `expiration` under the same bound; it dates the tombstone, not any matching. Expiry is the self-healing backstop for every abuse and leak below.
- `exec` names the descriptor encryption key the content was produced for (see Executor Discovery).
- Public tags are exactly one `d`, one `expiration`, one `exec`, and at most one `alt`, each with exactly one value; duplicated tags, extra tags, or extra tag values MUST be rejected. The executor MUST reject a lease carrying filter, kind, author, endpoint, or platform data in public tags.

### Content

`.content` MUST be NIP-44 ciphertext to the executor's advertised encryption pubkey. Plaintext:

```jsonc
{
  "v": 1,
  "origin": "<origin id, byte-for-byte from the descriptor>", // tenant binding, verified — never routed on
  "app_profile": "com.example.app/ios",      // selects transport credentials
  "transport": "apns",                       // "apns" | "fcm" | "unifiedpush"
  "endpoint": "<opaque transport endpoint>", // APNs token / FCM token / UP URL
  "generation": 3,                           // strictly increasing per lease address
  "active": true,                            // false = revocation tombstone
  "subscriptions": [
    { "filter": { "kinds": [9], "#p": ["<self>"] }, "class": "time_sensitive" },
    { "filter": { "kinds": [9], "#h": ["<channel-uuid>"] }, "class": "default",
      "ignore": [ { "kinds": [9], "authors": ["<noisy-bot>"], "#h": ["<channel-uuid>"] } ],
      "suppress": { "p_tags_max": 20 } }
  ]
}
```

The plaintext MUST be a single JSON object. Parsers MUST reject duplicate object keys anywhere in the plaintext, and executors MUST reject a plaintext containing members not defined for its `v` (`invalid: unknown field`) — schema evolution happens by version bump, not by silent extension. Size bounds are advertised in the descriptor and enforced before parsing: `.content` ciphertext ≤ `max_content_len` bytes, decrypted plaintext ≤ `max_plaintext_len` bytes, `d` ≤ 64 bytes, `endpoint` ≤ `max_endpoint_len` bytes, every string value ≤ `max_string_len` bytes.

**Schema (v=1).** For an active lease, required members are exactly `v`, `origin`, `app_profile`, `transport`, `endpoint`, `generation`, `active`, `subscriptions`; there are no optional top-level members. Types: `v` is a non-negative integer ≤ 2^53−1 and `generation` is a positive integer ≤ 2^53−1; `active` is a JSON boolean; `origin`, `app_profile`, `transport`, `endpoint` are strings; `subscriptions` is a non-empty array of subscription objects, each with required `filter` (object) and `class` (string from the class registry) and optional `ignore` (array of filter objects) and `suppress` (object with the single member `p_tags_max`, a positive integer). All timestamps anywhere in this NIP are integer Unix seconds; all descriptor limits are positive integers.

Validation is fail-closed: if any rule in this document fails, the executor MUST reject the entire lease with `invalid: <reason>` without disturbing a previously accepted lease at the same address.

### Acceptance and Origin Binding

`origin` is the tenant key, so no client-supplied value may ever *select* a tenant — it may only *confirm* one. The descriptor (see Executor Discovery) advertises a single canonical `origin` string for the relay/community it describes. The receiving server resolves the tenant from the authenticated connection the event arrived on (which relay/community endpoint, which community context), never from the lease. The lease's encrypted `origin` MUST then compare byte-for-byte equal to that server-resolved tenant's canonical origin; mismatch is rejected (`invalid: origin mismatch`). No normalization algorithm is defined or needed: clients copy the descriptor value verbatim. Executors MUST NOT route, partition, or match based on a client-supplied origin that has not passed this check.

A `kind:30350` event MUST be accepted only when all of the following hold, evaluated in order; the first failure determines the `OK` message:

1. The connection is NIP-42 authenticated and the authenticated pubkey equals the event `pubkey` (`auth-required:` / `restricted: pubkey does not match authenticated user`).
2. The event signature and id verify per NIP-01 (`invalid: bad signature`).
3. Public tags are exactly `{d, expiration, exec, alt?}` and pass the tag rules above (`invalid: <tag reason>`).
4. `exec` names a key the descriptor currently accepts, and `.content` decrypts under NIP-44 with that key (`invalid: unknown executor key` / `invalid: undecryptable content`).
5. The plaintext passes the size, duplicate-key, unknown-field, and schema checks above (`invalid: <schema reason>`).
6. `origin` passes the byte-equality binding check (`invalid: origin mismatch`).
7. If `active` is `true`: `app_profile` is advertised in the descriptor and `transport` equals the advertised transport of that selected `app_profile` entry (`invalid: transport mismatch`), every subscription passes the filter grammar, every `class` is advertised as supported for the lease's transport (`invalid: class not supported`), and quotas hold — including endpoint uniqueness (see Lifecycle), which is evaluated and enforced inside the same atomic acceptance transaction as step 8's commit, so two racing leases cannot both claim an endpoint. If `active` is `false`: the minimal inactive schema applies instead (see Lifecycle) and endpoint/app-profile availability MUST NOT be re-checked — revocation must never be blocked by a withdrawn profile.
8. If a lease was previously accepted at this `(pubkey, 30350, d)` address, the incoming event MUST win on **both** orderings: (a) it wins exact NIP-01 addressable-event ordering against the currently stored winner (greater `created_at`; tie broken by lexically lowest event id), and (b) its `generation` is strictly greater than the internal generation watermark for the address. Failing either check rejects the event (`invalid: stale replacement` / `invalid: stale generation`) and MUST leave the stored event, effective push state, and watermark all unchanged — so a malicious high-generation, old-`created_at` event cannot poison the watermark.

On acceptance the executor returns `OK true` and commits the stored event, the effective push state, and the generation watermark in one atomic transaction; after a crash or restart, effective state MUST be reconstructible from (or restored consistently with) that transactionally persisted state — a rebuilt view MUST never disagree with what `REQ` serves.

`REQ` and `COUNT` for `kind:30350` MUST be answered only on a NIP-42-authenticated connection and MUST return only events whose author equals the authenticated pubkey; to all other queriers the kind behaves as if no such events exist (no existence, count, tag, or content leakage). NIP-42 authentication is a precondition of this ACL, not a substitute for it.

### Filter Constraints

Each subscription `filter` is a NIP-01 filter object under these restrictions — a *restriction* of NIP-01, so the executor's existing matcher runs unchanged and all grammar work is sunk at write time:

1. **Narrowing selector.** Each filter MUST contain at least one of: `#p` (self only), `#h` (1–`max_h` channels), or `authors` (1–`max_authors` pubkeys). Bare kinds-only, since-only, or empty filters MUST be rejected (`invalid: lease filter not narrowed`).
2. **Exact values only.** Every `authors` and `#p` value MUST be exactly 64 lowercase hex characters (a full pubkey), and every `#e` value exactly 64 lowercase hex characters (a full event id); anything shorter, longer, or mixed-case is rejected (`invalid: non-exact match value`). This forecloses NIP-01 prefix matching from inside a lease. Each `#h` value MUST be a non-empty string of at most `max_string_len` bytes and MUST additionally satisfy the channel-identifier grammar the descriptor names in `h_grammar` (e.g. `"uuid-v4-lowercase"` for Buzz); an executor MUST reject values failing its advertised grammar.
3. **Self-scoped `#p`.** Every `#p` value MUST equal the lease author (`invalid: p-tag must be self`). A lease MUST NOT register a wake on another user's mentions — that is a surveillance primitive, and it would signal the existence of events the author may not read.
4. **Bounded, allow-listed kinds.** Each filter MUST include `kinds` (1–`max_kinds` entries), each drawn from the executor's advertised `push_kinds` (`invalid: kind not push-eligible`). Ephemeral kinds (20000–29999), presence, typing, and relay-signed snapshot kinds MUST NOT be push-eligible.
5. **No time-travel, no ids, no limit, no search.** `since`, `until`, `ids`, `limit`, and `search` MUST be rejected, not silently ignored. The lease's liveness window is its `expiration`; `ids` waking is nonsensical for future events.
6. **Tag hygiene.** Only `#p`, `#h`, `#e` selectors are permitted; `#p` and `#e` each have 1–`max_tag_values` values, while `#h` has 1–`max_h` values; empty tag arrays, unknown filter members, and multi-letter tags MUST be rejected. `#e` ("this thread") is permitted but is not a narrowing selector on its own.

### Suppression

A subscription MAY carry `ignore` (≤ `max_ignore` NIP-01 filters) and `suppress` (`p_tags_max` ≥ 1). Suppression evaluates after a positive match: if the matched event matches any `ignore` filter or carries more than `p_tags_max` `p` tags (the hellthread gate), the wake is dropped. `ignore` filters obey the grammar above *except* the narrowing-selector rule — they only subtract from an already-narrowed stream and cannot amplify. Suppression is safe to skip: a minimal executor MAY ignore it and remain correct, since extra wakes are harmless. Consequently a client MUST NOT infer from any observed behavior that suppression was enforced; it is best-effort noise reduction, not policy.

### Priority Classes

Each subscription carries exactly one `class`:

| Class | Meaning | APNs `interruption-level` | Android importance |
|---|---|---|---|
| `silent` | Sync-only wake, no alert | not user-visible; see APNs profile | `IMPORTANCE_MIN` |
| `default` | Standard notification | `active` | `IMPORTANCE_DEFAULT` |
| `time_sensitive` | Breaks through Focus/DND within OS policy | `time-sensitive` | `IMPORTANCE_HIGH` |
| `urgent` | Reserved: approval gates | `critical` if entitled, else `time-sensitive` | `IMPORTANCE_HIGH` + full-screen intent where policy allows |

Classes are strictly ordered: `silent` < `default` < `time_sensitive` < `urgent`. When one deduplicated wake covers matches from multiple subscriptions or leases targeting the same endpoint (see Coalescing), the wake's effective class is the highest eligible class among those matches. The descriptor's `class_support` is authoritative: a lease naming a class unsupported for its transport MUST be rejected at acceptance (`invalid: class not supported`), never silently downgraded.

The executor MUST restrict `urgent` to the descriptor-advertised allow-list of approval-request kinds whose eligibility is decidable from the public event envelope (`invalid: class not permitted for kind`). Urgent DMs are explicitly out of scope for v1: gift-wrapped DM content is opaque to the executor, so no privacy-safe urgency marker exists yet; a future revision may add one.

`silent` remains a matching preference only. The public Buzz APNs profile sends the one fixed reconnect alert and does not expose relay-selected notification classes to the transport boundary.

Clients MUST NOT register any lease or subscription as a side effect of joining a channel or surface — absent explicit user opt-in the notifiable set is empty.

### Quotas

A lease address `(pubkey, 30350, d)` holds exactly one effective lease, and `d` MUST be distinct per `(installation, origin)` — a fresh random value per origin, so leases at different origins are unlinkable. Additionally, at most one active lease per `(author, origin, app_profile, transport, endpoint)` may exist: an executor MUST reject an active lease whose endpoint tuple duplicates another of the same author's active leases at a different address (`invalid: endpoint already leased`) — this keeps endpoint identity unambiguous for deduplication and class resolution. Quotas: per pubkey per origin, ≤ `max_leases_per_pubkey` active lease addresses; per lease, ≤ `max_subscriptions_per_lease` subscriptions. Because a lease is addressable, the normal client flow replaces rather than accumulates; quota rejection (`invalid: lease quota exceeded`) MUST NOT disturb existing valid leases.

## Executor Discovery

Until this draft has an upstream NIP number, executors MUST NOT advertise it in NIP-11 `supported_nips`; they advertise `"nip-pl"` in NIP-11 `supported_extensions` (NIP-ER precedent) together with a descriptor:

```jsonc
{
  "push": {
    "origin": "wss://relay.example",         // canonical origin id; copied verbatim into lease content
    "keys": [ { "id": "2026-06", "pubkey": "<hex>", "current": true },
              { "id": "2026-01", "pubkey": "<hex>", "retiring": true } ],
    "app_profiles": [ { "id": "com.example.app/ios", "transport": "apns" },
                      { "id": "com.example.app/android", "transport": "fcm" } ],
    "push_kinds": [9, 1059, 40007, 46010, 7],
    "urgent_kinds": [46010],
    "h_grammar": "uuid-v4-lowercase",
    "class_support": { "apns": ["silent","default","time_sensitive","urgent"],
                       "fcm": ["silent","default","time_sensitive","urgent"] },
    "limitation": {
      "max_lease_ttl": 2592000,
      "max_leases_per_pubkey": 16,
      "max_subscriptions_per_lease": 16, "max_kinds": 16,
      "max_authors": 20, "max_h": 50, "max_tag_values": 20, "max_ignore": 8,
      "max_content_len": 65536, "max_plaintext_len": 32768,
      "max_endpoint_len": 4096, "max_string_len": 512
    }
  }
}
```

A descriptor is valid only if: exactly one key is marked `current` and key ids are unique; app-profile ids are unique; `endpoint` is an `https://` URL; `urgent_kinds ⊆ push_kinds`; and every `class_support` value comes from the class registry in this NIP. Clients MUST treat a descriptor failing these checks as absence of push support.

The executor URL and credentials come from the descriptor, never from the lease. A lease cannot point the executor at an arbitrary HTTP endpoint; this removes the callback-amplification class of attack entirely. Executors MUST NOT dereference a client-supplied `endpoint` URL except as the selected transport profile explicitly defines (UnifiedPush is the only profile whose endpoint is a URL, and it is validated per that profile before use).

Leases MUST be author-only reads, as specified in Acceptance and Origin Binding, following the NIP-ER access pattern.

## Matching Semantics and Tenant Isolation

An executor MUST evaluate a lease only against events accepted by the relay origin named by that lease. A match does not grant access to an event: before enqueueing a wake, the relay MUST verify that the lease author is authorized to read the event at that origin at match time. Authorization established when the lease was created is insufficient, because membership and other read permissions may subsequently change. **A lease is a wake request, never a read grant.**

Filter matching MUST use only the accepted event envelope and relay-local authorization state. An executor MUST NOT decrypt NIP-44 content, NIP-59 seals or gift wraps, or any other encrypted event content to decide whether to wake an installation. For NIP-59 gift wraps, only outer-envelope fields, including the outer `p` tag, are eligible for matching.

The verified canonical origin is part of every lease and match key. An executor serving more than one origin MUST partition, at minimum, lease state, filter indexes, cursors, durable outbox jobs, endpoint lookup, foreground-suppression state, gateway capability state, quotas, and rate limits by origin. It MUST NOT match a lease against a global event, pubkey, or tag stream, and MUST NOT use authorization state from one origin to approve a wake or gateway delivery at another origin.

A wake job MUST preserve the origin and lease address selected at match time. Workers MUST re-check the lease's active state, expiration, endpoint generation, and current read authorization before delivery. A failed authorization check MUST suppress that wake without revealing whether the event existed. Implementations SHOULD make the accepted event and outbox insertion one durable transaction, or provide equivalent crash-safe processing, but delivery through a platform transport remains best-effort.

Separate origins may independently wake the same installation for the same event. Such duplicate wakes are valid; clients deduplicate authoritative events by event id after fetching them from their respective origins.

## Wake Delivery

Every conforming transport sends only a fixed **reconnect** signal. The transport service, not the relay/executor, MUST construct the complete application payload. A relay request MUST NOT contain notification text, title, subtitle, URL, deep link, event or lease identifier, channel, sender, count, ciphertext, generic JSON, extension map, or any other application-content field. Unknown request members MUST be rejected rather than ignored.

For every actual platform-send attempt `a`, the application body MUST satisfy `application_body(a) = C_transport`, where `C_transport` is one documented byte constant selected only by gateway deployment/profile. The equality quantifies over all accepted relay bodies, signatures, grants, endpoints, request identifiers, expirations, profiles, and provider responses. A transport MAY vary only explicitly enumerated platform routing controls that are not application-body bytes: destination, authenticated provider topic/environment, expiration, provider request id, push type, and priority. These values MUST NOT be copied into the application body. Timing and frequency remain observable transport metadata and MUST be bounded by gateway-owned abuse controls.

On receipt, the application reconnects using relay/account state already stored locally and fetches authoritative events through ordinary authenticated `REQ`. The push signal carries no origin or relay selector; clients MAY sync every locally configured origin. There is no wake-grant or rich-preview payload in this version.

## Transport Profiles

Common invariant, all transports: the application payload is a transport-owned reconnect constant and MUST NOT depend on relay input, event data, or fetch success.

### APNs

The APNs application body is the exact UTF-8 byte constant `{"aps":{"alert":{"body":"Reconnect to your relay now"},"mutable-content":1}}`. It has no custom member, event identifier, unread count, or relay-supplied byte. The constant mutable-content flag lets the Buzz Notification Service Extension compute a local badge and, when separately authorized data is available, replace the generic text; the gateway does not carry that data. The gateway MUST send that exact body for every accepted APNs attempt; it MUST NOT serialize any relay request, endpoint grant, provider response, or generic JSON value into the body. `apns-topic`, environment, credentials, push type `alert`, and priority `10` come only from gateway configuration. `apns-id` is a canonical UUID and `apns-expiration` is bounded by the endpoint capability and a gateway-local ceiling.

### FCM

A future FCM profile MUST define one gateway-owned constant data message with identical noninterference semantics. Until that constant and its wire tests are registered, FCM is not a conforming v1 public-gateway profile.

### UnifiedPush (optional)

UnifiedPush is not a conforming public-gateway profile in v1 because arbitrary distributor endpoints and message bodies do not meet the fixed-payload authority boundary. A future profile requires a separately registered constant body and hostile-endpoint analysis.

## Lease and Key Lifecycle

A lease is identified by `(author, kind, d)`. A replacement supersedes the prior lease at the same address only by passing the full acceptance sequence, including winning both NIP-01 addressable ordering and the strictly-increasing generation watermark (check 8). Any rejected replacement — stale by either ordering, or invalid for any other reason — MUST leave the stored event, effective push state, and watermark unchanged.

An active lease becomes ineffective when its `expiration` passes. Executors MUST NOT match, enqueue, or deliver wakes for an expired lease. Clients SHOULD refresh active leases before expiry; failure to refresh MUST NOT extend the prior lease. Expiry is a safety backstop, not evidence that a platform endpoint has been deleted.

**Revocation.** Revocation is exclusively a higher-generation replacement with the minimal inactive plaintext — exactly `{"v", "origin", "generation", "active": false}`; `app_profile`, `transport`, `endpoint` and `subscriptions` MUST be absent. NIP-09 deletion is unsupported for `kind:30350`: relays MUST ignore deletion requests targeting this kind, so the stored/effective/watermark invariant has exactly one transition path. The executor validates the inactive schema without consulting endpoint or app-profile availability, so revocation succeeds even after an app profile or transport has been withdrawn from the descriptor. On acceptance the executor MUST treat it as a tombstone for that lease address: stop matching, cancel undelivered jobs where practical, and delete transport endpoint material when no longer required for audit or abuse prevention. Reactivation is an ordinary active replacement with a yet-higher generation. The executor MUST persist the generation watermark for a lease address until at least `max(last_active_expiration, tombstone_accepted_at + max_lease_ttl) + allowed_skew` when a tombstone exists, or `last_active_expiration + allowed_skew` when none does (after which any replay fails the expiration lower bound) — or a longer descriptor-advertised fixed retention — so a replayed older event can never resurrect a revoked lease. Logging out one installation MUST NOT alter sibling installation leases.

**Endpoint rotation.** When a platform rotates an endpoint token, the client MUST publish a replacement at the same lease address with an incremented `generation` and the new endpoint encrypted in `content`. The executor MUST deliver only to the highest accepted generation. A permanent invalid-endpoint response from a transport MUST disable only that endpoint generation; it MUST NOT revoke the author's identity or affect sibling leases. A later valid replacement with a newer generation MAY reactivate the lease. Executors SHOULD apply bounded retries to transient transport failures without changing the accepted lease.

Each encrypted lease MUST identify the descriptor encryption key for which its content was produced. A descriptor MUST advertise one current encryption key and MAY advertise retiring keys together with their identifiers. On rotation, an executor MUST either retain each retiring private key for at least the maximum lease lifetime advertised while that key was current, plus allowed clock skew, or retain the endpoint material already decrypted from accepted leases until those leases expire or are revoked. Key rotation MUST NOT silently invalidate an accepted lease.

Clients SHOULD replace leases under the descriptor's current key before their existing leases expire. An executor MUST reject a replacement encrypted to an unknown or no-longer-accepted key without disturbing the prior valid lease. After a retiring key's acceptance window closes, executors MUST reject new leases encrypted to that key and SHOULD erase its private material once no accepted lease or operational recovery window requires it.

## Remote Signers

This NIP introduces no delegation mechanism. A client whose user key is held by a NIP-46 remote signer creates the same root-authored lease as a local-key client. It asks the signer to perform `nip44_encrypt` to the executor's advertised encryption pubkey and `sign_event:30350` for the completed lease. When the relay requires NIP-42 authentication, the client must also be able to obtain the required kind `22242` AUTH signature, for example through the corresponding `sign_event:22242` signer permission. The relay applies identical authentication, signature, replacement, and authorization rules regardless of signer location.

A client SHOULD request only the NIP-46 permissions needed for these operations. The executor MUST NOT accept a NIP-46 client transport key, bunker URL, connection secret, authorization URL, or signer session as a substitute for a lease signed by the user's pubkey. Clients MUST NOT place such signer material in public tags or encrypted lease content.

A pubkey-only client cannot create, replace, or revoke a lease. If a platform endpoint rotates while the remote signer is unavailable, the client MUST NOT publish an unsigned update or reuse another installation's authorization. It SHOULD queue the replacement until the signer is available; the existing lease remains bounded by its expiry and the executor's permanent-endpoint-error handling.

Implementations MUST NOT interpret this section as NIP-26 delegation. A future specification may define a narrowly scoped installation authorization for unattended endpoint rotation, but such a capability is neither required nor implied here.

## Public APNs Gateway Profile (Buzz, normative)

This section registers the public last-hop profile served at `https://push.buzz.xyz`. It is an optional profile of NIP-PL, but every requirement in this section is normative for implementations that use it. The gateway is stateful: it retains installation authority, encrypted APNs-token custody, relay delegations, replay reservations, and endpoint quotas. The relay remains the executor and retains lease acceptance, matching, tenant authorization, endpoint uniqueness, coalescing, durable jobs/retries, and lease-generation invalidation.

### Registered values and lease mapping

The registered `app_profile` values are `buzz-ios-production` (Apple production APNs environment) and `buzz-ios-sandbox` (Apple sandbox APNs environment). A gateway deployment MUST enable only profiles for which its App Attest application identifier, APNs topic, credentials, and APNs environment are configured consistently. The APNs token registered with the gateway is called the **installation endpoint** and never leaves gateway custody after enrollment.

The opaque string returned as `endpoint_grant` by `POST /v1/delegations` is the **delivery capability**. For this profile, the active lease plaintext's `endpoint` member MUST contain that `endpoint_grant`, not the raw APNs token. `transport` MUST be `apns`, and `app_profile` MUST equal the profile sealed into the grant. Base-protocol endpoint uniqueness, rotation, hashing, and coalescing operate on this opaque lease `endpoint` within an origin. A capability is scoped to one installation, relay signing pubkey, endpoint epoch, generation, and expiry; grants independently issued to different relays are intentionally distinct. The gateway separately enforces global installation-endpoint uniqueness using `(app_profile, SHA-256(token))`. A public-profile relay MUST treat `endpoint` as opaque and MUST NOT parse or transform it.

### Common HTTP and value rules

All routes below accept only `POST`. Clients MUST send `Content-Type: application/json`; bodies are UTF-8 JSON and MUST be at most 8192 bytes. Every request object is closed: unknown members, duplicate members at any depth, missing or incorrectly typed members, trailing non-whitespace data, or a `v` other than integer `1` are `400 {"error":"invalid_request"}`. Integers are signed JSON integers in the ranges stated below. Unix times are integer seconds. UUIDs use the canonical lowercase hyphenated representation. Relay pubkeys are exactly 64 lowercase hexadecimal characters. APNs endpoints are non-empty, even-length lowercase hexadecimal strings encoding at most 512 bytes. Challenges are exactly 32 bytes encoded as unpadded URL-safe base64. `key_id`, `attestation`, and `assertion` use padded or unpadded standard base64 as accepted by Apple's App Attest API; decoded key ids are exactly 32 bytes, attestations are 1..16384 bytes, and assertions are 1..1024 bytes. An `endpoint_grant`, including its key-id prefix, MUST be at most 4096 bytes.

Successful and error responses are UTF-8 `application/json`. Closed error bodies are `{"error":"invalid_request"}`, `{"error":"invalid_attestation"}`, `{"error":"not_authorized"}`, `{"error":"invalid_auth"}`, `{"error":"invalid_grant"}`, `{"error":"temporarily_unavailable"}`, `{"error":"configuration_fault"}`, or `{"error":"not_ready"}`. Authority/custody/quota rejection MUST NOT reveal whether an installation, delegation, or endpoint exists. In particular, delivery grant/authority/replay/quota failures collapse to `404 invalid_grant`; storage failures use `503 temporarily_unavailable`.

### Exact App Attest transcript construction

Every App Attest operation signs a **transcript**, not the received request bytes. Transcript bytes are UTF-8 bytes of:

```
<domain> + "\\n" + <compact ordered JSON object>
```

The JSON object has no insignificant whitespace and members appear in the exact order shown below. Strings use JSON escaping for quotation mark, reverse solidus, and U+0000..U+001F; all authority-bearing strings admitted by this profile are ASCII. UUID strings are canonical lowercase-hyphenated. Integers use shortest decimal notation. The fixed `audience` value is part of the signed object and prevents cross-route use. For enrollment, these exact transcript bytes are the App Attest `clientData` supplied to attestation verification. For every assertion route, `clientDataHash = SHA-256(transcript bytes)` is verified by App Attest. The separately stored challenge must equal the request `challenge`, is single-use, expires after 300 seconds, and is consumed only after successful cryptographic verification. Assertion `signCount` MUST strictly increase atomically for the installation.

### Challenge

`POST /v1/installations/challenges`

Request: `{"v":1}`.

Success `200`:

```json
{"challenge_id":"<uuid>","challenge":"<base64url-no-pad-32-bytes>","expires_at":<unix-seconds>}
```

The challenge is single-use. Invalid input is `400 invalid_request`; storage/randomness failure is `503 temporarily_unavailable`.

### Installation enrollment

`POST /v1/installations`

Request members, in any request order:

```json
{"v":1,"challenge_id":"<uuid>","challenge":"<challenge>","key_id":"<standard-base64>","attestation":"<standard-base64 CBOR>","app_profile":"buzz-ios-production","endpoint":"<lowercase APNs-token hex>","endpoint_epoch":1,"expires_at":<unix-seconds>}
```

`expires_at` MUST satisfy `now < expires_at <= now + configured_max_installation_lifetime`; the selected profile MUST be enabled. The exact transcript is domain `buzz.push.enroll.v1` followed by this ordered object:

```json
{"v":1,"audience":"https://push.buzz.xyz/v1/installations","challenge_id":"<uuid>","challenge":"<challenge>","key_id":"<standard-base64>","app_profile":"<registered-profile>","endpoint":"<lowercase-hex>","endpoint_epoch":1,"expires_at":<unix-seconds>}
```

The gateway verifies Apple's attestation chain, configured application identifier, production AAGUID, key identifier, and transcript. Apple documents no APNs-token-to-App-Attest-key binding; token provenance at enrollment is an explicit bootstrap assumption. It then stores only encrypted token custody plus its fingerprint. Success `201`:

```json
{"installation_handle":"<uuid>","endpoint_epoch":1,"expires_at":<unix-seconds>}
```

Invalid attestation is `401 invalid_attestation`; a consumed/expired challenge or duplicate key/token is `404 not_authorized`.

### Relay delegation and capability issuance

`POST /v1/delegations`

```json
{"v":1,"challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","endpoint_epoch":<positive-integer>,"generation":<positive-integer>,"relay_pubkey":"<64-lowercase-hex>","not_before":<unix-seconds>,"expires_at":<unix-seconds>,"assertion":"<standard-base64 CBOR>"}
```

`not_before <= now + 300`, `not_before < expires_at`, and `expires_at <= min(now + configured_max_grant_lifetime, installation.expires_at)`. The endpoint epoch MUST equal the current installation epoch. For each `(installation_handle, relay_pubkey)`, generation MUST strictly increase. Transcript domain `buzz.push.delegate.v1`; ordered object:

```json
{"v":1,"audience":"https://push.buzz.xyz/v1/delegations","challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","endpoint_epoch":<integer>,"generation":<integer>,"relay_pubkey":"<hex>","not_before":<integer>,"expires_at":<integer>}
```

Success `201`: `{"endpoint_grant":"<opaque-capability>"}`. The sealed grant contains no APNs token. Grant-key rotation MUST retain decrypt-only predecessor keys through the maximum lifetime of grants they issued.

### Endpoint rotation

`POST /v1/installations/endpoint`

```json
{"v":1,"challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","endpoint_epoch":<positive-integer>,"new_endpoint_epoch":<integer>,"endpoint":"<lowercase APNs-token hex>","assertion":"<standard-base64 CBOR>"}
```

`new_endpoint_epoch` MUST equal `endpoint_epoch + 1` without overflow. Transcript domain `buzz.push.rotate-endpoint.v1`; ordered object:

```json
{"v":1,"audience":"https://push.buzz.xyz/v1/installations/endpoint","challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","endpoint_epoch":<integer>,"new_endpoint_epoch":<integer>,"endpoint":"<lowercase-hex>"}
```

A successful atomic rotation invalidates every grant sealed to the old epoch and returns `200 {"status":"rotated"}`.

### Delegation and installation revocation

`POST /v1/delegations/revoke` request:

```json
{"v":1,"challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","relay_pubkey":"<64-lowercase-hex>","generation":<positive-integer>,"assertion":"<standard-base64 CBOR>"}
```

Transcript domain `buzz.push.revoke-delegation.v1`; ordered object:

```json
{"v":1,"audience":"https://push.buzz.xyz/v1/delegations/revoke","challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","relay_pubkey":"<hex>","generation":<integer>}
```

The generation identifies the current delegation generation. Success is `200 {"status":"revoked"}`.

`POST /v1/installations/revoke` request:

```json
{"v":1,"challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","endpoint_epoch":<positive-integer>,"new_endpoint_epoch":<integer>,"assertion":"<standard-base64 CBOR>"}
```

`new_endpoint_epoch` MUST equal `endpoint_epoch + 1` without overflow. Transcript domain `buzz.push.revoke-installation.v1`; ordered object:

```json
{"v":1,"audience":"https://push.buzz.xyz/v1/installations/revoke","challenge_id":"<uuid>","challenge":"<challenge>","installation_handle":"<uuid>","endpoint_epoch":<integer>,"new_endpoint_epoch":<integer>}
```

Success is `200 {"status":"revoked"}`. The revocation atomically invalidates the installation and every delegation.

### Relay delivery

`POST /v1/deliveries/apns` has the exact externally configured URL `https://push.buzz.xyz/v1/deliveries/apns`. Request:

```json
{"v":1,"endpoint_grant":"<opaque-capability>","request_id":"<uuid>","expires_at":<unix-seconds>}
```

The relay supplies a NIP-98 `Authorization: Nostr <standard-base64-event-json>` header for method `POST`, the exact URL above, and the SHA-256 payload hash of the **received request body bytes**. The gateway verifies the NIP-98 event signature, timestamp under NIP-98 rules, method, URL, and payload; the event pubkey is the relay identity. It decrypts `endpoint_grant`, requires that signer, current installation/delegation, endpoint epoch and generation, and both `now <= request.expires_at <= grant.expires_at`. Every NIP-98 event id is burned at admission.

The relay's durable job UUID is `request_id` and becomes the stable APNs `apns-id`. Delivery replay/quota reservation is one transaction. The commit of that transaction is send-begin: a revocation or rotation commit that completes first prevents the old-capability send; a send admitted first may finish. Terminal outcomes retain the `(relay_pubkey, request_id)` reservation; transient/configuration outcomes release it only after provider processing so a fresh NIP-98 event may retry the same job. Endpoint quota is charged once per admitted attempt and never refunded. A crash before transient cleanup can reject that id until its bounded request expiry; exactly-once provider delivery is not guaranteed.

Responses:

- `200 {"status":"accepted"}` — APNs accepted; terminal reservation retained.
- `410 {"status":"invalid_endpoint","generation":<integer>,"invalid_at":<unix-seconds-or-null>}` — permanent endpoint invalidation; terminal reservation retained. The relay applies it only if that generation remains current.
- `503 {"status":"retry","retry_after_seconds":<positive-integer-or-null>}` — transient APNs outcome; request reservation released after processing.
- `503 {"error":"configuration_fault"}` — provider configuration fault; request reservation released after processing.
- `400 {"error":"invalid_request"}` — malformed request or permanent APNs request fault; a provider-reached permanent fault is terminal.
- `401 {"error":"invalid_auth"}` — absent or invalid NIP-98 authorization.
- `404 {"error":"invalid_grant"}` — capability, signer, authority, replay, expiry, or quota rejection.
- `503 {"error":"temporarily_unavailable"}` — durable authority/custody/disposition failure.

The gateway performs one APNs request, except that an APNs expired-provider-token response permits one credential refresh and one retry. The application body is always the exact constant registered in the APNs transport profile above; no request or grant field enters it.

## Implementation Notes (Buzz, non-normative)

Per `RESEARCH/PUSH_RELAY_INTEGRATION.md` (pinned SHA `88c089d`): the lease matcher hooks the generic post-storage dispatch seam (`buzz-relay/src/handlers/event.rs:245 dispatch_persistent_event`), not `handle_side_effects`; Redis pub/sub is community-scoped routing precedent but not the durable offline-matching source; `event_mentions` is a ready indexed primitive for self-`#p` and needs-action subscriptions but is **not** authorization — private-channel wakes re-check same-community visibility at match/send time. Known footgun: some internal producers bypass `dispatch_persistent_event`; implementation must centralize durable dispatch or add push dispatch at each internal publish path.

## Privacy Considerations

What each party learns:

| Party | Learns |
|---|---|
| Platform push service (Apple/Google/distributor) | that a fixed reconnect wake occurred for this app installation, plus timing and enumerated transport metadata; no relay-supplied application bytes |
| Executor / relay | lease filters in plaintext (it must match them), the transport endpoint, and wake timing — this is new information relative to the bare event store, entrusted to the executor because it is the origin's trusted component |
| Other relay users | nothing: leases are author-only reads |

The wake-hint model means notification metadata held by platform vendors reduces to traffic analysis of wake timing. Lease count and replacement cadence are visible to the executor; `d`-randomness prevents linking leases to hardware identities, and per-origin `d` values prevent executors serving multiple origins from linking one installation across them.

## Security Considerations

Amplification is disarmed at write time by construction: no un-narrowed filter, no allow-list-external kind, no time-travel, no callback URLs, exact 64-hex match values (no prefix or glob surface reachable from a lease), byte-bounded content and strings, bounded quotas on every axis, endpoint-unique active leases, and one durable wake job per `(origin, app_profile, transport, H(endpoint), event id)`. Residual matching cost is bounded by the quotas; residual delivery cost by the wake rate cap.

Zombie leases (e.g. `#h` after leaving a channel) are neutralized by match-time authorization re-check; leaked or abandoned leases self-heal at `expiration`. A lease never expands what its author can read: the fixed wake contains no event or relay content, and all reads flow through normal authenticated `REQ`. Compromise of the user key permits lease manipulation as it permits other signed actions, but cannot change the gateway-authored APNs body.


## Registry

- `kind:30350`: push lease (addressable)
- `exec` tag: executor encryption-key identifier for `kind:30350`
- NIP-11 `supported_extensions`: contains `"nip-pl"` pre-numbering; descriptor object `push` as specified in Executor Discovery
- Classes: `silent`, `default`, `time_sensitive`, `urgent`
- `h_grammar` values: `"uuid-v4-lowercase"` (initial entry; origins may register additional grammars with this NIP)
- Public APNs gateway profile: base URL `https://push.buzz.xyz`; app profiles `buzz-ios-production`, `buzz-ios-sandbox`; wire version `1`
