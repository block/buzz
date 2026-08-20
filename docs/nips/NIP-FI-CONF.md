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

This profile is separately claimable and is never advertised in discovery.
Conformance is a property of a reviewed revision, not a wire feature, and a
public claim of it would be an unverifiable assertion about the server's own
testing.

This profile defines no wire behavior, denial mapping, invariant, or admission
rule. Where it names one, NIP-FI core or the owning profile is normative.

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
carried into another. The governing document revision and exit fixture digest
are in the tuple because the same evidence means different things under
different text: a claim that does not name the revision it was judged against
is unfalsifiable once the specification moves, and a suite that does not name
its fixture cannot be shown to have run the pinned inputs. A report contains
every applicable oracle from core and every claimed profile exactly once, with
status `pass` or `not-applicable` only, except that `FI-CONF-INTEROP-EXIT`
alone may instead carry `deferred` under the condition defined in
**Interoperability exit test**. Blank, skipped, expected-failure, and
not-run results cannot support a claim (`FI-CONF-CLAIM-COMPLETE`).

Enrollment mode is part of the claim unit and is private. It is recorded in the
access-controlled report, never in discovery or any public artifact.

## Evidence rules

Each passing oracle records the claim tuple, a stable test identifier and
adapter entry point, the command with start time, end time, exit status, and
any random seed, the synthetic input or a privacy-safe digest of it, the
before-and-after authoritative state relevant to the oracle, the expected
outcome and the observed outcome, and artifact locations with SHA-256 digests.
Stateful oracles use an isolated database or namespace and inspect committed
state rather than inferring it from a response. Concurrency oracles record
every contender and the single serialized outcome. Time-boundary oracles use a
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
only. Under the private-posture rule no discovery output distinguishes
enrollment mode, so a discovery witness for that oracle cannot exist; requiring
one would make the oracle unsatisfiable. Discovery invariance is proved
separately by `FI-TRACE-DISCOVERY-PRIVATE`, which compares complete discovery
bytes across enrollment modes.

Deployment-obligation requirements — those marked in core or a profile as
`[deployment artifact: ...]` — are evidenced by the named access-controlled
review record at the claimed deployment revision, not by a behavioral oracle.
A claim listing an artifact without the record is incomplete.

Reports and artifacts hold private deployment detail and MUST remain access
controlled. They MUST NOT enter public reports, examples, discovery, or
protocol output, and MUST NOT contain raw assertions, secrets, or unredacted
`iss`, `sub`, or claim values. The shared exit fixture is exempt: its values are
synthetic by construction and name no real principal, issuer, or key, and the
interoperability exit test cannot run on redacted inputs.

## Denial fixtures

`FI-TRACE-DENIAL-ORACLE` requires one fixture per **private condition**, not
one per public class. A per-class suite passes trivially: it compares a class
against itself. The enumeration below is the required fixture set
(`FI-CONF-DENIAL-FIXTURES`). Its public-class column restates NIP-FI core,
which owns that mapping and the exact response bytes.

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

The names in the private-condition column are fixture identifiers for this
enumeration. Some are the symbols core's preparation pseudocode denies by name;
that set is read from core, never restated here, because a copied list is state
that drifts and the whole subject of this section is that unguarded restatements
rot. `policy_denied` and `dependency_unreadable` are core's conditions expressed
only in prose — a bare policy denial and `FI-INV-14` fail-closed — and core is
not required to name them symbolically; they are the only two core rows so
exempted. The remaining rows name conditions in the owning profiles' literal
private-denial-condition tables. None is a wire value, and a deployment MAY use
different private reason codes
internally as long as every enumerated condition has a fixture.

Every row whose public class is `authorization_denied` is in the private-state
anonymity set. Their public responses MUST compare byte-identical to each other,
not merely equal in prefix or status.
Rows for an unclaimed profile are `not-applicable` with absence evidence. A
profile that introduces a new private condition MUST add its row; an
unenumerated condition escapes this oracle entirely.

