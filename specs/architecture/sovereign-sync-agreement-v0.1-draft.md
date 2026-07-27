# Sovereign Sync Agreement v0.1 (draft)

Status: draft for discussion; not yet implemented, not yet upstreamed. Kind
numbers are provisional pending assignment in the shared registry
(`buzz-core/src/kind.rs`). This draft is descriptive: every clause family
below already runs as operator configuration in the portable relay adapters;
the draft's contribution is turning that configuration into signed,
journal-resident, mutually attributable intent.

## Decision

A **sync agreement** is not a new protocol object. It is a matched pair of
signed **sync declarations** — one published by each sovereign party — whose
contents are compatible and which pin one another. Replication between the
parties is already governed by the portable replication and identity
profiles; the agreement layer makes the *governance itself* durable,
attributable, and observable.

This replaces nothing at the transport boundary. Peer evidence (NIP-98/NIP-42)
still authenticates every session; destination policy still admits every
record; declarations are names and intent, never credentials.

## Why a matched pair, not a countersigned document

A Nostr event carries one signature. Rather than inventing a
countersignature envelope, each party signs its own half:

- the **source** declares what it exports and to whom;
- the **destination** declares what it admits and from whom.

An agreement **exists** exactly when the two declarations match under the
deterministic rule in *Matching*. Three properties fall out:

1. **Tension stays visible.** An unmatched declaration is an observable
   proposal or an observable drift, not a hidden config difference. When one
   party revises its half, the match breaks until the other party re-pins —
   the disagreement is in both journals, timestamped and signed.
2. **Both journals hold the whole agreement.** Declarations are ordinary
   events; they replicate over the same streams they govern. After one sync,
   each party's journal contains both halves.
3. **No coordination server.** Matching is a pure function over two events
   any observer can evaluate.

## Kind

One provisional kind, `3070x` (**TBD**), parameterized replaceable. All
declaration roles share it; the `d` tag disambiguates:

```
d = "<role>/<stream-id>"        role ∈ { export, admit, read, key-grant }
```

Addressable semantics give each `(party, role, stream)` cell last-write-wins
with the NIP-01 tie-break, and the monotonic `created_at` discipline from
NIP-AE §Writing applies. Revocation is a replacement whose body carries
`"status": "revoked"` — history of the grant remains in the journal; only
the head changes.

## Declaration roles

Bodies are JSON in `content` (plaintext — declarations govern streams, and
their observability is the point; anything private belongs in what the
streams carry, not in the governance). Unknown fields MUST be ignored.

### `export` — source-side

Declares one exported stream. The selection is part of the stream's
identity: changing it requires a new stream ID (established invariant from
the selective-streams work).

```jsonc
{
  "kind": 3070x,
  "tags": [
    ["d", "export/rendezvous/ted-sovereign"],
    ["p", "<reader-or-destination-pubkey>"]      // zero or more offered parties
  ],
  "content": {
    "status": "active",
    "selection": { "from_source": "ted-laptop/sovereign" },
      // exactly one of: {"mirror": true} | {"filter": [ ... ]} |
      //                 {"from_source": "<id>"}
    "cursor_space": "cf-sqlite-v1:",             // informative
    "artifacts": "referenced"                     // none | referenced (see below)
  }
}
```

### `admit` — destination-side

Declares the destination-controlled binding for one stream: which transport
principal, under which verification keys, pinned to which export.

```jsonc
{
  "tags": [
    ["d", "admit/rendezvous/ted-sovereign"],
    ["p", "<transport-principal-pubkey>"],       // one or more active keys
    ["e", "<export-declaration-event-id>"]       // the pin (see Matching)
  ],
  "content": {
    "status": "active",
    "principal": "did:buzz:node-b-puller",       // stable label; keys rotate
    "retention": { "keep": "journal" }           // journal | effective
  }
}
```

### `read` — source-side reader grant

Authorizes a principal to drain an exported stream (the rendezvous role).
Same shape as `admit` seen from the other side: `d = "read/<stream-id>"`,
`p` = reader pubkey(s), `e` pin to the export declaration.

### `key-grant` — compartment capability record

Records that a compartment's read capability was conveyed to a grantee. The
event contains **no key material** — the key moves out of band; this is the
attributable record that it moved, and the anchor for rotation discipline.

```jsonc
{
  "tags": [
    ["d", "key-grant/ctx-buzz"],
    ["p", "<grantee-pubkey>"]
  ],
  "content": {
    "status": "active",
    "compartment": "<compartment-pubkey>",       // public label, not the path
    "granted_at": 1785090000,
    "rotation": "on-revoke"
      // encryption grants cannot be retracted; revocation of this
      // declaration obligates the grantor to rotate the compartment
      // (new derived key, new #p label) for future content
  }
}
```

## Matching

An agreement over stream `S` between source `A` and destination `B` exists
iff all of the following hold at the current heads:

1. `A` has an active `export/S` declaration whose `p` tags include `B` (or,
   for rendezvous reads, an active `read/S` naming `B`).
2. `B` has an active `admit/S` declaration whose `p` tags name the transport
   principal actually presenting evidence, and whose `e` tag equals the
   event ID of `A`'s current `export/S` head.
3. The declarations agree on the stream ID byte-for-byte.

