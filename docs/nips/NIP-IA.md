NIP-IA
======

Identity Archival
-----------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-11 (relay information document), NIP-42 (Authentication of Clients to Relays), NIP-43 (Relay Access Metadata and Requests), NIP-70 (Protected Events), NIP-OA (Owner Attestation)

## Abstract

This NIP defines a relay-scoped protocol for archiving and unarchiving identities. An archived identity is a pubkey that the relay says should be hidden from active-member and autocomplete surfaces on that relay, while preserving its historical events and without implying any global reputation state.

The protocol has three event families:

- user-signed requests (`kind:9035` archive request, `kind:9036` unarchive request),
- relay-signed deltas (`kind:8002` archived identity, `kind:8003` unarchived identity), and
- a relay-signed current-state snapshot (`kind:13535` archived identities list).

Relays MAY accept archive and unarchive requests according to local policy. This document defines the wire format, verification rules, and minimum interoperability semantics. The recommended policy recognizes admin requests, self requests, and owner-of-agent requests proven with NIP-OA.

## Motivation

Relays accumulate stale pubkeys. Humans rotate keys, contractors leave, bots are rebuilt, and agents created from temporary worktrees continue to appear in member pickers long after they are useful. Existing Nostr primitives do not cleanly express "this pubkey is retired here; hide it from active UI, but keep its history and do not treat it as globally bad."

NIP-09 deletion requests are authored by the deleted key and are about event removal. They do not help when the old key is lost, and they are too destructive for normal key rotation: David's old messages should remain attributed to David's old pubkey.

NIP-51 mute lists are personal. They require every user to mute the same retired key and do not give the relay a single authoritative view for its own membership and autocomplete surfaces.

NIP-43 membership removal is access control. It answers "may this pubkey connect or publish here?" It does not answer "should this old identity still show up as an active person/bot in UI?" A key can be archived without being banned; a spammer can be both removed via NIP-43 and archived via this NIP.

NIP-IA fills that gap. The relay publishes a transparent, relay-signed archive state. Clients can hide archived identities in relay-scoped UI without rewriting history, deleting events, or treating the archive as a global blocklist.

## Non-Goals

This NIP does not delete events. Historical events authored by an archived pubkey remain valid Nostr events.

This NIP does not define bans, kicks, or relay access revocation. Use NIP-43 membership removal for relay access control.

This NIP does not define global reputation. An archive state from relay A applies only to relay A. Clients MUST NOT use it as a global blocklist or as evidence that other relays should hide the same pubkey.

This NIP does not require relays to accept every request. Request authorization is relay policy. The protocol makes accepted decisions transparent and auditable.

This NIP does not transfer authorship. Owner-of-agent archive requests prove authority to ask for archival; they do not make the owner the author of the agent's historical events.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **relay identity**: The relay signing pubkey advertised in its NIP-11 `self` field. NIP-IA relay-signed events are valid only when signed by this key.
- **target**: The pubkey being archived or unarchived.
- **actor**: The pubkey that signed a `kind:9035` or `kind:9036` request.
- **archived identity**: A target pubkey currently listed in the relay's latest valid `kind:13535` archive snapshot.
- **archive delta**: A relay-signed `kind:8002` event announcing that a target became archived.
- **unarchive delta**: A relay-signed `kind:8003` event announcing that a target became unarchived.
- **archive request**: A user-signed `kind:9035` event requesting that the relay archive a target.
- **unarchive request**: A user-signed `kind:9036` event requesting that the relay unarchive a target.
- **consent path**: The relay-attested reason it accepted a request: `self`, `owner`, `admin`, or `relay`.
- **active member**: A pubkey currently permitted by the relay's authoritative membership/access-control state, normally reflected by NIP-43.

## Kinds

| Kind | Name | Signer | Storage | Purpose |
|------|------|--------|---------|---------|
| `9035` | Archive Request | user / agent | policy-defined; MAY be stored | Ask relay to archive a target |
| `9036` | Unarchive Request | user / agent | policy-defined; MAY be stored | Ask relay to unarchive a target |
| `8002` | Archived Identity | relay | regular | Relay-signed archive delta |
| `8003` | Unarchived Identity | relay | regular | Relay-signed unarchive delta |
| `13535` | Archived Identities List | relay | replaceable | Current relay archive state |

