NIP-AE
======

Agent Engrams
-------------

`draft` `optional`

This NIP defines a convention for AI agents to store persistent, structured memory — *engrams* — on Nostr. Memory consists of addressable `kind:30174` events ([NIP-01](01.md)) signed by the agent's key and encrypted with [NIP-44](44.md) using the conversation key between the agent and its owner. Because that key is symmetric, both parties decrypt every event; the owner can always read everything the agent remembers.

## Kind

This NIP claims `kind:30174` for agent engrams. It is in the addressable range per [NIP-01](01.md): addressable events store only the latest per `(kind, pubkey, d)`, with relay query and retention behavior governed by NIP-01 (relays SHOULD return only the latest; some may retain older versions).

A dedicated kind (rather than encoding agent memory as a profile over NIP-78 `kind:30078` "Application-specific Data") is taken for two reasons: (1) it isolates this NIP's address space from any other application that the agent's pubkey also writes — `core` and `mem/…` slugs cannot collide with another app's `d` tag choices, regardless of agent reuse; (2) it lets observers, indexers, and unknown-kind viewers identify these events from the kind alone, without attempting NIP-44 decryption as a namespace demultiplexer.

## Roles

- **agent** — a Nostr identity (`pubkey_a`) that signs memory events.
- **owner** — a Nostr identity (`pubkey_o`) the agent serves. Identified by the `p` tag.

Memory is scoped to a single `(pubkey_a, pubkey_o)` pair. An agent serving multiple owners holds an independent memory per pair.

The phrase **configured relays** used throughout this NIP is, in order of precedence: (1) the agent's write relays as advertised in its [NIP-65](65.md) `kind:10002` relay list (`pubkey_a` is the author of every record) — entries marked `write` or with no marker, ignoring `read`-only entries and entries whose URL is not a syntactically valid `ws://` or `wss://` URL; (2) the out-of-band agreed list when no `kind:10002` is published, when the published list yields zero usable entries after the filtering above, or for the bootstrap window before owner and agent have observed the agent's first `kind:10002`. URLs are compared *for equality only* after **canonicalizing**: lowercase scheme and host, strip default port (443 for `wss`, 80 for `ws`), strip a trailing slash on an otherwise empty path; the path is otherwise preserved verbatim. After canonicalization, duplicates MUST be deduplicated before querying. Connections SHOULD be made to the advertised URL as written, not the canonical form, so that any relay-side path or host disambiguation is preserved. The owner applies the same comparison rule to locate the agent's memory.

Because persistence rides the agent's configured relay set, the agent SHOULD republish current heads to the new set before decommissioning any relay it is leaving. This NIP defines no automatic migration mechanism; agents that rotate relays without migrating their heads will lose access to memory not also present on retained relays.

## Record types

Two `kind:30174` record types share the same envelope and differ only by the slug at which they are addressed:

- **`core`** — exactly one per `(pubkey_a, pubkey_o)` pair. Holds agent identity, rules, and goals. Bootstrap address.
- **`memory`** — zero or more per `(pubkey_a, pubkey_o)` pair. Each holds one logical entry.

Both are *addressable* per [NIP-01](01.md): only the newest event per `(kind, pubkey_a, d)` is served, and head selection (below) tolerates relays that surface older versions anyway.

## Slugs

A **slug** identifies a record. A valid slug is either the reserved string `core` or matches:

```
^mem/[a-z0-9][a-z0-9_-]{0,63}(/[a-z0-9][a-z0-9_-]{0,63})*$
```

with total length ≤ 255 bytes. Wherever this NIP refers to "a slug" elsewhere (including the wiki-link syntax), it means a string satisfying this grammar.

## Addressing

The `d` tag of a record is derived from its slug:

```
K_c = nip44_conversation_key(seckey_a, pubkey_o)
    = nip44_conversation_key(seckey_o, pubkey_a)         # symmetric per NIP-44
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

There MUST be exactly one `d` tag and it MUST be the value derived in *Addressing*. There MUST be exactly one `p` tag and it MUST contain `pubkey_o`; it both identifies the owner publicly and tells the agent which counterparty key was used (the owner uses the event's `pubkey` field as the same hint in the opposite direction). Implementations MAY include a [NIP-31](31.md) `["alt", "encrypted agent memory record"]` tag (or equivalent fixed string) to give unknown-kind viewers a non-leaking summary; additional tags beyond `d`, `p`, and `alt` are not defined by this NIP and have no effect on validity. The decrypted `content` is a JSON object (see *Bodies*).

## Bodies

A body's `slug` discriminates its type: `slug == "core"` is a **core body**; any slug matching the `mem/…` grammar is a **memory body**.

**Memory body** is a JSON object containing `slug` (a valid slug) and `value` (a UTF-8 string or `null`). **Core body** is a JSON object containing `slug` (the string `"core"`) and `profile` (a UTF-8 string).

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
  "profile": "<agent identity, rules, goals>"
}
```

`profile` is free-form UTF-8 maintained by the agent. Clients MAY maintain a local cache of `{slug → {event_id, created_at}}` for memory entries to accelerate listing, but such a cache is implementation-local and outside this NIP — the authoritative listing procedure is the walk in *Listing*.

Implementations MAY additionally publish [NIP-09](09.md) deletion requests for superseded or tombstoned events of either type; the in-band tombstone (for memory) and replacement (for core) are the protocol-level semantics and are what readers act on. Per NIP-09 a deletion request MUST be authored by the same key as the events it targets, so only `pubkey_a` may delete these records; such requests SHOULD include `["k", "30174"]` and use an `a`-tag identifier `30174:<pubkey_a>:<d>`. A NIP-09 request asks honoring relays to delete every targeted event with `created_at` ≤ the request's `created_at`; whether relays honor it is their policy. A subsequent write with a later timestamp resurrects the slug under *Head selection* and is the intended recovery path. Honoring and non-honoring relays will diverge on pre-deletion history.

## Encryption

`content` is encrypted with [NIP-44](44.md) v2 using `K_c`. NIP-44 limits plaintext to 65,535 bytes; this limit applies to the body bytes passed to NIP-44 (whatever JSON serialization the implementation chose).

## Head selection

An event is **valid** for this NIP if all of the following hold:

1. `kind == 30174`, `pubkey == pubkey_a`, exactly one `d` tag, exactly one `p` tag, and the `p` tag value is `pubkey_o`.
2. Its signature verifies (per [NIP-01](01.md)). Validation MUST occur before decryption (per [NIP-44](44.md)).
3. Its `content` decrypts under `K_c` and parses as a JSON object. Duplicate object member names anywhere in the body MUST cause this rule to fail (parsers that silently first-wins or last-wins would otherwise diverge on head selection).
4. The body's `slug` matches the *Slugs* grammar and re-derives to the event's `d` tag per *Addressing*.
5. The body's shape matches the type its `slug` discriminates (per *Bodies*).

Let `d = derive(s)` per *Addressing*. The **head** of slug `s` is computed by querying every configured relay for `kind:30174` events authored by `pubkey_a` whose tags contain `["d", d]` and `["p", pubkey_o]`, taking the union of results, discarding invalid events, and selecting the surviving event with the greatest `created_at` (ties broken by lowest event `id` per [NIP-01](01.md)). The same procedure is used for reading, writing verification, and listing.

## Writing

To write slug `s` with body `b`:

1. Compute `d` and serialize `b` to JSON. Implementations MUST reject the write if the serialized body exceeds 65,535 bytes (the NIP-44 plaintext limit).
2. Compute the head of `s` per *Head selection* and let `T` be its `created_at` (or 0 if no head exists). Set `created_at := max(now, T + 1)`. Monotonicity defeats the NIP-01 same-second tiebreak (unpredictable under NIP-44 random nonces) and ensures fresh clients with no local state still produce strictly newer writes. If the resulting `created_at` is far enough in the future that publishing it would itself be undesirable (e.g. the prior head's `created_at` is implausibly ahead of wall-clock time), the head SHOULD be treated as clock-poisoned and the write surfaced as a conflict rather than published; choice of threshold is left to the implementation.
3. Encrypt with NIP-44 under `K_c`. Tag `["d", d]`, `["p", pubkey_o]`. Sign and publish to the configured relays. The `p` tag carries its usual [NIP-01](01.md) meaning (a referenced pubkey), which means generic NIP-65-aware clients may also fan it out to the owner's read relays; this NIP neither requires nor forbids that behavior. Authoritative discovery is always from the agent's configured relays so that owners and observers converge on the same head set; copies arriving on owner read relays are a redundant cache, not a separate channel.
4. **Verify (recommended).** Implementations SHOULD recompute the head of `s` per *Head selection* after waiting for at least one relay's `OK` acknowledgement, optionally with a short propagation delay to absorb inter-relay skew. If the recomputed head is not the event just published, the writer SHOULD surface a **conflict** rather than silently retry. Verification is best-effort: disjoint relay sets, partitions, and writes arriving after the recompute window will not be caught and remain subject to the eventual-consistency semantics described under *Concurrency*.

## Reading

To read slug `s`: compute the head per *Head selection*. If it is absent or a tombstone, the slug has no entry. Otherwise return `value` (memory) or the body (core).

## Listing

To list every memory entry for `(pubkey_a, pubkey_o)`: query every configured relay for `kind:30174` events from `pubkey_a` tagged `["p", pubkey_o]`, take the union, and discard invalid events (per *Head selection*). Group the survivors by `d` tag; for each group, select the event with the greatest `created_at` (ties broken by lowest `id`). Drop tombstones. Return the set of `{slug, event_id, created_at}` tuples (omitting `core`).

Listing is **best-effort**: Nostr has no protocol-level pagination, so relays MAY cap the number of events returned per query, and a result set silently bounded by such a cap will under-report. Implementations SHOULD treat the head-tuple set as a snapshot, not a guaranteed-complete enumeration, and SHOULD surface a per-relay event count or "limit reached" signal where one is available so callers can detect truncation. An out-of-band acceleration (e.g. a relay-maintained materialized view over public `d` tags, or an implementation-local cache) MAY be used so long as it returns the same tuples as the procedure above; a future NIP can standardize one without changing the wire format defined here.

## References and reachability (non-normative)

This section describes an optional convention; conformance does not require honoring it, and validity is unaffected.

A body MAY reference other slugs using wiki-link syntax: `[[<slug>]]`, where `<slug>` matches the *Slugs* grammar. When implementations choose to extract references, they do so by literal substring match over the body's string fields (`profile` for core, `value` for memory); this NIP defines no escaping mechanism and no markup-aware exclusion. Bare slug-shaped strings without brackets are NOT references.

A **reachability graph** rooted at `core.profile`, with edges being the `[[…]]` references in `profile` and in reachable memories' `value`, gives implementations a deterministic answer to "which memories are referenced from the agent's identity surface." Slugs outside this set are **orphans**. Clients that present this view to users SHOULD expose orphans for review and MUST NOT delete them automatically. A companion NIP may make this normative.

## Concurrency

The verification step of *Writing* detects two concurrent writers whose events both reached the relay union: whichever loses (does not become the head) surfaces a conflict. Detection is best-effort — disjoint relay sets, network partitions, and writes arriving after verification will not be caught, and may converge to different heads at different observers until the next read crosses them.

## Security considerations

- **Agent key compromise.** Holders of `seckey_a` can rewrite or tombstone any record and can derive `K_c` against every known owner pubkey, decrypting all past and future records for those pairs. On relays that honor addressable-event replacement no protocol-level trace of a rewrite remains; archival relays may show *that* rewrites occurred but cannot by themselves identify which version is authoritative. This NIP defines no mechanism for authoritative version chaining.
- **Owner key compromise.** Holders of `seckey_o` can decrypt all records but cannot write them; the consequence is confidentiality loss, not integrity loss.
- **Metadata leak.** The triple `(pubkey_a, kind:30174, p=pubkey_o)` reveals that an account uses agent memory and identifies its owner. Pseudonymous, not anonymous.
- **No owner write authority.** Only `seckey_a` can author records. This NIP defines no protocol-level mechanism by which an owner directs the agent's memory; that interaction is out of band.
- **Memory poisoning.** Encryption protects confidentiality, not the truthfulness of what the agent decides to remember. Admission control is the implementer's problem.

## Reference test vectors

> **TEST KEYS — DO NOT USE IN PRODUCTION.** The keys, nonces, and Schnorr aux values below are pinned for reproducibility. Production code MUST source nonces and aux from a CSPRNG.

### Inputs

```
seckey_a    = 0000000000000000000000000000000000000000000000000000000000000001
seckey_o    = 0000000000000000000000000000000000000000000000000000000000000002
schnorr_aux = 0000000000000000000000000000000000000000000000000000000000000000   (all events)
```

Bodies are pinned as exact UTF-8 byte strings (no whitespace, key order as listed):

```
body_1 = {"slug":"mem/example","value":"hello, agent memory"}
body_2 = {"slug":"mem/notes/2026-05-12","value":"meeting note: [[mem/example]]"}
body_3 = {"slug":"mem/example","value":null}
body_4 = {"slug":"core","profile":"test agent. see [[mem/example]] and [[mem/notes/2026-05-12]]."}
```

### Derived

```
pubkey_a = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
pubkey_o = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
K_c      = c41c775356fd92eadc63ff5a0dc1da211b268cbea22316767095b2871ea1412d   (matches nip44.vectors.json for sec1=…01, sec2=…02)

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

**Event 4 — core (publishes the agent profile; references `mem/example` and `mem/notes/2026-05-12` via wiki-links in `profile`):**
```
created_at      = 1700000003
nip44_nonce     = 0000000000000000000000000000000000000000000000000000000000000004
content         = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEEeZHAFjhc8DAcKaVSSB7IoKG3nr+dX3LXlU7UIdOKayhIVPXvl4WuFmBSVxLO6yEV5vnLvzbo7rU0uPRYyAJLPNnifVTCw2EQZH70zOwTc/mVvaATHKzqcFo5VHrbpKNTzeNnz1Vds2yg2DXmdxaoWQA4YfnlLwZDOpyu9JP1uB1Yw==
content_len     = 220
sha256(content) = 070f0f3e2e2bdc016b3ae06e8754e7814ffd4e98f0d5a70d75d1e8eab0d0e474
id              = 980419c4d231266471242456c832d0c2eb1e6974468dc795f3ae327484129058
sig             = ce113fff1205eadb38928b224a90247be1a00b0c3f8ab583d4a5f7274ddba51ebb5eb9d627d44664a78d2e870e61835cf61446cc812ecea139e8b7d41b8e238f
```

### Implementation gotchas

Three places where independent re-derivations are most likely to diverge silently:

1. **NIP-44 ECDH IKM is *raw* `shared_x`** — the 32-byte x-coordinate of the shared secp256k1 point, unhashed. Libraries whose default `ecdh()` returns SHA-256(`shared_x`) (such as `libsecp256k1`'s default ECDH hash function) will produce a different `K_c`. Use a scalar-multiply path that exposes the bare point's x-coordinate.
2. **BIP-340 Schnorr `aux = 0x00…00` is not "aux omitted."** Aux of 32 zero bytes is passed through the `BIP0340/aux` tagged hash and XOR'd with the secret key; this matches the published BIP-340 test vectors. Some libsecp256k1 bindings expose only `schnorrsig_sign_custom` and default to NULL extraparams, which silently *skips* the XOR and produces different (still-valid) signatures. Use the 4-argument `schnorrsig_sign(ctx, sig, msg32, keypair, aux32)` form with the 32-byte aux explicitly. Self-check: reproducing BIP-340 test vector 0 (sec=`0…03`, msg=zeros, aux=zeros) MUST yield sig prefix `e907831f80…`.
3. **NIP-01 event-id serialization** is `json.dumps([0, pubkey, created_at, kind, tags, content], separators=(",", ":"), ensure_ascii=False)` over UTF-8 bytes. `ensure_ascii=False` matters even when bodies are pure ASCII — relying on default `ensure_ascii=True` will diverge the moment any body contains a non-ASCII character.
