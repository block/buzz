NIP-AR
======

Immutable Processor Artifacts
-----------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-44 (versioned encryption).

## Abstract

This document defines Buzz-local `kind:4640`, an immutable processor artifact. Its plaintext is a versioned, artifact-type-neutral JSON envelope. Artifact types define their own payload and provenance contracts without receiving new event kinds.

Audience and encoding are outer-event profiles. They do not alter the plaintext envelope. This version implements only an owner-private, NIP-44 v2 encrypted profile; channel and community-public profiles are reserved until their full relay authorization behavior exists.

## Motivation

Processor output is broader than folds: summaries, generated media, extraction results, build reports, evaluations, and future artifacts need different semantic contracts. Making fold identity, temporal coverage, or citations universal would freeze one processor into the base protocol.

The event kind supplies storage semantics and a query discriminator. The envelope's `artifact_type` supplies application semantics. The payload `schema` supplies a versioned data contract. `media_type` supplies rendering information. Conflating these axes makes every new artifact type either lie about fold fields or allocate another kind.

## Non-Goals

This version does not define channel-visible, channel-encrypted, or community-public authorization. It does not define group-key distribution, artifact lenses, an addressable “current artifact” head, migration of legacy fold payloads, or generic relay validation of encrypted profile payloads.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY as defined in RFC 2119.

- **artifact envelope**: the canonical plaintext JSON carried directly or encrypted in `content`.
- **artifact profile**: the contract selected by `artifact_type`.
- **audience profile**: outer tags and relay policy determining who may discover and read an event.
- **encoding profile**: how the artifact-envelope bytes are represented in `content`.

## Kind

| Kind | Name | Signer | Class | Purpose |
|------|------|--------|-------|---------|
| `4640` | Artifact | processor | regular | One immutable processor output |

`kind:4640` is an ordinary stored, non-replaceable event under NIP-01. Every publication has a distinct event id. History does not depend on a relay retaining replaced versions.

`4640` is Buzz-specific. It was already used by the accumulator branch for immutable fold artifacts when this generic envelope was introduced, so broadening that allocation avoids a second parallel artifact kind and preserves the kind's lifecycle. Generic Nostr clients may ignore it safely.

A future logical “current artifact” pointer MAY use a separate addressable event. It MUST point to immutable artifacts rather than changing `kind:4640` replacement semantics.

## Plaintext Envelope

```jsonc
{
  "version": 1,
  "artifact_type": "xyz.block.buzz.fold",
  "schema": "channel-digest@v1",
  "media_type": "text/markdown",
  "payload": {},
  "provenance": {}
}
```

| Field | Cardinality | Meaning |
|-------|-------------|---------|
| `version` | exactly 1 | Envelope version; this document defines integer `1`. |
| `artifact_type` | exactly 1 | Non-empty, globally scoped semantic profile identifier. |
| `schema` | exactly 1 | Non-empty profile-specific payload schema identifier. |
| `media_type` | exactly 1 | Non-empty media type of the primary rendered output. |
| `payload` | exactly 1 | Any JSON value accepted by the selected profile. |
| `provenance` | 0 or 1 | Profile-specific provenance protected with the payload. |

Writers MUST serialize the envelope as UTF-8 JSON. Readers MUST reject an unsupported `version`, an unknown field in version 1, or a payload that violates its selected artifact profile. Unknown `artifact_type` values MUST NOT be interpreted as folds; clients MAY retain or display them generically.

`artifact_type`, `schema`, and `media_type` are deliberately distinct. An artifact type may have multiple evolving schemas and may produce multiple renderable media types.

## Outer Event Profiles

Every profile MUST include exactly one `format` tag:

```json
["format", "buzz-artifact-v1"]
```

| Profile | Outer tags | `content` | Status |
|---------|------------|-----------|--------|
| Owner-private | `format`; `encoding=nip44-v2`; no `h` | NIP-44 v2 encryption of envelope JSON from author to self | supported |
| Channel-visible | one `h`; `format`; no `encoding` | envelope JSON | reserved |
| Channel-encrypted | one `h`; `format`; future group encoding | encrypted envelope JSON | reserved |
| Community-public | `format`; no `h`; no `encoding` | envelope JSON | reserved |

For the supported profile, the encoding tag is exactly:

```json
["encoding", "nip44-v2"]
```

The supported owner-private profile MUST NOT contain `h`, `a`, `e`, or `p` tags. Artifact profile data and provenance MUST NOT be copied into cleartext outer tags. The event still reveals its kind, author, creation time, format, encoding, and ciphertext size.

Writers MUST refuse reserved audience profiles. A format document does not create authorization: channel profiles remain unsupported until relay ingest, REQ, COUNT, by-id, search, and live-fanout gates all enforce the same policy.

## Fold Artifact Profile

The accumulator uses `artifact_type: "xyz.block.buzz.fold"`, `media_type: "text/markdown"`, and its selected fold schema. Its `payload` is the accumulator's versioned fold artifact object, including fold name, artifact version, document output, exact shown event ids, half-open coverage summary, source selection and transitive source channels, producer model, prompt hash, truncation state, and creation time.

These are fold-profile invariants, not base-envelope requirements:

- coverage truth is the exact shown-event-id set; timestamp bounds are summaries;
- citations MUST refer only to events shown during the applicable run;
- incomplete budget-limited runs MUST remain explicit;
- source-channel taint MUST travel with the fold chain and sharing MUST fail closed when provenance cannot prove the target audience already has access;
- the envelope `schema` MUST equal the fold payload's schema.

Artifacts such as generated images or build reports need not have temporal coverage, a fold name, citations, or a prior-version chain.

## Relay Behavior

For the only supported profile, Buzz treats `kind:4640` as author-only. The relay MUST omit its existence, count, content, tags, search matches, by-id results, and live fanout from every reader other than the authenticated author. A stray `h` MUST NOT widen or redirect access.

The relay validates the Nostr envelope and generic event limits. Since content is encrypted, profile-payload validation occurs after decryption in the client that owns the event.

## Client Behavior

A writer MUST:

1. construct and profile-validate the artifact payload;
2. construct the version-1 generic envelope;
3. serialize that envelope once;
4. encrypt those exact bytes according to the selected encoding;
5. build only the exact outer tag shape allowed by the selected audience/encoding matrix;
6. sign and publish the immutable event.

An owner-private reader MUST decrypt, parse the envelope, validate the envelope version, dispatch by `artifact_type`, and validate that profile. It MUST fail closed rather than treating malformed generic envelopes as another known profile.

During the accumulator branch transition, its CLI MAY read the legacy fold-only plaintext shape for compatibility with artifacts that branch already published. New writes MUST use the generic envelope.

## Security and Privacy

Cleartext provenance defeats encrypted content. Source channel coordinates, exact input event ids, fold names, schemas that expose sensitive intent, recipient identities not required for routing, and human-readable slugs MUST remain inside the encrypted envelope for owner-private artifacts.

NIP-44 self-encryption protects content from the relay but does not conceal event metadata or ciphertext size. Gift wrapping and group encryption are separate protocols and are not implied by this document.
