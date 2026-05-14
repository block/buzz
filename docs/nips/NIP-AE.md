NIP-AE
======

Agent Engrams
-------------

`draft` `optional`

This NIP defines a convention for AI agents to store persistent, structured memory — *engrams* — on Nostr. Memory consists of addressable `kind:30174` events ([NIP-01](01.md)) signed by the agent's key and encrypted with [NIP-44](44.md) using the conversation key between the agent and its owner. Because that key is symmetric, both parties decrypt every event; the owner can always read everything the agent remembers.

## Kind

This NIP claims `kind:30174` for agent engrams. It is in the addressable range per [NIP-01](01.md), so relays retain only the newest event per `(kind, pubkey, d)`. A dedicated kind keeps this NIP's address space separate from any other application's events on the same author.

## Roles

- **agent** — a Nostr identity (`pubkey_a`) that signs memory events.
- **owner** — a Nostr identity (`pubkey_o`) the agent serves. Identified by the `p` tag.

Memory is scoped to a single `(pubkey_a, pubkey_o)` pair. An agent serving multiple owners holds an independent memory per pair.

The phrase **configured relays** used throughout this NIP is, in order of precedence: (1) the agent's write relays as advertised in its [NIP-65](65.md) `kind:10002` relay list (`pubkey_a` is the author of every record), (2) an out-of-band agreed list configured by both parties when no `kind:10002` is published. URLs are compared after **canonicalizing**: lowercase scheme and host, strip default port (443 for `wss`, 80 for `ws`), strip a trailing slash on an otherwise empty path; the path is otherwise preserved verbatim. After canonicalization, duplicates MUST be deduplicated before querying. The owner uses the same list (and the same canonicalization) to locate the agent's memory.

## Record types

Two `kind:30174` record types share the same envelope and differ only by the slug at which they are addressed:

- **`core`** — exactly one per `(pubkey_a, pubkey_o)` pair. Holds agent identity/rules/goals and a client-maintained slug index. Bootstrap address.
- **`memory`** — zero or more per `(pubkey_a, pubkey_o)` pair. Each holds one logical entry.

Both are *addressable* per [NIP-01](01.md): the relay retains only the newest event per `(kind, pubkey_a, d)`.

## Slugs

A **slug** identifies a record. A valid slug is either the reserved string `core` or matches:

```
^mem/[a-z0-9][a-z0-9_-]{0,63}(/[a-z0-9][a-z0-9_-]{0,63})*$
```

with total length ≤ 255 bytes. Wherever this NIP refers to "a slug" elsewhere (including the wiki-link syntax), it means a string satisfying this grammar.

## Addressing

The `d` tag of a record is derived from its slug:

```
K_c = nip44_conversation_key(nsec_a, pubkey_o)
    = nip44_conversation_key(nsec_o, pubkey_a)         # symmetric per NIP-44
d   = lower_hex(HMAC-SHA256(K_c, utf8("agent-memory/v1/d-tag") || 0x00 || utf8(slug)))
```

`K_c` is the [NIP-44](44.md) conversation key — the output of `HKDF-extract` over the 32-byte x-coordinate of the ECDH shared point, with `salt = utf8("nip44-v2")` — and is therefore uniformly random, suitable for direct use as an HMAC key. Each party computes it with their own private key and the other party's public key; the result is identical to both. `d` is the full 64-hex-character HMAC output and reveals no information about the slug to passive observers. The domain prefix `"agent-memory/v1/d-tag"` (followed by a single `0x00` byte separating it from the slug bytes) is fixed and version-tagged independently of this NIP's assigned number; future versions MUST change it to avoid colliding with deployed v1 records.

Implementations MUST NOT include the slug or any plaintext form of it in tags.

## Event envelope

```jsonc
{
  "kind": 30174,
  "pubkey": "<pubkey_a>",
  "created_at": <unix_seconds>,
  "tags": [
    ["d", "<64-hex>"],
    ["p", "<pubkey_o>"]
  ],
  "content": "<nip44_ciphertext>"
}
```