`kind:13535` is replaceable per NIP-01 (`10000 <= n < 20000`). Clients use the latest valid `kind:13535` signed by the relay identity as current state. The snapshot is relay-scoped: it is signed by the relay identity advertised in NIP-11 `self`, mirroring NIP-43's relay-membership snapshot shape. Relays without a stable NIP-11 `self` pubkey MUST NOT publish NIP-IA relay-signed state, because clients would have no stable key against which to verify it.

## Event Formats

### `kind:9035` Archive Request

An archive request is signed by the actor and asks the relay to archive a target.

```jsonc
{
  "kind": 9035,
  "pubkey": "<actor-pubkey-hex>",
  "content": "<optional human-readable reason>",
  "tags": [
    ["-"],
    ["p", "<target-pubkey-hex>"],
    ["reason", "<optional machine-readable reason-code>"],
    ["replaced-by", "<replacement-pubkey-hex>"],
    ["auth", "<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]
  ]
}
```

Required tags:

- exactly one `p` tag identifying the target,
- exactly one NIP-70 `-` tag.

Request events SHOULD be sent to the target relay and need not be useful on any other relay. Relays MAY store accepted requests for audit, but MUST NOT require clients on other relays to process them.

Optional tags:

- `reason`: a short machine-readable reason code. Suggested values include `rotated`, `retired`, `bot-rebuilt`, `left-organization`, and `spam`. Unknown values MUST be ignored by clients.
- `replaced-by`: a replacement pubkey, useful for key rotation. If present, it MUST be a valid 64-character lowercase hex pubkey and MUST NOT equal the target.
- `auth`: a NIP-OA owner-attestation tag. See §Owner-of-Agent Requests.

The `content` field MAY contain a human-readable explanation. Clients MUST NOT parse authorization semantics from `content`.

### `kind:9036` Unarchive Request

An unarchive request is signed by the actor and asks the relay to unarchive a target.

```jsonc
{
  "kind": 9036,
  "pubkey": "<actor-pubkey-hex>",
  "content": "<optional human-readable reason>",
  "tags": [
    ["-"],
    ["p", "<target-pubkey-hex>"],
    ["reason", "<optional machine-readable reason-code>"],
    ["auth", "<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]
  ]
}
```

Required tags:

- exactly one `p` tag identifying the target,
- exactly one NIP-70 `-` tag.

Optional tags are the same as `kind:9035`, except `replaced-by` has no defined meaning on unarchive requests and SHOULD NOT be used.

### `kind:8002` Archived Identity

An archive delta is signed by the relay identity after the relay accepts an archive request or archives an identity by local administrative action.

```jsonc
{
  "kind": 8002,
  "pubkey": "<relay-pubkey-hex>",
  "content": "<optional human-readable reason>",
  "tags": [
    ["-"],
    ["p", "<target-pubkey-hex>"],
    ["consent", "<self|owner|admin|relay>", "<actor-or-owner-pubkey-hex>"],
    ["e", "<request-event-id-hex>"],
    ["reason", "<optional machine-readable reason-code>"],
    ["replaced-by", "<replacement-pubkey-hex>"]
  ]
}
```

Required tags:

- exactly one `p` tag identifying the target,
- exactly one NIP-70 `-` tag,
- exactly one `consent` tag.

The `consent` tag's second element MUST be one of:

- `self`: the target signed the request directly. The third element, if present, MUST equal the target.
- `owner`: an owner signed the request and proved owner-of-agent authority with NIP-OA. The third element MUST be the owner pubkey.
- `admin`: an actor accepted by the relay's local admin policy. The third element MUST be the admin actor pubkey.
- `relay`: the relay archived the identity by local policy without a user request. The third element SHOULD be omitted.

If the delta was caused by a request event, the delta MUST include an `e` tag referencing that request event id. The request event SHOULD be retrievable from the relay for audit. If the relay archived an identity on its own initiative with no request event, the `e` tag MAY be omitted and the `consent` path MUST be `relay`.

### `kind:8003` Unarchived Identity

An unarchive delta is signed by the relay identity after the relay accepts an unarchive request or unarchives an identity by local administrative action.

```jsonc
{
  "kind": 8003,
  "pubkey": "<relay-pubkey-hex>",
  "content": "<optional human-readable reason>",
  "tags": [
    ["-"],
    ["p", "<target-pubkey-hex>"],
    ["consent", "<self|owner|admin|relay>", "<actor-or-owner-pubkey-hex>"],
    ["e", "<request-event-id-hex>"],
    ["reason", "<optional machine-readable reason-code>"]
  ]
}
```

Required and optional tags have the same meaning as `kind:8002`, except `replaced-by` has no defined meaning and SHOULD NOT be used.

