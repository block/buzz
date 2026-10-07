NIP-FI-CONF
===========

Conformance evidence profile
----------------------------

`draft` `optional`

**Dependencies**: NIP-FI core. Applies additionally to any claimed
NIP-FI-EDGE, NIP-FI-LIFECYCLE, and NIP-FI-DELEG profile.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", and
"MAY" in this document are to be interpreted as described in BCP 14 (RFC 2119
and RFC 8174) when, and only when, they appear in all capitals.

## Abstract

NIP-FI core and its profiles state required behavior. This profile states what
counts as evidence that an implementation has it: the claim unit, the evidence
rules, the complete denial-fixture enumeration, mutation adequacy, and the
interoperability exit test.

This profile is separately claimable and is never advertised in discovery:
conformance is a property of a reviewed revision, not a wire feature. It
defines no wire behavior, denial mapping, invariant, or admission rule; where
it names one, NIP-FI core or the owning profile is normative.

## Claim unit

A conformance claim names exactly one immutable tuple:

```text
(implementation revision,
 adapter revision,
 build artifact digest,
 deployment revision,
 governing document revision,
 exit fixture digest,
 claimed profiles,
 assertion_policy_id,
 transport_contract_id,
 enrollment mode)
```

Changing any element creates a new claim. Results from one tuple MUST NOT be
carried into another. A report contains every applicable oracle from core and
every claimed profile exactly once, with status `pass` or `not-applicable`
only, except that `FI-CONF-INTEROP-EXIT` alone may instead carry `deferred`
under the condition in **Interoperability exit test**. Blank, skipped,
expected-failure, and not-run results cannot support a claim
(`FI-CONF-CLAIM-COMPLETE`).

Enrollment mode is part of the claim unit and is private: it is recorded in
the access-controlled report, never in discovery or any public artifact.

## Evidence rules

Each passing oracle records the claim tuple, a stable test identifier and
adapter entry point, the command with start time, end time, exit status, and
any random seed, the synthetic input or a privacy-safe digest of it, the
before-and-after authoritative state relevant to the oracle, the expected and
observed outcomes, and artifact locations with SHA-256 digests. Stateful
oracles use an isolated database or namespace and inspect committed state
rather than inferring it from a response. Concurrency oracles record every
contender and the single serialized outcome. Time-boundary oracles use a
controlled clock.

Adapters MUST drive public or production-equivalent entry points. A storage
helper MAY inspect state or inject a dependency outage; it MUST NOT replace the
operation under test. Calling an internal authorization function without
traversing the protected ingress does not satisfy ingress coverage.

None of the following satisfies any oracle: searching source, documentation,
schemas, or binaries for a token; asserting that a route calls a named
function; recording a test name without its execution result; using a mock to
prove a deployed network boundary; citing a check from another revision; or
marking an oracle passed because the feature is configured.

`FI-TRACE-TOFU-THEFT` takes an access-controlled **configuration** witness
only; under the private-posture rule no discovery witness for enrollment mode
can exist. Discovery invariance is proved separately by
`FI-TRACE-DISCOVERY-PRIVATE`.

Requirements marked `[deployment artifact: ...]` in core or a profile are
evidenced by the named access-controlled review record at the claimed
deployment revision, not by a behavioral oracle. A claim listing an artifact
without the record is incomplete.

Reports and artifacts hold private deployment detail and MUST remain access
controlled. They MUST NOT enter public reports, examples, discovery, or
protocol output, and MUST NOT contain raw assertions, secrets, or unredacted
`iss`, `sub`, or claim values. The shared exit fixture is exempt: its values
are synthetic by construction and name no real principal, issuer, or key.

## Denial fixtures

`FI-TRACE-DENIAL-ORACLE` requires one fixture per **private condition**, not
one per public class; a per-class suite compares a class against itself. The
enumeration below is the required fixture set (`FI-CONF-DENIAL-FIXTURES`). The
public-class column restates NIP-FI core, which owns the mapping and the bytes.