The anonymity comparison is **wider than the interoperability compared object
defined below, and deliberately so**. Between two private conditions on the same
implementation, every response byte as transmitted MUST agree — including
transfer framing, and not only the content — except values a server cannot
hold constant across two instants, such as `Date`. It is not limited to the
header fields core names. The narrower object below exists because two
*different* implementations cannot be required to agree on fields core does not
pin; that reasoning does not apply within one implementation, where any field
varying by private condition is a disclosure whatever its name. A suite that
reuses the interoperability object here would pass an implementation that
returns its private reason code in an unnamed header, or one that varies its
chunk boundaries by private condition.

**Enumeration agreement.** The preceding paragraph makes a quantified claim
about this table, and a fix verified against the one row it changes can still
falsify it. `policy_denied` and `dependency_unreadable` are the **prose-only
allowlist**: core's conditions that core is not required to name symbolically.
Both sets here are named by symbol, never by row number, because this table is
required to grow and every positional reference silently retargets when it does.
The suite MUST check, mechanically at the claimed head
(`FI-CONF-DENIAL-FIXTURES`):

1. every symbol core denies by name has a row here, attributed to core;
2. every symbol core denies by name carries the same public class — quantified
   over core's symbolic set, not over all core-attributed rows, since
   `dependency_unreadable` is core-attributed and correctly
   `authorization_unavailable`; and
3. the set of symbols named by core-attributed rows equals core's symbolic
   denial set together with the allowlist, exactly, and the allowlist is
   disjoint from core's symbolic denial set; and
4. for each claimed profile that owns a literal private-denial-condition table,
   the set of `(identifier, public class)` pairs in that table equals the set of
   pairs in rows attributed to that profile here, exactly. A row with multiple
   profile owners contributes its pair to each owner named in that cell.

Every check above MUST be run against the unmutated document and be green before
any mutant is scored. A check that is red on a conforming document detects
nothing: it cannot be observed to flip, so every mutant reads as caught. Two of
these checks shipped red for exactly that reason.

Checks 3 and 4 keep copied enumerations honest in both directions. Check 4
catches a profile condition renamed, added, removed, reclassified, or attributed
to the wrong owner without the matching fixture-table change. It is run for each
claimed owning profile. Rows for an unclaimed profile remain `not-applicable`.

Check 3 keeps the core allowlist honest in both directions. Its equality half
catches
a core-attributed row core never emits. Its disjointness half is what makes
promotion visible: if a later core turns `policy_denied` or
`dependency_unreadable` into a named symbol, every other check still passes —
the row is already attributed to core — and only disjointness fails, leaving the
stale allowlist entry as the thing to delete. Promotion MUST NOT be applied to
`dependency_unreadable` without moving it out of the set check 2 quantifies
over; it is the one core condition whose public class differs.

**Compared object.** This object governs the interoperability comparison between
two implementations. The anonymity comparison above is wider. Byte-identity is
asserted over the response bytes an implementation chooses, which excludes bytes
a conforming HTTP server cannot hold constant. Over Nostr the compared object is
the complete relay message excluding only the event or subscription identifier
echoed from the request, encoded as compact JSON with no insignificant
whitespace, per NIP-01's serialization rules. Over HTTP the compared object is
exactly what NIP-FI core pins: the status code, the complete body, and the exact
values of only the header fields core's denial table names. Header field *names*
are matched case-insensitively per RFC 9110 Section 5.1; their values are
compared exactly. The compared body is the *content* per RFC 9110 Section 6.4 —
after transfer-decoding, chunk framing and trailer fields excluded — not the
message body on the wire. That reading is scoped to this interoperability object
and does not reach the anonymity comparison above, which stays over transmitted
octets. `Content-Length` is deliberately not pinned: framing is the sender's
choice and pinning it would widen the object for no privacy gain.
Header order and unnamed header fields are outside it, and their values MUST NOT
depend on the private condition — which the anonymity requirement above already
demands and tests directly.