### `kind:13535` Archived Identities List

The archive list is the relay's current-state snapshot.

```jsonc
{
  "kind": 13535,
  "pubkey": "<relay-pubkey-hex>",
  "content": "",
  "tags": [
    ["-"],
    ["p", "<archived-pubkey-hex>"],
    ["p", "<archived-pubkey-hex>"],
    ...
  ]
}
```

Required tags:

- exactly one NIP-70 `-` tag.

The NIP-70 marker is intentional on the snapshot even though the snapshot is replaceable. It tells generic relays and clients not to rebroadcast relay-authoritative administrative state outside the relay context where the signing key is meaningful.

Each archived identity is represented by a bare `p` tag whose second element is the archived pubkey. Additional elements on `p` tags are not defined by this NIP and MUST be ignored for archive-state construction. Metadata such as reason, replacement pubkey, actor, and consent path belongs on the `kind:8002`/`kind:8003` deltas, not on the list.

The relay SHOULD publish a new `kind:13535` list after every accepted archive or unarchive operation. Clients MUST treat the latest valid list as authoritative current state. Deltas are useful for live updates and audit history, but if a delta history and the latest list disagree, the latest valid list wins.

## Request Authorization Policy

A relay MAY accept or reject archive and unarchive requests according to local policy. This section defines a RECOMMENDED policy profile for interoperable implementations.

### Admin Requests

A relay MAY accept `kind:9035` and `kind:9036` requests from actors authorized under the relay's local admin policy. NIP-IA does not define how admin authority is assigned; this is implementation-defined.

Admin requests MAY target any pubkey. Accepted admin archive deltas MUST use `consent=admin` and identify the admin actor in the third element of the `consent` tag.

### Self Requests

A relay SHOULD accept `kind:9035` requests where `actor == target`. A user may retire their own pubkey.

A relay MUST accept a well-formed `kind:9036` request where `actor == target`, unless the target is currently banned or otherwise barred by access-control policy independent of NIP-IA. This self-unarchive path is the anti-shadowban property of this NIP: a normally authenticated user can ask to become visible again, and the relay's response is explicit. Relays that reject self-unarchive for a non-access-control reason SHOULD still emit or retain an auditable rejection reason via their normal `OK` response path.

Accepted self deltas MUST use `consent=self`. If a request has `actor == target` and also carries a valid `auth` tag, the relay MUST treat it as a self request and ignore the `auth` tag for consent-path selection.

### Owner-of-Agent Requests

A relay MAY accept requests where the actor is an owner key and the target is an agent key authorized by that owner under NIP-OA. This covers the common zombie-agent case: the human still controls the owner key, but the old agent key is gone or dormant and cannot sign a self-archive request.

There are two interchangeable ways to establish the owner-of-agent relationship. Both produce `consent=owner`. They differ only in where the relay obtains the NIP-OA proof:

- **request-borne**: the owner attaches a NIP-OA `auth` tag to the request itself, and
- **published profile attestation**: the relay reads a NIP-OA `auth` tag from the target's own latest `kind:0` profile.

A relay MAY support either or both. The published-profile-attestation path is RECOMMENDED for the zombie case, because it does not require the owner to retain any credential: the target's profile attestation remains retrievable from the relay after the agent key is gone.

#### Request-Borne Credential

To accept a request-borne owner-of-agent request, the relay MUST verify exactly one `auth` tag on the request using the NIP-OA cryptographic construction with the target as the authorized agent pubkey:

1. The `auth` tag MUST have exactly four elements.
2. The owner pubkey in the tag MUST equal the request actor (`event.pubkey`).
3. The target from the request's `p` tag MUST be the pubkey used in the NIP-OA preimage: `nostr:agent-auth:` || `<target-pubkey>` || `:` || `<conditions>`.
4. The Schnorr signature MUST verify under the owner pubkey.
5. The conditions string MUST be syntactically valid per NIP-OA.
6. Any `created_at<` and `created_at>` clauses MUST be evaluated against the request event's `created_at`.
7. `kind=` clauses, if present, are not meaningful for NIP-IA request authorization and MUST NOT be used to deny an otherwise valid owner-of-agent archive or unarchive request. Owners SHOULD issue NIP-IA-specific credentials with an empty conditions string or only time bounds to avoid ambiguity.