| # | Private condition | Public class | Defined by |
|---|---|---|---|
| 1 | assertion, proof, or delegation evidence absent | `missing_evidence` | core |
| 2 | edge provenance absent or incomplete on an edge-required route (assertion may be present) | `missing_evidence` | NIP-FI-EDGE |
| 3 | evidence present but rejected: signature, key selection, issuer, audience, time, size, ambiguity, token class, body binding, or edge provenance (present but rejected) | `evidence_rejected` | core, NIP-FI-EDGE |
| 4 | replayed evidence — committed replay identity already claimed | `authorization_denied` | core, NIP-FI-EDGE |
| 5 | `key_mismatch` — asserted key is not the proven actor | `authorization_denied` | core |
| 6 | `attestation_required` — attested-key enrollment without a matching key claim | `authorization_denied` | core |
| 7 | `binding_conflict` — either side of the active relation is taken | `authorization_denied` | core |
| 8 | `pair_retired` | `authorization_denied` | core |
| 9 | `key_revoked` | `authorization_denied` | core |
| 10 | `policy_denied` — local operation policy | `authorization_denied` | core |
| 11 | `binding_required` — enrollment policy creates no binding at this request: provisioned mode with no binding, or any unrecognized policy value | `authorization_denied` | core |
| 12 | `identity_disabled` | `authorization_denied` | NIP-FI-LIFECYCLE |
| 13 | `explicit_replacement_required` — pending lineage | `authorization_denied` | NIP-FI-LIFECYCLE |
| 14 | `binding_expired` — administrative expiry | `authorization_denied` | NIP-FI-LIFECYCLE |
| 15 | `delegation_not_current` — owner or relationship no longer current | `authorization_denied` | NIP-FI-DELEG |
| 16 | `dependency_unreadable` | `authorization_unavailable` | core |

Private-condition names are fixture identifiers, not wire values; a deployment
MAY use other internal reason codes if every enumerated condition has a
fixture. Rows for an unclaimed profile are `not-applicable` with absence
evidence. A profile that introduces a private condition MUST add its row; an
unenumerated condition escapes this oracle entirely.

**Enumeration agreement.** `policy_denied` and `dependency_unreadable` are the
*prose-only allowlist*: core conditions that core states in prose and does not
name symbolically. The suite MUST check mechanically at the claimed head, by
symbol and never by row number, and every check MUST be green on the unmutated
documents before any mutant is scored:

1. every symbol core denies by name has a row here attributed to core with the
   same public class;
2. the set of symbols in core-attributed rows equals core's symbolic denial set
   together with the allowlist, exactly, and the allowlist is disjoint from
   that set; and
3. for each claimed profile that owns a private-denial-condition table, the
   set of `(identifier, public class)` pairs in that table equals the set of
   pairs attributed to that profile here, exactly; a row with multiple owners
   contributes its pair to each.

If a later core names an allowlisted symbol, check 2's disjointness fails
until the allowlist entry is deleted, and check 1 validates the promoted
symbol's class.

**Anonymity comparison.** Every `authorization_denied` row is in the
private-state anonymity set. Between two private conditions on one
implementation, every response byte as transmitted MUST agree — transfer
framing included — except values a server cannot hold constant across two
instants, such as `Date`. This is wider than the interoperability object
below: within one implementation, any byte that varies by private condition is
a disclosure, whatever field it sits in.

**Interoperability compared object.** Between two implementations, comparison
is over what core pins and nothing more. Over Nostr: the complete relay message
excluding only the event or subscription identifier echoed from the request,
as compact JSON with no insignificant whitespace per NIP-01. Over HTTP: the
status code; the content per RFC 9110 Section 6.4, after transfer decoding with
chunk framing and trailers excluded; and the exact values of only the header
fields core's denial table names, field names matched case-insensitively per
RFC 9110 Section 5.1. `Content-Length` is not pinned. Header order and unnamed
fields are outside the object, and their values MUST NOT depend on the private
condition. A field core names that an implementation cannot hold constant MUST
be reported with the reason, and its value MUST be independent of the private
condition. If core later pins another field, it joins with no edit here.

**Run discipline.** The oracle runs a fixed positive iteration count on a
pinned isolated runner at the exact claimed head. Before the run the operator
records the environment, public-response corpus, bounds, sampling method,
statistical rule, noise treatment, and acceptance threshold. A breach fails the
gate, MUST NOT trigger an automatic retry, and is retained and investigated
before a separately authorized rerun.

`authorization_unavailable` is observably distinct from `authorization_denied`.
This is accepted residual: it discloses no per-principal state, and collapsing
it would make fail-closed behavior undiagnosable.

**Negative control.** The suite MUST include an implementation deliberately
patched to vary its denial response by private condition, and it MUST fail
this oracle.

## Mutation adequacy

An oracle that cannot fail is untested text that reads as tested. The
denominator is the **listed oracle**: every table row whose first cell names
exactly one complete literal oracle identifier, in NIP-FI core, in each claimed
normative profile, and in this document when CONF is claimed — selected by
that cell, not by section title. It is not the set of normative sentences, RFC
2119 keywords, or invariant labels, none of which two readers enumerate alike.

For each listed oracle the suite MUST retain at least one **mutant**: an
implementation variant that violates a requirement that oracle governs,
together with that oracle's failing output (`FI-CONF-MUTATION`). Evidence is
the exact patch identity, the oracle identifier, and the retained failure
output at the claimed head. For this document's own oracles the implementation
under test includes the conformance suite and its report; a mutant is a single
variant of the suite or report that the entry's own oracle rejects.

