NIP-SW
======

Owner-Scoped Channel-Section Workspaces
---------------------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (event format and addressable events), NIP-42 (authenticated relay connections), NIP-44 (pairwise key wrapping)

## Abstract

NIP-SW defines one relay-authoritative channel-section workspace per `(community, owner pubkey)`. Owners grant exact pubkeys `viewer`, `mover`, or `manager` access. Actors sign typed commands as themselves; the relay authorizes and applies commands transactionally to normalized state. The relay publishes a revisioned projection only to the owner and active grantees.

The relay sees structural section UUIDs, ordering, and channel assignments. Section names and icons are ciphertext under a random per-workspace content key. That key is wrapped separately to each authorized reader using NIP-44; it grants confidentiality access, never mutation authority.

## Kinds

| Kind | Name | Signer | Storage | Stage |
|------|------|--------|---------|-------|
| `9050` | Import v1 | owner | command, not ordinary event storage | 1 |
| `9051` | Grant | owner | command, not ordinary event storage | 1 |
| `9052` | Revoke + rotate | owner | command, not ordinary event storage | 1 |
| `9053` | Move channel | owner, mover, manager | command, not ordinary event storage | 2 |
| `9054` | Manage sections | owner, manager | command, not ordinary event storage | 3 |
| `30623` | Workspace projection | relay | addressable/current projection | 1 |

The allocation was checked against this repository and the upstream `nostr-protocol/nips` event-kind table at commit `656cecc7c0a815b6a2b218d3b5d6f078b3f4dbab`: `9050`–`9054` and `30623` were unassigned upstream and unused in Buzz. These are Buzz-specific kinds; a future upstream collision requires a protocol revision.

## Coordinate and authority

The server-resolved community and command's owner pubkey identify a workspace. A client never supplies a community UUID. The relay derives community from the authenticated connection's host.

- Owner authority is implicit and immutable.
- A `viewer` may read the projection.
- A `mover` may read and submit kind 9053.
- A `manager` may read and submit kinds 9053 and 9054.
- Only the owner may submit kinds 9050, 9051, and 9052.

Every command carries a UUID `action_id`. The durable action row binds `(community, owner, action_id)` to a canonical command SHA-256. An exact retry returns its original revision. Reusing an action ID for different bytes is rejected. The relay durably stores the complete verified signed event JSON beside its event ID and command hash; this is the rebuild source, while normalized rows are the serving projection.

## Stage 1 command bodies

Command content uses the `nip-sw-canonical-json-v1` profile: recursively sort object keys by Unicode code-point order, preserve array order, emit UTF-8 with JSON escaping and no insignificant whitespace, and permit only integer JSON numbers. Strict typed decoding rejects duplicate and unknown keys before canonicalization. SHA-256 is computed over these exact UTF-8 bytes. Legacy plaintext uses the same profile before `source_hash` is computed. `NIP-SW.fixtures.json` contains byte-exact plaintext and command hash vectors. Tags provide routing and coarse filtering, but content defines the command and is validated before mutation. Every command has exactly one `p` tag naming the owner and exactly one `action` tag matching content's `action_id`.

### Revision-zero import (`kind:9050`)

```jsonc
{
  "kind": 9050,
  "tags": [
    ["p", "<owner-pubkey>"],
    ["action", "<uuid>"]
  ],
  "content": "{\"version\":1,\"action_id\":\"<uuid>\",\"source_event_id\":\"<64-lower-hex>\",\"source_hash\":\"<sha256-lower-hex>\",\"key_epoch\":1,\"owner_key_envelope\":\"<nip44>\",\"sections\":[...],\"assignments\":[...]}"
}
```

Import requirements:

- signer equals owner;
- workspace revision is zero and has no migration marker;
- `key_epoch` is 1;
- section IDs are non-nil and unique;
- ranks are exactly the permutation `0..section_count`;
- channels are unique, exist in the same community, and are not deleted;
- assignment destinations name imported sections;
- maximum 100 sections, 1,000 assignments, and 256 active delegate grants;
- repeated identical `action_id` + command hash is idempotent; any different import after migration is rejected.

The relay atomically writes normalized sections, assignments, the owner's envelope, revision 1, layout revision 1, and a migration marker containing the source event ID and canonical plaintext hash.

### Grant (`kind:9051`)

```jsonc
{
  "kind": 9051,
  "tags": [["p", "<owner>"], ["actor", "<delegate>"], ["action", "<uuid>"]],
  "content": "{\"version\":1,\"action_id\":\"<uuid>\",\"role\":\"viewer\",\"key_epoch\":<current>,\"key_envelope\":\"<nip44>\"}"
}
```

