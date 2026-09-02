NIP-PC
======

Collaborative Project State
---------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format and addressable events), NIP-09 (event deletion), NIP-29 (home-channel roles), and [NIP-MP](NIP-MP.md) (owner-signed Project identity)

## Abstract

This NIP defines collaborative related-channel membership and a relay-authoritative read model for NIP-MP Projects. The owner-signed `kind:30621` remains the portable Project identity. An authorized actor submits a transactional `kind:47010` change, and the relay publishes effective state as a relay-signed addressable `kind:30623` Project State event.

The projection gives clients one signed event to read, while the relational row remains authoritative if publication or fan-out fails. This version does not widen the owner-only replacement authority of `kind:30621`; collaborators mutate only related-channel membership through `kind:47010`.

## Project Change Command

`kind:47010` is a global command requiring `repos:write` and a global authentication token:

```jsonc
{
  "kind": 47010,
  "pubkey": "<actor-pubkey-hex>",
  "tags": [
    ["a", "30621:<owner-pubkey-hex>:<project-d>"],
    ["expected-revision", "7"]
  ],
  "content": "{\"v\":1,\"patch\":{\"related_channels\":{\"add\":[\"11111111-1111-4111-8111-111111111111\"],\"remove\":[]}}}"
}
```

The command contains exactly those two tags. `expected-revision` is a canonical integer in `1..=9223372036854775807`. The version-1 JSON shape is strict: unknown fields are rejected; `add` and `remove` are required, non-overlapping sets of canonical UUIDs; both may contain at most 64 entries; and the patch must not be empty. Adding the home channel or an already-related channel, removing an absent channel, or producing more than 64 related channels is invalid.

The Project owner is always authorized. A different signer is authorized only while they are an active `owner` or `admin` of the Project's current, active home channel. Removed members and archived or deleted channels grant no authority. Projects without a home channel therefore remain owner-managed. Delegation and channel `created_by` do not confer Project authority.

The relay serializes each command by Project coordinate, checks the expected revision, and atomically stores the immutable command event, applies the complete patch, and advances the revision. Two distinct commands against one revision cannot both commit.

Replaying the exact accepted event is idempotent, including when its event row has been soft-deleted. This guarantee is bounded by the relay's ordinary event timestamp-drift window: after that window a client re-reads `kind:30623` and signs a fresh command against the current revision.

## Project Coordinate

Project state is keyed by the canonical NIP-01 coordinate:

```text
30621:<owner-pubkey-hex>:<project-d>
```

The kind segment is the literal `30621`, the owner is exactly 64 lowercase hexadecimal characters, and the Project `d` value is non-empty and preserved verbatim. Parsing splits on the first two colons so a `d` value containing a colon remains addressable.

The coordinate selects state only inside the host-bound community. A relay MUST NOT derive the community from a client-supplied Project tag.

## Authoritative State

The authoritative row is keyed by `(community, Project owner, Project d)` and carries a monotonic signed-64-bit revision, deletion state, the current owner identity event id, the last change or lifecycle event id, and the effective Project document.

The first accepted owner-signed `kind:30621` materializes revision `1`. Each accepted Project change, newer owner identity, deletion, or recreation increments the revision. Revisions never reset, including after deletion and recreation. A duplicate or superseded event does not advance the revision, and overflow rejects the mutation.

An accepted newer owner-signed `kind:30621` replaces owner-controlled NIP-MP fields and extension tags while preserving collaboratively added related channels. Related channels present in the new identity are unioned with relational membership; the new home channel is excluded from that union, and a result over 64 channels is rejected. An accepted owner-authorized NIP-09 deletion advances the row to a deleted tombstone. It does not delete member repositories or referenced channels. A later valid owner-signed `kind:30621` recreates the Project at the next revision.

Commands, identity replacement, and deletion each commit atomically with their relational update.

## Project State Event

`kind:30623` is relay-only and addressable:

```jsonc
{
  "kind": 30623,
  "pubkey": "<relay-identity-pubkey-hex>",
  "tags": [
    ["d", "<sha256-project-coordinate-hex>"],
    ["a", "30621:<owner-pubkey-hex>:<project-d>"],
    ["rev", "8"],
    ["e", "<current-kind-30621-event-id>", "", "identity"],
    ["e", "<last-change-or-lifecycle-event-id>", "", "change"]
  ],
  "content": "{\"v\":1,\"deleted\":false,\"project_tags\":[[\"d\",\"<project-d>\"]]}"
}
```

The address `d` is the lowercase SHA-256 hex digest of the UTF-8 Project coordinate. Hashing keeps the projection key fixed at 64 bytes even when the Project's own `d` approaches its 1024-byte bound. The `a` tag carries the unhashed coordinate. The `rev` tag is a canonical base-10 integer in `1..=9223372036854775807`: digits only, no sign, and no leading zero.

The `identity` event id names the current owner-signed `kind:30621`. The `change` marker names the command, identity, or deletion event that produced the revision. On initial materialization and owner recovery, both references may name the same identity event.

For a live Project, `project_tags` is the complete effective NIP-MP-compatible tag set. It includes exactly one Project-slug `d` tag. Known set-valued fields are emitted in deterministic lexical order, while preserved unknown extension tags retain their byte values and relative order. Transport-only `auth` tags are not Project metadata.

A deleted Project is projected as:

```json
{"v":1,"deleted":true,"project_tags":[]}
```

Version 1 decoders MUST reject unknown JSON fields. A client reading one requested coordinate MUST verify the event signature, require the relay identity advertised by its trusted relay connection, require `kind:30623`, require exactly one matching `a` tag and one canonical `rev` tag, and require content version `1`. Clients do not need to reproduce the relay's effective-tag canonicalization algorithm before displaying a valid projection.

Client submission of `kind:30623` MUST be rejected, including an event whose signer happens to equal the configured relay pubkey but which arrived through ordinary client ingest.

## Publication and Reconciliation

The relational row remains authoritative if projection publication or fan-out fails. The relay MUST retain a durable retry or reconciliation path and MUST NOT report a committed lifecycle event as rolled back merely because derived publication failed. Reconciliation republishes the current row without advancing its revision.

Every newly signed projection for one address MUST have a `created_at` strictly greater than the previous accepted projection at that address, including repair of the same revision. Allocation uses `max(current Unix time, previous projection created_at + 1)`. If no greater timestamp can be represented, publication fails without changing authoritative state.

Clients use `rev` as the state revision. Repair or relay-key rotation may produce a new projection event id without changing the revision, so the projection event id is not a revision token.

## Schema Evolution

Projection content carries an explicit version. A client that does not understand a Project State version MUST ignore that projection rather than guess at its state. New fields require a new understood version; version-1 decoders reject them through strict JSON deserialization.

## Security Considerations

Structural parsing alone does not establish trust. Clients MUST bind the projection signer to the relay identity advertised through their trusted relay connection and MUST match the unhashed `a` tag to the Project coordinate they requested.