While `FI-CONF-INTEROP-EXIT` is validly deferred it remains in claim
completeness but is excluded from this section's mutation and global-control
obligations, since its failing output cannot exist without the run. Both
obligations attach with the run and MUST be discharged before either
implementation's interoperable conformance claim is accepted. No other
oracle's obligation under this section is deferrable.

Normative prose outside the oracle tables remains binding but is not a second
denominator. Prose that no listed oracle can detect is untestable text: add the
oracle that detects it, or delete it.

1. **One at a time.** Mutants are applied singly against an otherwise
   unmodified implementation, so layered defenses cannot mask each other.
2. **Attribution.** The kill MUST come from the entry's own oracle. A mutant
   killed only by another oracle establishes coverage for neither.
3. **One entry per mutant.** A mutant satisfies only the entry it was selected
   for, even when it also kills other oracles.
4. **Reachability.** The suite MUST witness that a fixture reaches the mutated
   decision, not merely the enclosing operation.
5. **Survivors are recorded.** A mutant its named oracle fails to kill is a
   defect in the specification or the suite. It is recorded with that
   disposition and MUST NOT be waived or replaced by an easier mutant.

Two global controls bound the suite. A deny-everything implementation MUST
fail every positive oracle; an allow-everything implementation MUST fail every
negative oracle. Neither substitutes for per-entry mutants.

## Interoperability exit test

A claim of core conformance requires evidence that the documents alone are
sufficient to build against (`FI-CONF-INTEROP-EXIT`). Two implementations that
have not shared code and have not consulted a common reference implementation
each produce, from NIP-FI core and any claimed profile documents alone:

- one valid `client-attached` request, over WebSocket upgrade and over HTTP,
  compared over its signing inputs as defined below; and
- one byte-exact public denial response for each of the four public classes,
  on each transport where the class can be decided, compared over the
  interoperability compared object under **Denial fixtures**.

Independence is a claim about code, not inputs: two implementations given
different issuers, keys, or clocks cannot produce equal bytes. The run is
therefore parameterized by a **shared exit fixture** that both sides load and
neither side authors:

- one issuer identity and one JWK set, including the private key needed to
  mint assertions and the `kid` selecting it;
- one assertion per denial class and one for the valid request, each as
  complete pre-signature protected-header and claim-set JSON values —
  including `alg`, `typ`, `kid`, every member the policy allows, and fixed
  `iss`, `sub`, `aud`, `nostr_pubkey`, `client_id`, `iat`, `exp`, and token
  class;
- one Nostr secret key for the proof, with the complete unsigned event fields
  for each transport — the NIP-98 event over HTTP and the NIP-42 event with
  its challenge and relay values over the WebSocket upgrade — including
  `created_at`;
- one frozen evaluation instant, and the skew and lifetime bounds in force;
  and
- the domain, target resource, operation, and enrollment policy for each case.

The canonical fixture is authored by this document's editors, not by any
claiming implementation, and MUST be published as a single file at
`docs/nips/fixtures/nip-fi-conf-exit.json` in the same repository as these
documents, with its SHA-256 digest, before any `FI-CONF-INTEROP-EXIT` run.
Both sides MUST load that file, MUST verify the digest before the run, and
MUST record the digest with the evidence; a run against any other fixture
instance is not `FI-CONF-INTEROP-EXIT` evidence. While the canonical fixture
is unpublished, the claim tuple's exit fixture digest records the reserved
value `pending-canonical-fixture`, valid only in a claim whose
`FI-CONF-INTEROP-EXIT` result is `deferred`. Publication changes the element
and therefore creates a new claim.

**Request compared object.** Signature octets are excluded, because conforming
implementations need not agree on them (randomized `ES256` and fresh-aux
BIP-340 do not) and no document here pins JWS or JSON member order. The
compared object is the **signing inputs**: for each transport's Nostr proof,
the NIP-01 serialization the event id is taken over, compared against its own
transport's serialization; for the assertion, the decoded protected header and
claim set compared as JSON values with member order excluded. Every value the
compared object depends on MUST be pinned in the fixture.

The exchanged artifact per case is the complete request and response frame on
each transport — for HTTP the request line, headers, and body and the response
status, headers, and body; for Nostr the complete client and relay messages —
so that a mismatch can be explained from fields outside the compared object.

The test passes when outputs compare equal over their compared objects and
each implementation accepts the other's valid request and reproduces the
other's denials. Exit evidence includes the exchanged artifacts and each
implementation's statement of independence. A divergence traced to an
underspecified value is a defect in the specification, not in either
implementation, and is fixed there.