The content also repeats `actor_pubkey` and it MUST equal the single `actor` tag. Stage 1 clients expose `viewer`, but the durable role vocabulary is frozen as `viewer|mover|manager` for later stages. The signer MUST equal owner. Grant and current-epoch envelope installation are one transaction.

### Revoke and rotate (`kind:9052`)

```jsonc
{
  "kind": 9052,
  "tags": [["p", "<owner>"], ["actor", "<revoked-reader>"], ["action", "<uuid>"]],
  "content": "{\"version\":1,\"action_id\":\"<uuid>\",\"new_key_epoch\":<current+1>,\"sections\":[<all-active-sections-reencrypted>],\"envelopes\":[<owner-and-every-remaining-reader>]}"
}
```

The content also repeats `actor_pubkey` and it MUST equal the single `actor` tag. Revocation is atomic: mark the exact grant revoked, advance the key epoch by one, replace every active section's encrypted label/icon, and install exactly one envelope for the owner and each remaining active grantee. The revoked reader must not receive a new envelope. Revocation prevents future reads and commands; it cannot erase plaintext or keys a reader already retained.

## Metadata encryption

The workspace content key is exactly 32 random bytes. Labels and icons use AES-256-GCM with a fresh random 12-byte nonce for every encryption. The wire envelope is `aes256gcm:<base64url-no-pad nonce>:<base64url-no-pad ciphertext||16-byte-tag>`. Plaintext is UTF-8.

AAD is the canonical JSON UTF-8 encoding of `{"version":1,"community":"<canonical relay authority>","owner_pubkey":"<64-lower-hex>","section_id":"<lowercase UUID>","key_epoch":<integer>,"purpose":"label|icon"}`. Clients MUST reject authentication failure and MUST NOT retry with weaker or omitted AAD. Binding community, owner, section, epoch, and purpose prevents cross-tenant, cross-owner, cross-section, stale-epoch, and label/icon substitution. `NIP-SW.fixtures.json` contains a deterministic AES-GCM vector and enumerates every AAD mutation that must fail.

NIP-44 is used only to pairwise-wrap the 32-byte workspace key; its standard versioned encoding remains unchanged.

## Projection (`kind:30623`)

The relay signs a current projection after each accepted command. It is addressable by `d=<owner-pubkey>` and carries one `p` tag per currently authorized reader, including the owner. Its JSON content follows `NIP-SW.fixtures.json`.

Projection fields are `version`, `owner_pubkey`, monotonic `revision`, `layout_revision`, `key_epoch`, migration marker, the requesting reader's pairwise key envelope, ordered encrypted sections, and current assignments. Because envelopes differ per reader, the relay synthesizes a reader-specific response rather than exposing one shared stored event body.

Read authorization is applied at every delivery surface: historical REQ, live subscription/fan-out, HTTP query, and search. A filter-level gate alone is insufficient because an attacker may query a known event ID. Revocation evicts affected live subscriptions immediately.

Clients persist only verified projections. A projection at or below the cached revision is ignored. A gap greater than one causes a full authorized refetch. During relay outage, the last verified cache may render but is stale and cannot become a replacement write.

## Migration and cutover

An updated owner client decrypts the newest valid kind-30078 `d=channel-sections` event, computes the canonical plaintext hash, creates the workspace content key and owner envelope, then submits kind 9050. It switches authority only after reading back the matching migration marker and projection. Once the marker exists, updated clients MUST NOT publish the legacy section blob again. The old event remains a read-only rollback artifact during the compatibility window.

Delegated moves remain feature-gated until supported desktop and mobile clients both honor this marker. Dual-writing is forbidden: an old whole blob cannot safely represent later normalized commands.

## Shared fixtures

`NIP-SW.fixtures.json` is the cross-platform oracle for kinds, roles, limits, projection shape, revision-gap behavior, and the role matrix. Rust, desktop, and mobile tests consume this same file. A protocol change is incomplete until the fixture and all consumers change together.

## Security and privacy

- The owner private key is never shared. Delegates sign as themselves.
- NIP-44 envelopes grant decryption only; relay authorization grants mutation.
- The relay learns delegation, section UUID/order, and assignments as an accepted product tradeoff, but not labels/icons.
- Authorization is checked at command ingest time, so a command created offline by a subsequently revoked actor fails.
- Community is server-resolved and included in every database key. Same owner/section/channel UUIDs in another community are unrelated.
- Signed command events and durable action hashes provide audit/rebuild evidence; wall-clock time never resolves semantic conflicts.