Each reading is stated because an independently written conforming
implementation diverges on it by language default, not by error: a canonicalizing
HTTP library emits `Www-Authenticate`, a server that sets no `Content-Length`
frames the body as chunked, a JSON encoder inserts spaces after `,` and `:`, and
two servers emit different automatic fields — `Server`, `Connection` — in
different orders. Comparing those would fail every conforming pair, and an
oracle no conforming implementation can satisfy is a defect in this document
rather than evidence about either implementation. The compared object is
therefore closed over what core names and nothing more; if core later pins an
additional field, it joins with no edit here. `Date` needs no special exclusion,
since core does not name it. Any field an implementation must exclude despite
core naming it MUST be reported with the reason it cannot be held constant, and
its value MUST be independent of the private condition.

The oracle runs a fixed positive iteration count on a pinned isolated runner at
the exact claimed head. Before the run the operator records the environment,
public-response corpus, bounds, sampling method, statistical rule, noise
treatment, and acceptance threshold. A breach fails the gate, MUST NOT trigger
an automatic retry, and is retained and investigated before a separately
authorized rerun.

`authorization_unavailable` is observably distinct from `authorization_denied`.
This is accepted residual: it discloses no per-principal state, and collapsing
it would make fail-closed behavior undiagnosable.

The suite MUST include a negative control: an implementation deliberately
patched to vary its denial response by private condition MUST fail this oracle.
Without it the suite asserts that it works instead of demonstrating it.

## Mutation adequacy

Naming an oracle for a requirement proves the requirement is claimed, not that
the oracle can fail. A requirement whose oracle cannot fail is untested and
reads as tested, which is worse than an acknowledged gap.

The denominator is the **listed oracle**. An oracle table is identified by its
rows: each names exactly one complete literal oracle identifier in its first
cell, with no shorthand and no name left to inference. The denominator is every
such row in NIP-FI core, in each claimed normative profile, and in this document
when CONF is claimed — selected by that first cell, not by section title, since
the tables do not share one. Enumerating it is reading rows, so two readers
obtain the same set. It is not the set of normative sentences, RFC 2119
keywords, or invariant labels: none is enumerable without judgement, and a
denominator two readers count differently decides how much of the specification
is tested at all.

For each listed oracle in core and each claimed profile, the suite MUST retain at
least one **mutant**: an implementation variant that violates a requirement that
oracle governs, together with that oracle's failing output (`FI-CONF-MUTATION`).
Evidence is the exact patch identity, the oracle identifier, and the retained
failure output at the claimed head.

While `FI-CONF-INTEROP-EXIT` is validly deferred under **Interoperability exit
test**, it remains in claim completeness but is excluded from the mutation and
global-control obligations of this section: its failing output cannot exist
without the run itself. Both obligations attach with the run and MUST be
discharged before either implementation's interoperable conformance claim is
accepted. No other oracle's obligation under this section is deferrable.

Normative prose outside the oracle tables remains binding. It is not a second
denominator. Prose that no listed oracle can detect is untestable text: add the
oracle that detects it, or delete it.

Five rules make the mutant meaningful:

1. **One at a time.** Mutants are applied singly against an otherwise unmodified
   implementation. Layered defenses mask each other: a guard looks covered
   because a different guard denies first.
2. **Attribution.** The kill MUST come from the entry's own oracle. A mutant
   killed only by some other oracle establishes coverage for neither.
3. **One entry per mutant.** A mutant satisfies only the entry it was selected
   for, even when it also kills other oracles. Otherwise one broad mutant
   discharges several obligations at once and the count reads complete while
   the coverage is not.
4. **Reachability.** The suite MUST witness that a fixture reaches the mutated
   decision, not merely the enclosing operation. A mutant behind a bound,
   length field, or earlier denial that no fixture ever passes is never
   exercised, and the suite reports clean on an implementation that is
   provably broken.
5. **Survivors are recorded.** A mutant its named oracle fails to kill is a
   defect in the specification or the suite. It is recorded with that
   disposition and MUST NOT be waived or replaced by an easier mutant.