**Negative control.** One implementation is patched to emit a denial that
differs from the other only outside the compared object — a header core does
not name, or reordered fields — and the run MUST still pass. A run that fails
this control is comparing more than core pins; the exit test is then the
defect. The control is retained with the evidence.

`FI-CONF-INTEROP-EXIT` is REQUIRED only once a second implementation meeting
the independence conditions exists. Until then a conformance claim MUST record
it as deferred with the machine-readable reason
`no-independent-implementation`. A deferred exit test MUST be run and passed
before the second implementation's conformance claim is accepted, and the
first implementation's claim MUST be re-evidenced against that run.

## Applicability

`not-applicable` requires a machine-readable reason and behavioral proof that
the surface is absent:

- edge oracles only when no trusted-edge profile is accepted, none is
  advertised, and executable cases reject every trusted-edge evidence shape;
- snapshot-rotation oracles only when no local key or status snapshot source
  is configured and executable evidence proves the absence;
- `FI-TRACE-TOFU-THEFT` only when TOFU is neither configurable nor configured
  and executable first-use cases deny;
- `FI-TRACE-CURRENT-STATUS-STALE` and `FI-TRACE-CURRENT-STATUS-REVOKED` only
  when every configured assertion policy declares freshness class
  `offline-jwt` and executable cases prove a presented witness is never
  consulted;
- `FI-TRACE-CAPABILITY-REVOCATION` only when no external capability
  projection requiring a declared revocation bound is configured, and
  executable evidence proves no assertion capability or local-policy value
  claims such a bound;
- lifecycle and delegation oracles only when the profile is unclaimed,
  disabled, and denied on every ingress; and
- every other oracle is required for an enforcing deployment.

An implementation that supports an optional surface runs its oracles even when
one deployed domain does not activate it.

## Release gate

Before NIP-FI enforcement or discovery is enabled, reviewers verify, at one
reviewed revision, that:

- one immutable claim tuple passes every applicable oracle other than a
  validly deferred `FI-CONF-INTEROP-EXIT`;
- if the canonical fixture was published before the review, the tuple's exit
  fixture digest is not `pending-canonical-fixture`;
- the protected-ingress inventory has no uncovered or competing authority;
- every listed oracle, other than a validly deferred `FI-CONF-INTEROP-EXIT`,
  has a killed, attributed, reachable mutant and every survivor is recorded;
- the denial-fixture enumeration is complete for the claimed profiles and its
  negative control fails as required;
- the interoperability exit test has passed against an independent
  implementation, or is recorded as deferred because none exists;
- every named deployment artifact exists at the claimed deployment revision;
  and
- public and operational sinks pass privacy-canary inspection.

Documentation review, source review, and static scans are review inputs. They
close no item in this gate.

## Behavioral oracles

| ID | Required outcome |
|---|---|
| `FI-CONF-CLAIM-COMPLETE` | A report missing an applicable oracle, duplicating one, carrying a result from another claim tuple, claiming a status other than `pass`/`not-applicable` — or `deferred` on any oracle other than `FI-CONF-INTEROP-EXIT` — or omitting mutant evidence for any oracle other than a deferred `FI-CONF-INTEROP-EXIT`, or recording the exit fixture digest `pending-canonical-fixture` with any `FI-CONF-INTEROP-EXIT` result other than `deferred`, is rejected. |
| `FI-CONF-DENIAL-FIXTURES` | Every enumerated private condition has a fixture; core and each claimed profile pass exact identifier/class/owner enumeration agreement; anonymity-set responses compare byte-identical; the distinguishing negative control fails. |
| `FI-CONF-MUTATION` | Every listed oracle — except `FI-CONF-INTEROP-EXIT` while validly deferred, per **Mutation adequacy** — has a singly-applied, attributed, reachability-witnessed mutant killed by that entry's own oracle; the deny-everything and allow-everything global controls fail every oracle **Mutation adequacy** requires of them, with retained evidence; survivors are recorded, not waived. |
| `FI-CONF-INTEROP-EXIT` | Two independent implementations produce, from the documents alone, valid requests equal over the request compared object and per-class denials equal over the denial compared object, and accept each other's output. |

## Security considerations

Conformance evidence is a privileged artifact: it enumerates private denial
conditions, enrollment posture, and deployment topology that the protocol
deliberately keeps off the wire. Publishing a report, a fixture corpus, or a
mutant catalogue would disclose exactly what `FI-INV-13` and
`FI-TRACE-DISCOVERY-PRIVATE` protect.

A passing suite bounds the behaviors it exercises and nothing else. Mutation
adequacy raises the cost of a masked defect; it does not prove absence of
defects, and a claim that cites this profile as proof of security rather than
of tested behavior is misusing it.