This mirrors NIP-AA's treatment of `kind=` at connection admission: the credential here is identity-binding evidence, not a per-event capability. It deliberately differs from event-level NIP-OA verification, where verifiers evaluate every clause against the event being verified and reject self-attestation by comparing the owner to `event.pubkey`. NIP-IA uses the NIP-OA signing preimage as ownership evidence for the target key; the request event itself is authored by the owner, so `auth` owner equals request `event.pubkey` is expected and valid in this specific verification context.

If accepted, relay deltas MUST use `consent=owner` and place the owner pubkey in the third element of the `consent` tag.

#### Published Profile Attestation

A relay MAY instead establish the owner-of-agent relationship from a NIP-OA `auth` tag the **target published in its own `kind:0` profile**. In this path the request itself carries no `auth` tag; the owner proves authority by being the owner pubkey named in the target's profile attestation and by signing the request with the owner key.

The proof event is the target's `kind:0` profile. Because `kind:0` is replaceable per NIP-01, the relay MUST use the **latest valid `kind:0` authored by the target that it knows at request time**. There is no fallback to an older profile version or to any non-profile event: a target revokes this path by republishing its profile without a valid `auth` tag, and the relay MUST honor that revocation.

To accept the request, the relay MUST verify the target's latest `kind:0` under the NIP-OA cryptographic construction:

1. The profile MUST be authored by the target (`profile.pubkey == target`) and MUST have a valid NIP-01 `id` and `sig`.
2. The profile MUST carry exactly one `auth` tag with exactly four elements. A profile carrying zero `auth` tags, more than one `auth` tag (per NIP-OA, multiple means no valid tag), or no valid `auth` tag fails this proof source; the relay MUST NOT fall back to an earlier profile.
3. The owner pubkey in the `auth` tag MUST equal the request actor (`request.pubkey`).
4. The Schnorr signature MUST verify under the owner pubkey over the NIP-OA preimage `nostr:agent-auth:` || `<target-pubkey>` || `:` || `<conditions>`, where `<conditions>` is the exact string from the profile's `auth` tag.
5. The conditions string MUST be syntactically valid per NIP-OA.
6. NIP-OA condition clauses MUST NOT be evaluated on this path. Like the request-borne path, this path reuses the NIP-OA signing preimage as identity-binding owner-of-target evidence in a NIP-IA-specific verification context; it is not full event-level NIP-OA provenance verification of the archive request. Because the proof is the target's latest profile used as a standing *ownership declaration*, this path additionally does not evaluate time clauses: `kind=`, `created_at<`, and `created_at>` describe the profile event, not the request, and MUST NOT deny an otherwise valid request. The request-borne path above still evaluates time clauses against the request's `created_at`.

If accepted, relay deltas MUST use `consent=owner` and place the owner pubkey in the third element of the `consent` tag. The delta SHOULD reference the profile event used as proof with a marked `e` tag, `["e", "<profile-event-id>", "", "proof", "<target-pubkey>"]`, kept distinct from the unmarked request `e` tag, so the ownership evidence remains independently auditable.

## Relay Processing Algorithm

When a relay receives a `kind:9035` or `kind:9036` request, it MUST execute the following checks before applying policy:

1. Verify the event id and signature per NIP-01.
2. Verify the event kind is `9035` or `9036`.
3. Require exactly one NIP-70 `-` tag.
4. Require exactly one valid `p` tag. The target MUST be 64-character lowercase hex. Relays MAY normalize uppercase hex to lowercase before processing, but emitted relay events SHOULD use lowercase hex.
5. If `replaced-by` is present, require a valid 64-character lowercase hex pubkey that differs from the target.
6. Enforce a relay-defined freshness window for request events. A ±120-second window is RECOMMENDED.
7. Determine the consent path under local policy. If no policy path accepts the request, reject.
8. Apply the state change idempotently. Archiving an already archived target and unarchiving a non-archived target SHOULD be treated as success, but relays MUST NOT emit a duplicate delta or new snapshot when no state changed.
9. If state changed, publish the corresponding `kind:8002` or `kind:8003` delta and a fresh `kind:13535` list.

When a relay rejects a request received via `EVENT`, it MUST respond with an `OK` message. Syntax and signature failures SHOULD use the `invalid:` prefix. Authorization failures SHOULD use the `restricted:` prefix. Relays MAY store rejected requests for audit, but rejected requests MUST NOT change archive state and MUST NOT produce `kind:8002`, `kind:8003`, or `kind:13535` updates.

## Client Behavior

Clients that support this NIP SHOULD query `kind:13535` from the relay identity advertised in NIP-11 `self` when connecting to a relay that advertises or is known to support NIP-IA.