Rule (2)'s pin is the drift detector: if `A` replaces its export declaration
(new selection ⇒ new stream ID; new readers or metadata ⇒ same `d`, new
event ID), `B`'s pin goes stale and the match breaks **visibly** until `B`
re-pins. Tooling SHOULD surface unmatched-declaration state; runtimes MAY
refuse to serve or ingest unmatched streams (strictness is adapter policy in
v0.1, normative in a later revision once operational experience accumulates).

## Artifacts

`"artifacts": "referenced"` in an export declares that events on the stream
may reference content-addressed blobs (`x` tags, pack digests) and that the
source (or the rendezvous custodian) serves them from its artifact store to
the same principals authorized for the stream. The event stream is the
manifest: a destination discovers missing blobs by walking references in
records it has already verified, fetching by hash, and verifying content —
possession is idempotent, so blob sync inherits the stream's
interruption-safety without its own cursor.

Git repositories are the special case that needs no special case: packs and
manifests are artifacts; ref state is NIP-34 `kind:30618`; the agreement
governs the stream those events travel on.

## Mapping from current operator configuration

| Today's config | Becomes |
| --- | --- |
| `streams.json` entry (laptop) / `BUZZ_REPLICATION_STREAMS` (Cloudflare) | `export/<stream>` declaration |
| `peer-trust.json` entry / `BUZZ_REPLICATION_PEERS` | `admit/<stream>` declaration |
| `BUZZ_REPLICATION_READERS` entry | `read/<stream>` declaration |
| hand-delivered compartment key | `key-grant/<label>` declaration |

Migration is mechanical: derive the declaration from the config entry, sign,
publish, and (optionally) regenerate config *from* the declaration heads —
at which point the journals are the source of truth and the files are a
cache. The four-places-edited-by-hand drift observed in practice is the
problem this ordering removes.

## Runtime evaluation (v0.1 adapter policy)

Adapters derive operating configuration from declaration heads at defined
evaluation points — the laptop adapter at process start, the Cloudflare
adapter at each replication request. Three rules:

1. **Owner anchor, node scope.** Only declaration heads authored by the
   node's owner pubkey AND carrying an `n` tag equal to the node's own label
   govern that node's configuration. Both anchors are bootstrap data (laptop
   `--owner`/`--node-label`, Cloudflare `BUZZ_OWNER_PUBKEY`/`BUZZ_NODE_LABEL`);
   they are identity, not policy, and stable across deploys. The `n` tag is
   what keeps a replicated journal safe to evaluate everywhere: one owner's
   declarations for different nodes coexist in every copy of the journal,
   and each node evaluates only its own. A head without an `n` tag governs
   no node's configuration (it can still be a relationship half). Foreign
   declarations remain relationship halves — they confer nothing without a
   matching owner half (invariant 5).
2. **Per-domain precedence, wholesale.** The domains are `admit/*` (sink
   peer trust), `export/*` (stream exports), and `read/*` (reader grants),
   each scoped to this node's label. If the journal holds *any* owner-signed
   head in a domain for this node — whatever its status — the journal
   governs that domain entirely and file/env config
   for the domain is ignored; only `status: "active"` heads confer trust.
   File/env is consulted solely when the journal holds no head in the
   domain (bootstrap). Revocation is therefore irreversible by fallback: a
   domain whose every head is revoked is an empty domain, not a reversion
   to files.
3. **Fail closed.** No owner anchor means no journal-derived configuration.
   No heads and no bootstrap config means empty trust.

## Security invariants

1. Declarations are intent, not credentials. Transport evidence and
   destination verification are unchanged and remain mandatory.
2. A `key-grant` event never contains key material, path names, or slugs —
   only the compartment's public label and the grantee.
3. Revoking `admit` or `read` takes effect at the next evaluation; revoking
   `key-grant` obligates compartment rotation (confidentiality of already-
   conveyed content is not retroactively recoverable, and the spec does not
   pretend otherwise).
4. Matching is evaluated over declaration *heads*; superseded declarations
   remain in both journals as history.
5. A declaration naming a counterparty confers nothing on that counterparty
   without the counterparty's own matching half.

## Explicitly outside v0.1

- Kind number assignment (upstream registry decision).
- Relay-side enforcement of matching (adapter policy for now).
- Multi-party (>2) agreements and delegation chains.
- Negotiation protocol (offers are just unmatched declarations).
- Retention auditing and proof-of-custody.
- NIP-77-style set reconciliation (orthogonal efficiency upgrade).

## Traceability

- Telos: [`../TELOS.md`](../TELOS.md)
- Parent boundary:
  [`portable-relay-boundary.md`](portable-relay-boundary.md)
- Replication semantics: the replication profile and
  [selective streams](portable-relay-boundary.md) invariants
  (predicate-is-identity, source-owned cursors, checkpoint-safe receipts)
- Identity semantics:
  [`portable-relay-identity-v0.1.md`](portable-relay-identity-v0.1.md)
- Prior art: NIP-AE (addressable heads, monotonic writes), NIP-OA
  (capability tags), NIP-34 (`kind:30618` ref state), Blossom
  (content-addressed blobs), upstream git CAS
  (`crates/buzz-relay/src/api/git/`)