There MUST be exactly one `d` tag and it MUST be the value derived in *Addressing*. There MUST be exactly one `p` tag and it MUST contain `pubkey_o`; it both identifies the owner publicly and tells the agent which counterparty key was used (the owner uses the event's `pubkey` field as the same hint in the opposite direction). The decrypted `content` is a JSON object (see *Bodies*).

## Bodies

A body's `slug` discriminates its type: `slug == "core"` is a **core body**; any slug matching the `mem/…` grammar is a **memory body**.

**Memory body** is a JSON object containing `slug` (a valid slug) and `value` (a UTF-8 string or `null`). **Core body** is a JSON object containing `slug` (the string `"core"`), `profile` (a UTF-8 string), and `index` (an object mapping valid `mem/…` slugs to objects containing `event_id` (lowercase 64-hex string, per [NIP-01](01.md)) and `created_at` (non-negative integer)).

Bodies MAY contain fields beyond those defined here; unknown fields MUST be ignored by readers and do not affect validity. A body missing a required field, or whose required field has the wrong type, is invalid (see *Head selection* rule (5)).

Richer taxonomies (provenance, trust levels, attention/working sets, structured links, owner-to-agent directives) are intentionally out of scope for this NIP and belong in companion NIPs that add fields under the unknown-fields-permissive rule above.

### Memory body

```jsonc
{ "slug": "<slug>", "value": "<utf-8 string>" }
```

A body with `"value": null` is a **tombstone**; the event is still published, but readers MUST treat the slug as absent.

### Core body

```jsonc
{
  "slug": "core",
  "profile": "<agent identity, rules, goals>",
  "index": {
    "<slug>": { "event_id": "<lowercase 64-hex>", "created_at": <unix_seconds> },
    ...
  }
}
```

`profile` is free-form UTF-8 maintained by the agent. `index` is a client-maintained cache and is **advisory, not authoritative** (see *Listing*).

Implementations MAY additionally publish [NIP-09](09.md) deletion requests for superseded or tombstoned events of either type; the in-band tombstone (for memory) and replacement (for core) are the protocol-level semantics and are what readers act on. Such deletion requests SHOULD include `["k", "30174"]` per NIP-09 and use an `a`-tag identifier `30174:<pubkey_a>:<d>`. NIP-09 deletes all versions up to the deletion's `created_at`; a subsequent write with a later timestamp resurrects the slug under *Head selection* and is the intended recovery path. Honoring and non-honoring relays will diverge on pre-deletion history.

## Encryption

`content` is encrypted with [NIP-44](44.md) v2 using `K_c`. NIP-44 limits plaintext to 65,535 bytes; this limit applies to the body bytes passed to NIP-44 (whatever JSON serialization the implementation chose).

## Head selection

An event is **valid** for this NIP if all of the following hold:

1. `kind == 30174`, `pubkey == pubkey_a`, exactly one `d` tag, exactly one `p` tag, and the `p` tag value is `pubkey_o`.
2. Its signature verifies (per [NIP-01](01.md)). Validation MUST occur before decryption (per [NIP-44](44.md)).
3. Its `content` decrypts under `K_c` and parses as a JSON object.
4. The body's `slug` matches the *Slugs* grammar and re-derives to the event's `d` tag per *Addressing*.
5. The body's shape matches the type its `slug` discriminates (per *Bodies*).

Let `d = derive(s)` per *Addressing*. The **head** of slug `s` is computed by querying every configured relay for `kind:30174` events authored by `pubkey_a` whose tags contain `["d", d]` and `["p", pubkey_o]`, taking the union of results, discarding invalid events, and selecting the surviving event with the greatest `created_at` (ties broken by lowest event `id` per [NIP-01](01.md)). The same procedure is used for reading, writing verification, and listing.

## Writing

To write slug `s` with body `b`:

1. Compute `d` and serialize `b` to JSON. Implementations MUST reject the write if the serialized body exceeds 65,535 bytes (the NIP-44 plaintext limit).
2. Compute the head of `s` per *Head selection* and let `T` be its `created_at` (or 0 if no head exists). Set `created_at := max(now, T + 1)`. Monotonicity defeats the NIP-01 same-second tiebreak (unpredictable under NIP-44 random nonces) and ensures fresh clients with no local state still produce strictly newer writes. If this value exceeds `now + 600` (i.e. the prior head's `created_at` is more than ten minutes in the future), the head is considered clock-poisoned: implementations MUST refuse the write and surface a conflict rather than publish a further-future timestamp that relays applying timestamp sanity-checks will also reject. Recovery is out of band.
3. Encrypt with NIP-44 under `K_c`. Tag `["d", d]`, `["p", pubkey_o]`. Sign and publish to the configured relays. This NIP scopes publishing to the agent's write relays only; owners discover memory by reading the same list, rather than by receiving `p`-tag-routed copies on their own [NIP-65](65.md) read relays — agents SHOULD NOT publish memory events to the owner's read relays.
4. **Verify.** Wait for at least one relay's `OK` acknowledgement for the published event before re-querying, then recompute the head of `s` per *Head selection*. Implementations SHOULD additionally wait for a small propagation window (default: one second after the first `OK`) before recomputing, to absorb the round-trip skew between relays that already accepted the write and relays that have not. If the recomputed head is not the event just published, treat as a **conflict** and surface to the caller; clients MUST NOT silently retry. Conflicts undetected within this window are treated as eventually-consistent and will be surfaced by the next read whose result differs from the writer's local view.

Memory writes SHOULD be followed by a core write that updates `index[s]` with the new `{event_id, created_at}` (or removes the entry on tombstone). Core writes apply the procedure above directly. Index updates MAY be batched or debounced across multiple memory writes; `index` is advisory.

## Reading

To read slug `s`: compute the head per *Head selection*. If it is absent or a tombstone, the slug has no entry. Otherwise return `value` (memory) or the body (core).

## Listing

Two mechanisms are defined; clients MUST support (a) and SHOULD support (b):

**(a) Walk.** Query every configured relay for `kind:30174` events from `pubkey_a` tagged `["p", pubkey_o]`, take the union, and discard invalid events (per *Head selection*). Group the survivors by `d` tag; for each group, select the event with the greatest `created_at` (ties broken by lowest `id`). Drop tombstones. Return the set of `{slug, event_id, created_at}` tuples (omitting `core`).

**(b) Cache.** Read `index` from core's body. Faster but may be stale; clients SHOULD reconcile against (a) periodically.

**Reindex** is the operation that replaces `index` with the result of (a) and republishes core. It is the recovery path for any divergence (CLI crash, concurrent writer, relay drop) and SHOULD be an explicit operation rather than automatic.

Discovery is specified by *result* — the set of head tuples — not by mechanism, so an out-of-band index (e.g. a relay-maintained materialized view over public `d` tags) can be added in a future NIP without changing the wire format defined here.

## References and reachability

A body MAY reference other slugs using wiki-link syntax: `[[<slug>]]`, where `<slug>` matches the *Slugs* grammar. References are extracted by literal substring match over the body's string fields (`profile` for core, `value` for memory); this NIP defines no escaping mechanism and no markup-aware exclusion. Bare slug-shaped strings without brackets are NOT references.

The **reachability graph** is rooted at `core.profile`; edges are the `[[…]]` references in `profile` and in reachable memories' `value`. The `index` field contributes no edges. Slugs outside this set are **orphans**; clients SHOULD expose them for review but MUST NOT delete them automatically.

## Concurrency

The verification step of *Writing* detects two concurrent writers whose events both reached the relay union: whichever loses (does not become the head) surfaces a conflict. Detection is best-effort — disjoint relay sets, network partitions, and writes arriving after verification will not be caught, and may converge to different heads at different observers until the next read crosses them.

## Security considerations

- **Agent key compromise.** Holders of `nsec_a` can rewrite or tombstone any record. On relays that honor addressable-event replacement no protocol-level trace remains; archival relays may show *that* rewrites occurred but cannot by themselves identify which version is authoritative. This NIP defines no mechanism for authoritative version chaining.
- **Owner key compromise.** Holders of `nsec_o` can decrypt all records but cannot write them; the consequence is confidentiality loss, not integrity loss.
- **Metadata leak.** The triple `(pubkey_a, kind:30174, p=pubkey_o)` reveals that an account uses agent memory and identifies its owner. Pseudonymous, not anonymous.
- **No owner write authority.** Only `nsec_a` can author records. This NIP defines no protocol-level mechanism by which an owner directs the agent's memory; that interaction is out of band.
- **Memory poisoning.** Encryption protects confidentiality, not the truthfulness of what the agent decides to remember. Admission control is the implementer's problem.

## Reference test vectors

> **TEST KEYS — DO NOT USE IN PRODUCTION.** The keys, nonces, and Schnorr aux values below are pinned for reproducibility. Production code MUST source nonces and aux from a CSPRNG.

### Inputs

```
nsec_a              = 0000000000000000000000000000000000000000000000000000000000000001
nsec_o              = 0000000000000000000000000000000000000000000000000000000000000002
schnorr_aux         = 0000000000000000000000000000000000000000000000000000000000000000   (all events)
```

Bodies are pinned as exact UTF-8 byte strings (no whitespace, key order as listed):

```
body_1 = {"slug":"mem/example","value":"hello, agent memory"}
body_2 = {"slug":"mem/notes/2026-05-12","value":"meeting note: [[mem/example]]"}
body_3 = {"slug":"mem/example","value":null}
body_4 = {"slug":"core","profile":"test agent. see [[mem/example]] and [[mem/notes/2026-05-12]].","index":{"mem/notes/2026-05-12":{"event_id":"1a43298ea1fa9b73462a85b9f16f5f6bd2a7ab18b0b02424e5ec3f3b8a48e030","created_at":1700000001}}}
```

### Derived

```
pubkey_a            = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798   (= secp256k1 generator G_x; cute sanity check for sec=1)
pubkey_o            = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
K_c                 = c41c775356fd92eadc63ff5a0dc1da211b268cbea22316767095b2871ea1412d   (matches nip44.vectors.json for sec1=…01, sec2=…02)

d("core")                  = bdc233238ffe52e272b44cc233c8f33a2bc510b08be04495b225964283be4a90
d("mem/example")           = 72d4f9629106451505d7d341ea85bb3ebad4f654fcfd2aad100d5a35f8a85cba
d("mem/notes/2026-05-12")  = 31651571a312780cfdc1f0b706b682ac9f3f51a053e8dca76fe57710bae5a4d4
```

### Events

Each event below uses `kind=30174`, `pubkey=pubkey_a`, `tags=[["d", d], ["p", pubkey_o]]`, and the `created_at`, NIP-44 nonce, and body listed. `sha256(content)` is taken over the base64 payload bytes (ASCII).

**Event 1 — write `mem/example`:**
```
created_at      = 1700000000
nip44_nonce     = 0000000000000000000000000000000000000000000000000000000000000001
content         = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABedgcxyfmpph68LBjCWZsTI5lb0Cbg8dIPVYVe/WVj/l4Yd8HGgzC8awyBi9bn9ClRdtd2IPsmont0jN/cajVSQhahTOwuNNwoJtZIg35aSsUzeCq4tQfd8E+fLoKomdPxjs=
content_len     = 176
sha256(content) = ff680a293019af12709972ae68b6ee79a47f354381a94ca4074d8e0fe3c8bb50
id              = f4a594177b7aeea4fe99a09efbf74ae85f0126244f322135682c405888a38689
sig             = 0a4582f0bc5995b9a010afda5984f568055988ebbe4552b4e0ec6d11aeb2b303af940f3d84726a7edd1763badb284eb3aa8457664ceba85a90d6252ed4b494cb
```

**Event 2 — write `mem/notes/2026-05-12`:**
```
created_at      = 1700000001
nip44_nonce     = 0000000000000000000000000000000000000000000000000000000000000002
content         = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACG/JBPvdZxDwAxOG7bY3AW2q1slZqBjQC3NxfPVtfcR+TGjp2GKtjyXyqNwG08GK+00I1u1vUZ4cCjcun9A7ra92rleKKJ5w57pqgFspbv1vClUJY5487A/5phVDHkw6DhRCSMDpEMw5Tapj3Wm1ponAVr5PciPOrTxltEfTVdSKaPA==
content_len     = 220
sha256(content) = ba7b026809363134c4f8de6cfbd82417b838e265281ff7e0005dc193bf1b32c8
id              = 1a43298ea1fa9b73462a85b9f16f5f6bd2a7ab18b0b02424e5ec3f3b8a48e030
sig             = dc9da456db1c89f070edc5f994786f270fc00e8ff19f33d5b0f6cea49421cd727fcd79bb288f3e3dbd5af9ca1ba67f9bd11b02a47c1e6c37cfd32665c17e4a24
```

**Event 3 — tombstone `mem/example` (supersedes Event 1; same `d`, greater `created_at`):**
```
created_at      = 1700000002
nip44_nonce     = 0000000000000000000000000000000000000000000000000000000000000003
content         = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADuau8i0Wu4+ULnp2qTfd+O23jJAapMRrKGGwabNVOlT9hSF8FViBHIS6f86/7xK4qGOin4IH8Wr/3cvHDcQGQd3IXQJr8LHgJkaYpQPdBO1bgqiFu8K3L/CLb1PgG1X7RQ8E=
content_len     = 176
sha256(content) = 0c9f72125f6460e68cb4b7ee42298afc8969840f83a156d90aa98a5f461fea44
id              = c8604bef05295856a67a88ec895e07b5b47a2febc23c82934734096a7b123b63
sig             = c8d53859cf08b3a9a20a5b01c61d12fa2f082f462adb635420f05dc6f9bb662a174e729023854bf53e5e35fae8f6f4c9d604e8979a070e298cd77cfb7e6b6468
```

**Event 4 — core (references Event 2's `id` in its `index`):**
```
created_at      = 1700000003
nip44_nonce     = 0000000000000000000000000000000000000000000000000000000000000004
content         = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEEV1HAFjhc8DAcKaVSSB7IoKG3nr+dX3LXlU7UIdOKayhIVPXvl4WuFmBSVxLO6yEV5vnLvzbo7rU0uPRYyAJLPNnifVTCw2EQZH70zOwTc/mVvaATHKzqcFotHOCAPboNxUN9fLHJDZ/3Mg/q5GOkQYqUWaa/cDdgQ5FM01oiREPwPOp8xySRQ/EyjZD1mhiIP+dwsYTVhScugJlGR6xJWUG3ohhTi0rj/L0exv/hYtuaOdZtCKmJmu230USmngFvu++nyQVzb0NwnUW958jVT8MM2HpPeYUlSqTwiKnMB6IqLOlM6JjiSoTW9vtMsbfdc+cx4OG6pZlDhtpuMSyoLak4wtTfqObN49yqR9GUiK2wuI/fj7TKf/BGapsCiIWpYs=
content_len     = 432
sha256(content) = e3b64fff85354a38fa5dc51a9738a9bd2bc2f8a129075f570af5e163b97b39a4
id              = 7daeaf5a9d9caeadde1597e38c0b4a3deaf3242f8fb9fdb835e52e54247599e5
sig             = 31459bc0cf13185b0368a913996211fcfc7aaeae4c124122ec4adf8887c798066494f796cfed29e10d99d60a5c973df1e4e49f33470e00aaa26268110a4ae7d4
```

### Implementation gotchas

Three places where independent re-derivations are most likely to diverge silently:

1. **NIP-44 ECDH IKM is *raw* `shared_x`** — the 32-byte x-coordinate of the shared secp256k1 point, unhashed. Libraries whose default `ecdh()` returns SHA-256(`shared_x`) (such as `libsecp256k1`'s default ECDH hash function) will produce a different `K_c`. Use a scalar-multiply path that exposes the bare point's x-coordinate.
2. **BIP-340 Schnorr `aux = 0x00…00` is not "aux omitted."** Aux of 32 zero bytes is passed through the `BIP0340/aux` tagged hash and XOR'd with the secret key; this matches the published BIP-340 test vectors. Some libsecp256k1 bindings expose only `schnorrsig_sign_custom` and default to NULL extraparams, which silently *skips* the XOR and produces different (still-valid) signatures. Use the 4-argument `schnorrsig_sign(ctx, sig, msg32, keypair, aux32)` form with the 32-byte aux explicitly. Self-check: reproducing BIP-340 test vector 0 (sec=`0…03`, msg=zeros, aux=zeros) MUST yield sig prefix `e907831f80…`.
3. **NIP-01 event-id serialization** is `json.dumps([0, pubkey, created_at, kind, tags, content], separators=(",", ":"), ensure_ascii=False)` over UTF-8 bytes. `ensure_ascii=False` matters even when bodies are pure ASCII — relying on default `ensure_ascii=True` will diverge the moment any body contains a non-ASCII character.