Clients MUST verify that `kind:13535`, `kind:8002`, and `kind:8003` events are signed by the relay identity. Events signed by any other key MUST NOT affect archive state.

Clients SHOULD hide archived identities from active-member lists, mention autocomplete, invite dialogs, agent pickers, and similar forward-looking discovery surfaces scoped to that relay.

Clients MUST NOT hide or rewrite historical events solely because their author is archived. Historical messages, reactions, files, and audit events remain authored by the archived pubkey.

Clients SHOULD surface archive metadata where relevant. For example, a profile view for an archived identity may show "Archived on this relay" plus reason, replacement pubkey, and consent path from the latest applicable delta.

Clients MUST scope archive state to the relay that signed it. If a user participates on multiple relays, a pubkey archived on relay A is not archived on relay B unless relay B signs its own NIP-IA state.

Clients SHOULD process live `kind:8002` and `kind:8003` deltas for immediate UI updates, but SHOULD periodically or on reconnect reconcile against the latest `kind:13535` snapshot.

## Snapshot and Delta Consistency

The latest valid `kind:13535` snapshot is authoritative. Deltas are an append-only explanation stream. Clients SHOULD track the highest `created_at` they have accepted per relay identity for `kind:13535` and reject older snapshots from the same relay identity, even if they arrive later over a subscription. This provides rollback resistance against stale snapshot replay. Same-`created_at` resolution follows NIP-01: retain the event with the lowest id (first in lexical order).

A client reconstructing state from scratch SHOULD:

1. Fetch the latest valid `kind:13535` signed by the relay identity.
2. Initialize archive state from its `p` tags.
3. Subscribe to future `kind:8002`, `kind:8003`, and `kind:13535` events signed by the relay identity.
4. Apply deltas optimistically for live UI.
5. Replace local state whenever a newer valid `kind:13535` arrives.

If the relay cannot provide the originating request event referenced by a delta's `e` tag, clients MAY still trust the relay-signed delta for current relay state, but SHOULD treat the audit trail as incomplete.

### Snapshot Size

A single `kind:13535` snapshot can become large. Ten thousand archived pubkeys produce hundreds of kilobytes of `p` tags, exceeding common relay event-size limits. This NIP intentionally keeps the v1 snapshot shape aligned with NIP-43's single-list model, but large relays SHOULD define local caps and MAY need a future paginated or chunked snapshot extension. Relays that cannot publish a complete snapshot within their event-size limit MUST document that limitation; deltas alone are not sufficient for a fresh client to bootstrap complete current state.

## Test Vectors

These vectors are deterministic given the keys and timestamps below. Each event's NIP-01 `id` is `SHA256(id_preimage)` where `id_preimage` is the compact UTF-8 JSON serialization of `[0, pubkey, created_at, kind, tags, content]` with separators `,` and `:` and `ensure_ascii=False`. Each `sig` is a BIP-340 Schnorr signature over `id` produced with 32-byte zero `aux`. BIP-340 signatures are non-deterministic in `aux`; verifiers MUST accept any signature that is cryptographically valid under the signer pubkey, not only the values shown here.

```text
owner_secret = 0000000000000000000000000000000000000000000000000000000000000001
owner_pubkey = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
agent_secret = 0000000000000000000000000000000000000000000000000000000000000002
agent_pubkey = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
relay_secret = 0000000000000000000000000000000000000000000000000000000000000003
relay_pubkey = f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
```

The five vectors below form a single chain: an owner-of-agent archive request (9035) is processed into a delta (8002) and a snapshot (13535); then the agent self-unarchives (9036) and the relay emits a delta (8003). The 8002 and 8003 `e` references are the real `id`s of the 9035 and 9036 requests, not placeholders. Implementations can verify the full request → delta → snapshot pipeline against one fixture set.

### NIP-OA auth tag (reused from NIP-OA test vectors)

```text
conditions   = kind=1&created_at<1713957000
preimage     = nostr:agent-auth:c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5:kind=1&created_at<1713957000
sha256       = 08cdecd55af4c28d3801fd69615dcf5cc04fab3bc134b38a840bf157197069a6
owner_sig    = 8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369
```

### Vector 1 — `kind:9035` owner-of-agent archive request (owner-signed)

