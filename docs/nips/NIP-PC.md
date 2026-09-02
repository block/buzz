NIP-PC
======

Relay-Authoritative Project State
---------------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format and addressable events), NIP-09 (event deletion), and [NIP-MP](NIP-MP.md) (owner-signed Project identity)

## Abstract

This NIP defines a relay-authoritative read model for NIP-MP Projects. The owner-signed `kind:30621` remains the portable Project identity. A relay materializes accepted identity replacements and deletions into relational state, then publishes that state as a relay-signed addressable `kind:30623` Project State event.

The projection gives clients one signed event to read, while the relational row remains authoritative if publication or fan-out fails. This version does not widen the owner-only replacement authority of `kind:30621`.

## Project Coordinate

Project state is keyed by the canonical NIP-01 coordinate:

```text
30621:<owner-pubkey-hex>:<project-d>
```

The kind segment is the literal `30621`, the owner is exactly 64 lowercase hexadecimal characters, and the Project `d` value is non-empty and preserved verbatim. Parsing splits on the first two colons so a `d` value containing a colon remains addressable.

The coordinate selects state only inside the host-bound community. A relay MUST NOT derive the community from a client-supplied Project tag.

## Authoritative State

The authoritative row is keyed by `(community, Project owner, Project d)` and carries a monotonic signed-64-bit revision, deletion state, the current owner identity event id, the last lifecycle event id, and the effective Project document.

The first accepted owner-signed `kind:30621` materializes revision `1`. Each accepted newer owner identity, deletion, or recreation increments the revision. Revisions never reset, including after deletion and recreation. A duplicate or superseded event does not advance the revision, and overflow rejects the lifecycle mutation.

An accepted newer owner-signed `kind:30621` is a full recovery snapshot. It replaces the effective NIP-MP fields and extension tags with those carried by that owner event. An accepted owner-authorized NIP-09 deletion advances the row to a deleted tombstone. It does not delete member repositories or referenced channels. A later valid owner-signed `kind:30621` recreates the Project at the next revision.

Identity replacement or deletion and its relational lifecycle update MUST commit atomically.

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
    ["e", "<last-lifecycle-event-id>", "", "change"]
  ],
  "content": "{\"v\":1,\"deleted\":false,\"project_tags\":[[\"d\",\"<project-d>\"]]}"
}
```

The address `d` is the lowercase SHA-256 hex digest of the UTF-8 Project coordinate. Hashing keeps the projection key fixed at 64 bytes even when the Project's own `d` approaches its 1024-byte bound. The `a` tag carries the unhashed coordinate. The `rev` tag is a canonical base-10 integer in `1..=9223372036854775807`: digits only, no sign, and no leading zero.

The `identity` event id names the current owner-signed `kind:30621`. The `change` marker names the identity or deletion event that produced the revision. On initial materialization and owner recovery, both references may name the same identity event.

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