Two global controls bound the suite from both sides. A deny-everything
implementation MUST fail every positive oracle, proving each oracle has a
positive arm. An allow-everything implementation MUST fail every negative
oracle, proving each has a negative arm. Neither control substitutes for
per-entry mutants; an implementation can pass both while violating any
individual requirement.

## Interoperability exit test

A claim of core conformance requires evidence that the document alone is
sufficient to build against (`FI-CONF-INTEROP-EXIT`). Two implementations that
have not shared code and have not consulted a common reference implementation
each produce, from NIP-FI core and any claimed profile documents alone:

- one valid `client-attached` request, over WebSocket upgrade and over HTTP,
  compared over its signing inputs as defined below; and
- one byte-exact public denial response for each of the four public classes, on
  both transports, compared over the object defined under **Denial fixtures**.

Independence is a claim about code and reference implementations, not about
inputs. Two implementations given different issuers, keys, or clocks cannot
produce equal bytes however correct both are, so the run is parameterized by a
**shared exit fixture** that both sides load and neither side authors:

- one issuer identity and one JWK set, including the private key needed to mint
  assertions and the `kid` selecting it;
- one assertion per denial class and one for the valid request, each expressed
  as complete pre-signature protected-header and claim-set JSON values —
  including `alg`, `typ`, `kid`, and every member the policy allows, and fixed
  `iss`, `sub`, `aud`, `nostr_pubkey`, `client_id`, `iat`, `exp`, and token
  class;
- one Nostr secret key for the proof, with the complete unsigned event fields
  for each transport — the NIP-98 event over HTTP and the NIP-42 event with its
  challenge and relay values over the WebSocket upgrade — including
  `created_at`, so both sides derive the same actor and the same event id;
- one frozen evaluation instant, and the skew and lifetime bounds in force; and
- the domain, target resource, operation, and enrollment policy for each case.

The canonical shared exit fixture is authored by this document's editors, not
by any claiming implementation, and MUST be published as a single file in the
same repository as these documents, at `docs/nips/fixtures/nip-fi-conf-exit.json`,
together with its SHA-256 digest, before any `FI-CONF-INTEROP-EXIT` run. Both
sides MUST load that file, MUST verify
the digest before the run, and MUST record the digest with the evidence. A run
against any other fixture instance is not `FI-CONF-INTEROP-EXIT` evidence.
While the canonical fixture file is not yet published, the exit fixture digest
element of the claim tuple records the reserved value
`pending-canonical-fixture`; that value is valid only in a claim whose
`FI-CONF-INTEROP-EXIT` result is `deferred`. Publication of the fixture changes
the element and therefore creates a new claim.

**Request compared object.** A minted request cannot be compared as bytes. Two
conforming implementations may produce different signature octets for the same
semantic input — randomized `ES256` and BIP-340 with fresh auxiliary input do,
while deterministic ECDSA and fixed-aux BIP-340 need not — and no document here
pins JWS or JSON member order. An object requiring equal octets would therefore
fail conforming pairs, which is the defect this document names one layer down.
The compared object is the **signing inputs**:

- for a NIP-98 proof over HTTP and a NIP-42 proof over the WebSocket upgrade,
  the NIP-01 serialization the event id is taken over, which NIP-01 fixes
  byte-for-byte; each transport's proof is compared against its own transport's
  serialization, not across transports; and
- for the assertion, the decoded protected header and claim set compared as
  JSON values, with member order excluded.

Signature octets are excluded because conforming implementations need not agree
on them, not because they disagree about this document; the mutual-acceptance
requirement below tests them instead. A divergence in a signing input is a
divergence in what this document told each side to sign, which is what the exit
test exists to detect.

Every value the compared object depends on MUST be pinned here. A value left to
the implementation is a divergence the test will attribute to a defect in this
document, which is the correct disposition but a slow way to discover a missing
fixture field.