```text
kind         = 9035
pubkey       = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
created_at   = 1713956400
content      = "Archiving zombie agent after rebuild."
tags         = [["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["reason","bot-rebuilt"],["auth","79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798","kind=1&created_at<1713957000","8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369"]]
id_preimage  = [0,"79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",1713956400,9035,[["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["reason","bot-rebuilt"],["auth","79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798","kind=1&created_at<1713957000","8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369"]],"Archiving zombie agent after rebuild."]
id           = 3eb98c5200ee3b0280471131c0e63b5a3a3b6049a3c51ee4f425e649a45389d8
sig          = 28d567e61ecf34625b0fa204c7cc8a00fc11fd3cc21e1408d8493f38e37b08673322b44231b60c37750147ce4bc7589fc068201bdde3f5ada798ec6d2c9cd63b
```

### Vector 2 — `kind:8002` archived-identity delta (relay-signed, `consent=owner`)

```text
kind         = 8002
pubkey       = f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
created_at   = 1713956401
content      = "Archiving zombie agent after rebuild."
tags         = [["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["consent","owner","79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"],["e","3eb98c5200ee3b0280471131c0e63b5a3a3b6049a3c51ee4f425e649a45389d8"],["reason","bot-rebuilt"]]
id_preimage  = [0,"f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",1713956401,8002,[["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["consent","owner","79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"],["e","3eb98c5200ee3b0280471131c0e63b5a3a3b6049a3c51ee4f425e649a45389d8"],["reason","bot-rebuilt"]],"Archiving zombie agent after rebuild."]
id           = cf4f9376861f90af3edcfabc8f6363e5e0894f0f1234592663352ec8977c4d86
sig          = 109eebd8325285b46b18a0b457be038a360189ab70ff912c4fb0ab73a930c4e99e3bb161e12c4547d190b57a786e97e553f249ab19b24cb076d18361d01e2cf7
```

### Vector 3 — `kind:13535` archived identities list snapshot (relay-signed)

```text
kind         = 13535
pubkey       = f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
created_at   = 1713956402
content      = ""
tags         = [["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"]]
id_preimage  = [0,"f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",1713956402,13535,[["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"]],""]
id           = 263a4e89f569146af145adea1630194a1f35e1290ae08b776d51237012cba9a7
sig          = 0e68776627a39432891b75a13f146ba16e92e7864144cf983c01012ea04a4817ddecf57b5f96b10e9a64ba96f0abc544ff5074e360d3f99cf7692d2ac98338ec
```

### Vector 4 — `kind:9036` self-unarchive request (target signs for itself)

```text
kind         = 9036
pubkey       = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
created_at   = 1713956500
content      = "I am active again."
tags         = [["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["reason","returned"]]
id_preimage  = [0,"c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",1713956500,9036,[["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["reason","returned"]],"I am active again."]
id           = 7415e4d62fa388b791b8cf787f4e5631be45634681d3056da973e0091ed8c05f
sig          = 0c941d38a0cea6e8af3d500b3147e61d4f82ac40ce53cd43c2ba7f3b2f51c832bb8c4958f9a3caf673fef4c49d3782c34f83db236e1485c3aa25f159f342a33e
```

### Vector 5 — `kind:8003` unarchived-identity delta (relay-signed, `consent=self`)

```text
kind         = 8003
pubkey       = f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
created_at   = 1713956501
content      = "I am active again."
tags         = [["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["consent","self","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["e","7415e4d62fa388b791b8cf787f4e5631be45634681d3056da973e0091ed8c05f"],["reason","returned"]]
id_preimage  = [0,"f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",1713956501,8003,[["-"],["p","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["consent","self","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"],["e","7415e4d62fa388b791b8cf787f4e5631be45634681d3056da973e0091ed8c05f"],["reason","returned"]],"I am active again."]
id           = a261e4f574669b5097a3d4ac2b7e9ab3185639499206373e5a5420169b7201d2
sig          = e97904fd39387ab41ff650da344d83b61626a6eaa97cf415648525fff2ae54054339b697f62780b37c8ab7e80f44a169ed23b4b33899510a614f619289fc84ee
```

## Implementation Gotchas

Three places where independent re-derivations are most likely to diverge:

1. **NIP-01 event-id serialization** is `json.dumps([0, pubkey, created_at, kind, tags, content], separators=(",", ":"), ensure_ascii=False)` over UTF-8 bytes. The serialization is positional (array indices), not key-sorted. Tag arrays preserve the order chosen by the author; verifiers MUST hash the same bytes the author hashed, so canonicalizing tag order before computing `id` will produce a different `id` than the published `sig` was made over.

