# NIP-FI runtime conformance and client-status boundary

This document maps Buzz runtime evidence to the normative
[NIP-FI specification](NIP-FI.md), [formal model](NIP-FI-MODEL.md), and
[conformance matrix](NIP-FI-CONFORMANCE.md). It does not make a conformance
claim. Discovery remains absent until an injected report proves that every
applicable row passed against one reviewed implementation revision.

## Discovery gate

`RelayInfo::build` omits both `limitation.federated_identity` and the top-level
`federated_identity` object. `ConformanceReadyNipFiDiscovery` is the only API
that can add them. It requires all of the following:

- a provider-neutral discovery value with at least one unique supported
  transport and exactly one enrollment mode;
- a positive finite delegated-lease maximum whenever delegation is advertised;
- an exact 40-character reviewed Git revision;
- an injected complete-stack result asserting that every applicable row passed
  at that same revision; and
- for `trusted-proxy`, deployment evidence for both origin isolation and
  stripping untrusted inbound assertion-header copies.

The reviewed revision and deployment evidence are gate inputs, not public
metadata. NIP-11 exposes no issuer URL, tenant URL, claim name, subject,
audience, assertion header name, or provisional NIP number. Unsupported
behavior is omitted rather than advertised as partially implemented.

The assertion header is singular at every ingress. Repeated field lines,
comma-combined values, invalid UTF-8, and empty values fail closed. An installed
adapter must supply verified transport provenance; header presence alone is
never classified as `trusted-proxy`. Protected media `GET` and `HEAD` remain
inside the enforcing transport inventory unless a separate reviewed public
media policy is selected.

The optional assertion `iat` check uses the shared injected authorization
clock and accepts a missing claim. A present value must be an unsigned integer
no later than injected verifier time plus the bounded 60-second skew. The
current `jsonwebtoken` dependency still evaluates `exp` and `nbf` against its
own system-clock source. Therefore `AS-3.future-iat` is covered, but this
candidate does not claim that all assertion-time checks use one injected clock;
that inherited limitation remains part of the full-stack review.

## Relay-authenticated client status

Kind `24244` is a Buzz-local, short-lived presentation contract, not NIP-FI
authorization evidence or a NIP-FI conformance surface. A status is signed by
the trusted relay and scoped to an exact server-resolved authorization domain
and event-author key. A current status contains a binding version, a
privacy-keyed policy revision, a monotonic durable status revision, and a
bounded validity window. A withdrawal contains only its exact scope, revision,
and bounded validity window. The two wire states are:

- `display_current`; or
- `withdrawn`, with no lifecycle cause or historical binding fields.

Clients fold only within one trusted relay/domain/author scope. A lower
revision is rejected. An equal revision is idempotent only for the identical
signed event; a conflicting equal revision is rejected. Expiry, disconnect,
relay-key change, authorization-domain change, or event-author change clears
presentation. Revision high-water state may survive a transient disconnect,
but it is never authority.

The relay adapter is one-way from `VerificationOnlyDisposition` to a signed
event. It has no dependency on authorization leases, membership mutation,
event ingest, persistence, subscriptions, pub/sub, ordinary delivery, or
NIP-85. The production seam targets an exact authenticated connection and can
construct its permit only from typed evidence that the RFC presentation,
privacy, and dedicated-client gates passed at one exact reviewed revision. The
stock binary supplies no such evidence, key, or transport, so status remains
disabled by default.

The optional label constructor accepts only privacy-approved server
configuration. There is no constructor from issuer data, subject data,
`display_name`, mutable profile content, or provider decisions. The policy
revision is a length-framed, domain-separated HMAC under an injected dedicated
client-status privacy key; identical provider values are unlinkable under
distinct keys.

## Stable row allocation

The synthetic fixture is
`crates/buzz-relay/tests/fixtures/nip_fi_trusted_proxy.json`. It contains no
production issuer, subject, domain, key, assertion, or tenant data.

