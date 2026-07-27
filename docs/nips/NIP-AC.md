NIP-AC
======

Agent Consent Windows
---------------------

`draft` `optional`

**Depends on**: [NIP-OA](NIP-OA.md) (Owner Attestation), BIP-340 Schnorr
signatures

> Note to maintainers: this document uses the code `AC` (Agent Consent)
> because `CW` is assigned to [NIP-CW](NIP-CW.md) Channel Window. The code may
> be reassigned.

## Abstract

This NIP defines a bounded, human-signed, revocable authorization window
that an owner grants to an agent, layered over [NIP-OA](NIP-OA.md)-style
provenance. Unlike a NIP-OA `auth` tag, a consent window can be unilaterally
revoked by the owner without relay cooperation, and its expiry is enforced
by the verifier's own clock rather than by relay policy.

## Motivation

NIP-OA grants an agent a *reusable capability*: one `auth` tag authorizes many
events, bounded only by optional `created_at` conditions. NIP-OA states this
directly — "A valid `auth` tag is a reusable capability" — and
[NIP-AA](NIP-AA.md) §Revocation Semantics states the consequence:
"Revocation requires one of: (a) removing the owner from the relay's member
list, (b) the `auth` tag's `created_at` conditions expiring, or (c) the relay
applying an independent denylist. NIP-OA credentials are reusable capabilities —
the owner cannot unilaterally revoke a previously issued `auth` tag without one
of these mechanisms."

Of those three, (a) and (c) are relay actions, and (b) is the passive expiry of
a condition evaluated against a field the agent itself populates — NIP-AA notes:
"`created_at` is agent-controlled. A misbehaving agent can set `created_at` to
any value."
None of the three mechanisms is initiated by the owner.

Under a trusted relay, the operator is the authority and membership removal is
an effective remedy. Under an untrusted relay, no such authority exists: there
is no membership to remove and no denylist a verifier is obliged to honor. An
owner who authorizes an agent for a task and later withdraws that authorization
has no signed instrument expressing the withdrawal.

This NIP defines the complementary instrument: a least-privilege, time-bounded
authorization, revocable by the owner at any time. The construction is
relay-independent, so the same agent-plane applies whether the relay is a
trusted workspace server or an untrusted relay.

## Non-Goals

This NIP does not replace NIP-OA or NIP-AA. Provenance (which owner authorized
an agent) and relay admission remain defined by those NIPs. This NIP determines
whether an existing authorization is currently in effect.

This NIP does not define transport. A window may be published as an event or
sealed inside a [NIP-59](59.md) wrap; this NIP is independent of the choice.

This NIP does not guarantee revocation propagation. It guarantees that a
revocation, once seen, is self-authenticating and needs no relay's cooperation
to take effect. Propagation is a delivery problem, bounded by the duration rule.

This NIP does not define scope semantics. `scope` is an opaque identifier the
issuer and verifier agree on out of band.

## Definitions

- **Consent window**: a signed statement by the owner's *human identity key*
  that an agent MAY act autonomously within a bounded time interval and scope.
- **Revocation**: a signed statement by the same human key that a window has
  ended, effective immediately regardless of its original expiry.
- **Verifier**: any party (recipient, harness, node) deciding whether an agent
  action is currently authorized.

## The Window

A consent window is a signed object with the following fields:

```jsonc
{
  "type":       "consent_window",
  "agent":      "<agent_pubkey_hex>",
  "scope":      "<opaque scope id — a channel, a task, a thread>",
  "not_before": <unix_seconds>,
  "not_after":  <unix_seconds>,      // MUST be > not_before; bounded (see below)
  "nonce":      "<random id, for revocation targeting>"
}
```

### Canonical Serialization

Signing is over a compact JSON array in fixed field order, following the
NIP-01 event-id precedent (positional, independent of object-key ordering, no
whitespace):

```
serialization = ["consent_window",<agent>,<scope>,<not_before>,<not_after>,<nonce>]
message       = SHA256(UTF8(serialization))
sig           = BIP-340 Schnorr(message, human_identity_secret_key)
```

Strings are JSON-escaped; integers are canonical decimals with no leading zeros,
no `+`, and no fractional part. The serialization contains no whitespace.
`agent` is lowercase hex.

Rules:

1. **Human-signed only**: A window signed by the agent key MUST be rejected. An
   agent cannot self-authorize. This restates NIP-OA's existing self-attestation
   rule ("If `<owner-pubkey-hex>` equals `event.pubkey`, the `auth` tag is
   invalid and MUST be rejected") for this object; it is not a new property.
2. **Bounded duration**: `not_after − not_before` MUST NOT exceed an
   implementation ceiling (RECOMMENDED ≤ 30 days). An unbounded window is a
   reusable capability, which NIP-OA already provides; this NIP addresses the
   bounded case. Verifiers MUST enforce the ceiling on receipt rather than
   relying on the issuer to observe it.
3. **Clock-enforced**: A verifier evaluates `not_before ≤ now < not_after`
   against its own clock at action time. Expiry requires no relay
   participation. This is a deliberate departure from NIP-OA, whose
   verification MUST NOT depend on the verifier's clock: NIP-OA proves a past
   authorization event, while this NIP evaluates a present permission.

## Revocation

The owner ends a window early by publishing (or sealing) a signed revocation:

```jsonc
{ "type": "consent_revoke", "agent": "<agent_pubkey_hex>", "nonce": "<window nonce>" }
```

It is serialized and signed in the same manner:

```
serialization = ["consent_revoke",<agent>,<nonce>]
message       = SHA256(UTF8(serialization))
sig           = BIP-340 Schnorr(message, human_identity_secret_key)
```

The revocation is signed by the same human identity key that signed the window.
A verifier that has seen a matching revocation MUST treat the window as closed
from that moment, regardless of `not_after`. Because the revocation is a
self-contained signed object, it is effective under an untrusted relay: no
membership removal or relay authority is required. A revocation is idempotent
and MUST be retained at least until the revoked window's `not_after` has
passed.

## Verifier Behavior

An agent action is authorized at time `now` if and only if all of the
following hold:

1. A valid consent window exists for `(agent, scope)` with
   `not_before ≤ now < not_after`, signed by the owner's human key.
2. No valid revocation for that window's `nonce` has been seen.
3. (If provenance is also required) a valid [NIP-OA](NIP-OA.md) attestation
   binds the agent to that owner.

Absent a live window, an autonomous agent action MUST fail closed. A
verifier that cannot evaluate the conditions — malformed object, unknown signer,
clock unavailable — MUST also fail closed.

## Relationship to Other NIPs

- [NIP-OA](NIP-OA.md): provenance (which owner authorized an agent, without
  expiry). This NIP adds duration, scope, and revocability; the two are
  orthogonal and composable. A deployment may use either, or both.
- [NIP-AA](NIP-AA.md): relay-side revocation for trusted deployments. This NIP is
  the relay-independent counterpart for untrusted-relay deployments. A
  trusted-relay workspace obtains immediate revocation from NIP-AA; this NIP
  covers deployments where no such authority exists.
- [NIP-59](59.md): a window MAY be carried inside a gift wrap for metadata
  privacy, in which case it is verified after unwrap.
- [NIP-AD](NIP-AD.md): a live window authorizes an agent
  to act; it makes no claim about the safety of content the agent has read. The
  two checks are independent and both apply.
- [NIP-46](46.md): the `perms` list scopes what a client may ask a remote signer
  to sign. That scope is policy held by the signer and evaluated at signing
  time. A consent window is a signed object a third party can evaluate at action
  time without contacting the issuer, and the two mechanisms address different
  points in the flow.
- [NIP-40](40.md): the `expiration` tag marks an event for deletion by relays
  and clients. `not_after` is an authorization bound evaluated by a verifier at
  action time and carries no storage or deletion semantics.
- [NIP-ER](NIP-ER.md): uses a `not_before` tag for reminder due times. This NIP
  reuses the name with the same sense of a time floor, in a different object.
- [NIP-IA](NIP-IA.md): uses a `consent` tag recording which party authorized an
  archival action (`self`, `owner`, `admin`, or `relay`). That is a provenance
  record for a completed action; a consent window is a forward-looking, bounded
  grant. The terms are unrelated and neither field name collides.

## Security Considerations

**Clock skew**: Verifiers enforce expiry with local clocks; a window's bounds
should allow for reasonable skew. Timestamps are authorization bounds and are
not inputs to key derivation.

**Revocation propagation**: Under an untrusted relay a revocation must reach the
verifier to take effect. Owners requiring immediate global revocation should
narrow `not_after` accordingly; the bounded-duration rule caps the exposure
window even if a revocation is delayed. This mechanism does not provide the
immediate session termination available to a trusted relay.

**Replay**: A window is a bearer statement about `(agent, scope)`, not a
one-time token; re-presenting it within its bounds is expected. The `nonce` is a
revocation target, not an anti-replay mechanism.

**Human key exposure**: The window is signed by the human identity key, not the
agent key. Implementations MUST NOT hold the human key in the agent's process;
a compromised agent could otherwise sign its own windows, defeating rule 1.

**Grant sprawl**: A verifier accepting windows from many issuers SHOULD bound how
many live windows it retains per issuer; unbounded retention of peer-supplied
signed objects is a memory-exhaustion vector.

## Test Vectors

> **TEST KEYS — DO NOT USE IN PRODUCTION.** The keys are those of NIP-OA's
> test vectors, so an implementation that already passes NIP-OA's vector reuses
> the same key material here. `schnorr_aux` is all zeros for every signature
> below; production code MUST source aux from a CSPRNG.

### Inputs

```
human_secret = 0000000000000000000000000000000000000000000000000000000000000001
human_pubkey = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
agent_secret = 0000000000000000000000000000000000000000000000000000000000000002
agent_pubkey = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
schnorr_aux  = 0000000000000000000000000000000000000000000000000000000000000000

scope      = channel:7f3a
nonce      = 9f2c1a7b4e6d80f3
not_before = 1750000000
not_after  = 1750604800          (not_before + 7 days)
```

### Vector 1 — consent window

```
serialization = ["consent_window","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5","channel:7f3a",1750000000,1750604800,"9f2c1a7b4e6d80f3"]
sha256        = f2c136defe0cba238555a212e71067c83392e3985eee83d69e2870df1b613838
sig           = e06cb3567d3637d1df4b30fb5acd53e1f8de90086124bbd250497cd50ba321ff7b56a05e945f0b4a9ad3c6d0fe4f1edbb61f3063567aa46fc554fb194163a036
```

The signature verifies against `human_pubkey`. The duration is
604800 s = 7 days, within the RECOMMENDED 30-day ceiling.

### Vector 2 — revocation of Vector 1

```
serialization = ["consent_revoke","c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5","9f2c1a7b4e6d80f3"]
sha256        = 8c42c7fccc6a9e8d7d99de382dcd982e26f99677984f475e6230f784bdb5ad09
sig           = dd7ea749ef60f1df7519b765d3ac99553562e5e3d277496a48fb9a3d1565a38ae48043dd26fdd5ce18b558f8a635d0e283093e2f6c9b39ec61135e35f1bfd662
```

After a verifier has seen this revocation, Vector 1 is closed at every `now`,
including `now < 1750604800`.

### Vector 3 — NEGATIVE: agent-signed window MUST be rejected

The same serialization as Vector 1, signed with `agent_secret` instead of
`human_secret`:

```
sig = 564f7bf690edc07048578b8d5ec34a391efc420a6308ecba2baf0919e7fd4d3c65c8014776d4e287a36d43d70f1433eccaaccbe27b1ae8a028e039ff9c414259
```

This signature is cryptographically valid against `agent_pubkey` and MUST
still be rejected: rule 1 requires the signer to be the owner's human identity
key. An implementation that verifies the signature without checking the
signer's identity will accept a self-authorizing agent.

### Vector 4 — boundary behavior

With Vector 1's window and no revocation seen:

| `now`        | Authorized |
|--------------|------------|
| `1749999999` | no — before `not_before` |
| `1750000000` | yes — `not_before` is inclusive |
| `1750604799` | yes |
| `1750604800` | no — `not_after` is exclusive |

## Reference Implementation

Eldr's `AgentEngine` implements this specification: `AIWindowAnnouncement` is
human-key-signed and time-bounded, `receiveWindow` gates on `clock.now()` against
`maxWindowDuration`, `endMyAIWindow` emits a signed revocation, and
`authorizeAutonomousSend` fails closed absent a live window. Its standing-grant
variant adds scope and budget, is human-signed, bounded to at most 30 days,
revocable, and limited to 64 live grants per granter. The vectors above were
generated with the same BIP-340 implementation Eldr uses to pass NIP-OA's
vector.