2. **BIP-340 Schnorr signatures are non-deterministic in `aux`.** The signatures in §Test Vectors were produced with 32-byte zero `aux`; a different `aux` (or a library that defaults to random `aux`) produces a different — equally valid — signature for the same `id`. Verifiers MUST verify a signature cryptographically; they MUST NOT compare a re-signed reproduction byte-for-byte against the published value.

3. **`auth` tag preimage is the NIP-OA preimage of the target, not of the request signer.** When verifying an owner-of-agent archive request (§Owner-of-Agent Requests), the `<event.pubkey>` slot in the NIP-OA preimage `nostr:agent-auth:<event.pubkey>:<conditions>` is the *target* (agent) pubkey, not the request signer (owner). Implementations that reuse a generic NIP-OA verifier MUST substitute the target before computing the preimage.

4. **Condition evaluation differs by proof source.** In the request-borne owner path the relay evaluates `created_at<`/`created_at>` clauses against the *request's* `created_at`. In the published-profile-attestation path the relay MUST NOT evaluate any clause: the profile `auth` tag is a standing ownership declaration, not authorization for the request, and an agent that only ever issued time-bounded credentials would otherwise be permanently un-archivable once those windows close — defeating the zombie case. A verifier that evaluates conditions on the profile path will silently reject valid requests.

## Security Considerations

**Relay authority is scoped**: A relay can honestly report its own archive state. It cannot make claims about other relays. Clients MUST NOT globalize archive state.

**Not a ban primitive**: Archival hides identities from active UI; it does not prevent connection, reading, writing, or event propagation. Relays that want to deny access MUST use NIP-43 or another access-control mechanism.

**Transparency and self-unarchive**: A relay that publishes archive state publicly cannot silently hide a pubkey without leaving a relay-signed artifact. The required self-unarchive path for non-banned users gives archived parties a protocol path to contest or reverse archival.

**Admin abuse remains possible**: A malicious or negligent relay admin can archive identities. NIP-IA does not prevent local policy abuse; it makes the action explicit, signed, and auditable.

**Request replay**: Relays SHOULD enforce request freshness and SHOULD include NIP-70 `-` tags on request and relay events. Freshness limits replay of old admin or owner requests. NIP-70 discourages third-party rebroadcast of administrative events.

**Owner-of-agent credential reuse**: NIP-OA `auth` tags are reusable capabilities. If an owner issues an unbounded credential for an agent, that credential can be reused by anyone who also controls the request-signing owner key. Since owner-of-agent NIP-IA requests are signed by the owner, compromise of the owner key is already sufficient to request archival. Owners SHOULD still bound NIP-OA credentials with `created_at<` where appropriate.

**Lost keys**: Self-unarchive requires the target key to sign. If the target key is lost, self-unarchive is impossible. Owner-of-agent unarchive MAY help for agents whose owner key remains available. Human key rotation should use `replaced-by` metadata so clients can guide users to the new identity.

**Ambiguous display names**: Clients MUST archive by pubkey, not by display name. A `replaced-by` tag is a hint, not proof that two keys belong to the same person unless independently verified.

## Privacy Considerations

Archive state is public to clients that can read the relay's NIP-IA events. This is intentional: NIP-IA is designed to avoid silent suppression.

A `replaced-by` tag links an old pubkey to a new pubkey. Relays SHOULD include it only when the actor requested it or local policy justifies the disclosure.

An owner-of-agent request discloses the owner-agent relationship through the NIP-OA `auth` tag and through `consent=owner` on the relay delta. This is necessary for auditability. Owners who do not want that relationship disclosed SHOULD not use the owner-of-agent request path.

Reason strings can reveal sensitive operational details. Relays SHOULD prefer short reason codes and avoid embedding private human-readable explanations in public events unless the actor explicitly provided them for that purpose.

## Examples

### Self-archive after key rotation

Alice rotates from `alice_old` to `alice_new`. She signs:

```jsonc
{
  "kind": 9035,
  "pubkey": "<alice_old>",
  "content": "Rotated to my new key.",
  "tags": [
    ["-"],
    ["p", "<alice_old>"],
    ["reason", "rotated"],
    ["replaced-by", "<alice_new>"]
  ]
}
```

The relay verifies `actor == target`, archives `alice_old`, emits:

```jsonc
{
  "kind": 8002,
  "pubkey": "<relay>",
  "content": "Rotated to my new key.",
  "tags": [
    ["-"],
    ["p", "<alice_old>"],
    ["consent", "self", "<alice_old>"],
    ["e", "<request-id>"],
    ["reason", "rotated"],
    ["replaced-by", "<alice_new>"]
  ]
}
```