The exchanged artifact per case is the complete request frame and the complete
response frame on each transport: for HTTP the request line, headers and body,
and the response status, headers and body; for Nostr the complete client
message and the complete relay message. Both sides emit whole frames even
though the compared objects are narrower, because the unnamed fields are what
the reader needs in order to explain a mismatch.

The evidence is the produced bytes and a statement of independence; the claim
tuple already pins the fixture and the document revision. The test passes when
the outputs compare equal over their compared objects and each implementation
accepts the other's valid request and reproduces the other's denials. Any
divergence traced to an underspecified value is a defect in the specification,
not in either implementation, and is fixed there.

This test has a mandatory negative control. One implementation is patched to
emit a denial that differs from the other only outside the compared object —
adding a header core does not name, or reordering fields — and the run MUST
still pass. A run that fails this control is comparing more than core pins and
would reject conforming pairs; the exit test itself is then the defect. The
control is retained with the evidence, because a comparison that only ever
reports equality proves nothing about what it would have caught.

`FI-CONF-INTEROP-EXIT` is REQUIRED only once a second implementation meeting
the independence conditions above exists. Until one exists the test cannot be
run, and a conformance claim MUST instead record it as deferred with the
machine-readable reason `no-independent-implementation`. A deferred exit test
MUST be run and passed before the second implementation's conformance claim is
accepted, and the first implementation's claim MUST be re-evidenced against
that run.

## Applicability

`not-applicable` requires a machine-readable reason and behavioral proof that
the surface is absent:

- edge oracles only when no trusted-edge profile is accepted, none is
  advertised, and executable cases reject every trusted-edge evidence shape;
- snapshot-rotation oracles only when no local key or status snapshot source is
  configured and executable evidence proves the absence;
- `FI-TRACE-TOFU-THEFT` only when TOFU is neither configurable nor configured
  and executable first-use cases deny;
- `FI-TRACE-CURRENT-STATUS-STALE` and `FI-TRACE-CURRENT-STATUS-REVOKED` only
  when every configured assertion policy declares freshness class
  `offline-jwt`, so no status witness exists to be stale or revoked, and
  executable cases prove a presented witness is never consulted;
- `FI-TRACE-CAPABILITY-REVOCATION` only when no external capability projection
  requiring a declared revocation bound is configured, and executable evidence
  proves no assertion capability or local-policy value claims such a bound;
- lifecycle and delegation oracles only when the profile is unclaimed, disabled,
  and denied on every ingress; and
- every other oracle is required for an enforcing deployment.

An implementation that supports an optional surface runs its oracles even when
one deployed domain does not activate it.

## Release gate

Before NIP-FI enforcement or discovery is enabled, reviewers verify that one
immutable claim tuple passes every applicable oracle, other than a validly
deferred `FI-CONF-INTEROP-EXIT`, at one reviewed revision;
that the protected-ingress inventory has no uncovered or competing authority;
that every listed oracle, other than a validly deferred `FI-CONF-INTEROP-EXIT`
under **Mutation adequacy**, has a killed, attributed, reachable mutant and every
survivor is recorded; that the denial-fixture enumeration is complete for the
claimed profiles and its negative control fails as required; that the
interoperability exit test has passed against an independent implementation, or
is recorded as deferred under **Interoperability exit test** because no
independent implementation exists;
that every named deployment artifact exists at the claimed deployment revision;
and that public and operational sinks pass privacy-canary inspection.

Documentation review, source review, and static scans are useful review inputs.
They close no item in this gate.

## Behavioral oracles

| ID | Required outcome |
|---|---|
| `FI-CONF-CLAIM-COMPLETE` | A report missing an applicable oracle, duplicating one, carrying a result from another claim tuple, claiming a status other than `pass`/`not-applicable` — or `deferred` on any oracle other than `FI-CONF-INTEROP-EXIT` — omitting mutant evidence for any oracle other than a deferred `FI-CONF-INTEROP-EXIT`, or recording the exit fixture digest `pending-canonical-fixture` with any `FI-CONF-INTEROP-EXIT` result other than `deferred`, is rejected. |
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