| Row | Evidence in this lane | Full-stack state |
|---|---|---|
| `TR-1.direct-bypass` | Negative origin-isolation fixture | Deployment proof still required |
| `TR-1.inbound-header-copy` | Negative header-copy fixture | Deployment proof still required |
| `TR-1.complete-deployment-evidence` | Positive two-control fixture shape | Real enforced-control evidence still required |
| `AS-3.future-iat` | Optional assertion `iat` accepts absence and bounded skew; malformed or farther-future values fail closed. Status also rejects future issue time | Covered by O4 |
| `BD-1.cross-domain` | Status validation and folding reject cross-domain scope | Authorization-runtime row must pass at the reviewed revision |
| `SE-4.invalidation` | Withdrawal and client clearing are covered | Lease invalidation runtime must pass at the reviewed revision |
| `DG-3.no-finite-bound` | Discovery cannot represent delegation without a positive bound | Delegation authorization must pass at the reviewed revision |
| `OP-2.discovery` | Default omission, provider-neutral fields, and complete-stack gate | Covered here; final claim still requires all rows |
| `OP-3.absent` | No real-user route or ordinary delivery path | Covered |
| `OP-3.implemented` | Dedicated exact-connection production seam exists behind typed complete-stack approval | Disabled unless the approval, privacy key, transport, and runtime are explicitly installed |
| `OP-4.privacy` | Keyed revision, bounded configured label, field/source scans | Covered |

The local client-status rows are:

- `J3C-STATUS-RELAY-SIGNER`
- `J3C-STATUS-EXACT-SCOPE`
- `J3C-STATUS-FRESHNESS`
- `J3C-STATUS-REVISION-FOLD`
- `J3C-STATUS-WITHDRAWAL`
- `J3C-STATUS-PRIVACY`
- `J3C-STATUS-VERIFY-ONLY`
- `J3C-STATUS-DEDICATED-TRANSPORT`
- `J3C-STATUS-REAL-USER-HIDDEN`

These J3C rows test presentation safety only. They cannot substitute for any
NIP-FI authorization, lifecycle, session, delegation, or deployment row.

## Public projection retirement join

The existing opt-in NIP-85 label projection is separate from both NIP-FI
authorization and kind `24244` client status. Its active assertion is TTL
bounded, but a committed revoke or rotate also needs an inactive parameterized
replacement for the old public key.

The public-projection retirement reconciler is a provider-neutral post-commit
seam. It derives private retry work from committed lifecycle rows and persists
only public event coordinates plus opaque binding generations. The relay reads
the exact relay-authored projection and, when active, writes the existing
`active=false`, `expiration=0`, label-free replacement. A missing or already
inactive projection is an idempotent terminal result. Store or clock failure
leaves both lifecycle authority and the active projection unchanged while the
work remains retryable.

Active publication and retirement share the identity-key and parameterized
event commit boundaries. Server-only head metadata prevents a delayed rotation
job from retiring a later legitimate use of the same key. Startup and periodic
reconciliation provide restart recovery and Redis/local delivery retry. This
lane does not add authenticated lifecycle endpoints or durable operator audit.

## Compatibility cases

| Case | Required result |
|---|---|
| Old relay, new client | No discovery or status; client shows no indicator |
| New relay, old client | Unknown ephemeral status is ignored; ordinary event behavior is unchanged |
| Mixed relay fleet before complete conformance | Discovery stays absent; presentation stays disabled |
| Stale client cache | Expired status is cleared; lower or conflicting revisions cannot restore it |
| Spoofed user event | Wrong signer, kind, tags, content, or signature is rejected |
| Cross-domain replay | Exact expected domain and author mismatch is rejected; scope change clears state |
| Relay signing-key rotation | Old presentation is cleared and the new relay key must be trusted independently |
| Privacy-key rotation | Policy revision changes; it grants no authority and clients accept it only at a higher durable status revision |
| Provider or lifecycle outage | Relay issues an opaque `withdrawn` status only with authoritative revision evidence, otherwise emits nothing |
| Gate disabled | No presentation runtime is installed; no real-user status is delivered |

## Mechanical checks

Run from the repository root in the Hermit environment:

```sh
cargo test -p buzz-core client_binding_status
cargo test -p buzz-relay authorization_runtime::status
cargo test -p buzz-relay nip11
cargo test -p buzz-relay --test nip_fi_runtime_conformance
```

The integration test scans ordinary relay ingest, API, router, state,
subscription, connection, and protocol sources, plus desktop, mobile, and web
client sources. Any reference to the status kind, contract, or disabled
delivery method fails the test.