and republishes `kind:13535` with `alice_old` included.

### Owner archives a zombie agent

An owner controls `owner_pubkey`. A previous agent key `agent_old` is no longer usable. The owner signs `kind:9035` with `pubkey = owner_pubkey`, target `agent_old`, and a NIP-OA `auth` tag proving `owner_pubkey` authorized `agent_old`.

The relay verifies the owner signature on the request, verifies the NIP-OA `auth` tag using `agent_old` in the preimage, accepts the request, and emits a `kind:8002` delta with:

```jsonc
[
  ["p", "<agent_old>"],
  ["consent", "owner", "<owner_pubkey>"],
  ["e", "<request-id>"],
  ["reason", "bot-rebuilt"]
]
```

The old agent disappears from active agent pickers on that relay. Its historical messages remain visible and authored by `agent_old`.

If the owner no longer holds a saved `auth` credential, they can use the published-profile-attestation path instead: `agent_old` published its NIP-OA `auth` tag (naming `owner_pubkey`) in its `kind:0` profile while it was alive, so that profile remains on the relay. The owner signs the same `kind:9035` with no `auth` tag, and the relay verifies `owner_pubkey` against the target's latest `kind:0`. The owner needs only their own key.

### Admin archive plus NIP-43 ban

A spammer should be hidden and barred from reconnecting. A relay admin removes the spammer via NIP-43 member removal (`kind:8001`) and archives the same pubkey with NIP-IA (`kind:9035`).

Clients hide the spammer because of NIP-IA. The relay denies access because of NIP-43. These are separate state transitions and remain separately auditable.

### Self-unarchive

A non-banned user decides they should be visible again. They sign:

```jsonc
{
  "kind": 9036,
  "pubkey": "<target>",
  "content": "I am active again.",
  "tags": [["-"], ["p", "<target>"], ["reason", "returned"]]
}
```

The relay verifies `actor == target`, removes the target from archive state, emits `kind:8003` with `consent=self`, and republishes `kind:13535` without the target.

## Invalid Cases

Relays MUST reject each of the following requests:

| Scenario | Reason |
|----------|--------|
| Missing `p` tag | no target |
| Multiple `p` tags | ambiguous target |
| Missing NIP-70 `-` tag | unprotected administrative request |
| Invalid event signature | not a valid actor request |
| `replaced-by` equals target | nonsensical replacement |
| Non-admin actor archives someone else without valid NIP-OA owner proof | unauthorized |
| Owner-of-agent request where `auth` owner does not equal actor | unauthorized |
| Owner-of-agent request where NIP-OA signature was made for a different agent pubkey | unauthorized |
| Profile-attestation request where the target's latest `kind:0` carries no valid `auth` tag | revoked or never declared; no fallback to older profiles |
| Profile-attestation request where the owner in the latest `kind:0` `auth` tag does not equal actor | unauthorized |
| Self-unarchive from a pubkey currently banned by access-control policy | access-control policy wins |
| Request outside relay freshness window | replay risk |

Clients MUST ignore each of the following relay events for archive-state purposes:

| Scenario | Reason |
|----------|--------|
| `kind:8002`, `kind:8003`, or `kind:13535` not signed by relay NIP-11 `self` key | not relay state |
| Relay event missing NIP-70 `-` tag | malformed protected event |
| Delta missing `p` tag | no target |
| Delta missing `consent` tag | unauditable decision |
| Snapshot `p` tag with invalid pubkey | invalid entry; clients SHOULD ignore that entry |

## Relation to Other NIPs

**NIP-01**: All NIP-IA events are ordinary Nostr events and must pass standard id/signature validation.

**NIP-11**: The relay identity is discovered through NIP-11 `self`. Clients use that key to verify relay-signed archive state.

**NIP-42**: Relays commonly require NIP-42 authentication before accepting `kind:9035` or `kind:9036` requests. This NIP does not change NIP-42.

**NIP-43**: NIP-IA composes with NIP-43. NIP-43 controls relay access and membership; NIP-IA controls relay-scoped visibility of retired identities. A pubkey may be archived but still a member, removed but not archived, both removed and archived, or neither.

**NIP-70**: NIP-IA requests, deltas, and snapshots use the NIP-70 `-` tag to mark events as protected administrative state that should not be casually rebroadcast by third parties.

**NIP-OA**: NIP-IA reuses NIP-OA owner attestations for owner-of-agent archive and unarchive requests. The request remains authored by the request signer. The NIP-OA tag is authorization evidence only.
