# NIP-BA assurance and compatibility note

Status: draft, awaiting independent redteam. Normative text is
[`../../nips/NIP-BA.md`](../../nips/NIP-BA.md). This note is evidence, not extra
normative protocol hidden to improve the word count.

## Sources and scope

This is a separate proposal on main, not a change to runtime behavior or a
claim that the existing SDK/host conforms:

- Original nine-operation prose: block/buzz#6790,
  `804ce76c167d0f435cbff65e7e3f4aad720915f6`, `docs/agent-broker.md`.
- Fifteen-operation Rust contract: block/buzz#6922,
  `115e7975a11d7d4d95043cb847abd13c1a32f370`,
  `crates/buzz-sdk/src/broker/{actions,mod.rs,wire.rs,correlate.rs}`.
- Client integration: block/buzz#6967,
  `3d2ea5b89adccf980879b9841e9a6b517b588b57`.
- Buzz-local NIPs from this branch's main base
  `7a9a5233d9d755e715be0c585cf7850e935d28cf`.

The remote-agent vision presently hands a signing key to the substrate. This
proposes optional different custody, not a replacement deployment management
plane. All durable user-visible state remains relay-scoped; the host's retry
journal is execution safety state, not an alternative source of channel truth.

## Deliberate clarifications beyond editorial compression

The wire names, argument/outcome members, 15 action versions, and numerical
limits follow #6922. The following need host/client agreement before adoption;
they are **not assertions of already-implemented behavior**:

1. Retry key includes community as well as principal. Atomic durable admission,
   in-flight joining, crash fencing, evidence-only reconciliation, and permanent
   retry tombstones close unspecified safety gaps. Availability/storage cost is
   intentional: finite retention without a new wire epoch cannot safely permit
   an old ID to execute again.
2. Authenticate/authorize release on every retry, including revoked sessions.
   A retry refusal is attempt-local and must not clear earlier uncertainty.
   This corrects the original blanket “failed means never happened” wording.
3. Specify memory core mapping to `profile`, absence/tombstones, exact compact
   size encoding, and observer partial-acceptance non-identifiability. No prefix
   guarantee was present in #6922; none is invented here.
4. Cursor filter binding/invalid-cursor refusal, thread ancestry validation,
   exact unsigned integer syntax, and current membership are explicit host
   obligations. They are not all enforced by the baseline SDK deserializer.
5. Clarify publication acknowledgement, partial lifecycle/watchdog effects,
   sorted changed-field subset, ambiguous-name refusal, and creation not implying
   a booted runtime.
6. Remove false claims that NIP-46 cannot inspect intent or that closed string
   schemas prevent all secret transmission. Storage reads intentionally return
   decrypted application content.

Remaining flexibility is explicit: deployment resource limits, initial read
window/order/cursor lifetime, runtime/provider/model defaults, provisioning and
deletion machinery, runtime telemetry body, and ownership-depth policy. This
spec does not standardize runtime-to-owner telemetry interpretation. A claim of
full cross-runtime interoperability would need that companion profile.

## Finite transition system

Run with Python 3.10+ (standard library only):

```sh
python3 docs/formal/nip-broker/model.py
```

`model.py` defines an explicit transition relation and breadth-first explores
all reachable states until a fixed point, not random traces or a depth cutoff.
A state is `(records, allowed, uncertain)`. Each record is
`(digest, phase, dispatches, effects, final)`, with final as a ghost observation
that survives result erasure. One shared request ID, two unequal body digests,
three contexts `(community,principal) = (0,0),(0,1),(1,0)` exercise collisions.
One ID is a symmetry abstraction, not proof for arbitrarily many IDs. Work on
different IDs can interact through real operation state, which is outside this
model. `effects` is a Boolean abstraction of “any effect took hold,” not the
number of relay events.

Transitions: admission, digest conflict, wait timeout, dispatch, effect,
completion, crash, reconciliation, result erasure, retry, and revocation.
Dispatch is atomically admitted once. Revocation prevents future dispatch and
stored-result release; it cannot recall already dispatched work. Reconciliation
may reveal real effects but never runs the operation again. Crashed work can
remain unknown forever. Lost responses are modeled by allowing completion to
occur without client observation and later retry; HTTP packet order is not
modeled. Reconciliation evidence is assumed truthful.

### Checked invariants and mutation witnesses

| Property | Normative seam | Deliberately broken transition |
|---|---|---|
| At most one dispatch per context/ID | Execution 2–5, retention | Concurrent redispatch, restart executor, evict protection |
| Known failure has no effects | Result and Execution 6 | Crash misreported as failure |
| Success has effect evidence | Published / completion | Baseline assertion (no separate mutant yet) |
| No cross-context stored-result release | Session / K definition | Drop community or principal |
| No release after revocation | Session / retry authorization | Replay bypasses revocation |
| Different bytes do not replay | Execution 1/3 | Ignore digest |
| Refusal retains previous uncertainty | Results, bold retry caveat | Clear uncertainty on refusal |

Nine deliberate mutations must each produce a counterexample for their expected
property; the process exits nonzero if the baseline violates an invariant or a
mutation survives. This tests model guards, **not production guards**. Claims
about production regressions require binding the actual host implementation,
which this PR does not supply.

### What this does not prove

Not a proof of JSON-parser correctness, cryptography, credential entropy, TLS,
Nostr event construction, thread ancestry resolution, pagination completeness,
NIP-AE convergence, lifecycle atomicity, or actual host behavior. Authorization
is abstracted to a Boolean; no policy engine is modeled. No fairness or liveness
claim: permanent partitions and unavailable reconciliation can remain unknown.
The at-most-once dispatch invariant does not establish exactly-once remote
effects. Multi-step operation adapters must separately establish their effect
and failure semantics. No malicious-host security theorem is possible when the
host holds the identity key.

## Upstream comparison protocol

Fixed before drafting: upstream NIPs 01, 05, 07, 09, 10, 29, 42, 44, 46, 98 at
`488b787848fcf1c6c3498c253264b8121b1a9692`. This is a purposive dependency/API/
security sample, not a random sample supporting a 90th-percentile assertion.

Dimensions: normative precision; independent implementability; failure/retry
clarity; security boundaries; reproducible verification; economy of expression.
Each gets 0 absent, 1 major gaps, 2 usable with questions, 3 explicit adequate,
4 unusually strong. N/A must not be scored zero. Independent review must cite
evidence and may reject the rubric. No averages may hide a weaker dimension.
Strictly better on every dimension requires every score to exceed the
comparator's; Pareto superiority (no weaker, at least one stronger) is a
separate, weaker criterion. Adoption, deployed interoperability, and ecosystem
maturity are additional dimensions that an unimplemented draft cannot beat by
editing prose. No “superior to 9/10” conclusion is asserted here.


## Executable wire examples

```sh
python3 -m unittest discover -s docs/formal/nip-broker -v
```

`vectors.json` contains a request and corresponding result for each of the 15
actions. `wire.py` is a dependency-free, independently written **partial wire
oracle**, not a production implementation or a full conformance certificate.
It checks closed shapes, scalar normalization, integer widths, UTF-8, identity
syntax, size limits, status/code pairs and selected response correlations.
`test_wire.py` systematically inserts unknown/null/duplicate members into the
example objects and exercises boundaries and byte-distinct equivalent JSON.
Example event IDs and d-tags are illustrative, not cryptographic vectors; the
checker does not verify signatures,
read-filter membership, ancestry, address derivation, or operation execution.

A deliberately bypassed closed-object guard admits a forbidden `scope` member;
the normal guard rejects it. An encoding regression was reproduced before the
fix: Python's JSON decoder accepted UTF-16 input. The oracle now decodes bytes
as UTF-8 explicitly. Neither experiment establishes a production SDK regression.

## Comparison findings (author assessment; independent review pending)

The fixed sample is not exchangeable: a browser capability, an event deletion
request and a distributed action service solve different problems. Absence of
an execution journal in a signature or serialization standard is **not a defect**.
Consequently, numerical totals would reward our chosen problem and hide tradeoffs.
The six dimensions above are inspection questions, not measured universal ranks.
These are concrete strengths to preserve or learn from, and limits to our claim:

| NIP at the pinned revision | Evidence in that document | NIP-BA comparison boundary |
|---|---|---|
| [01][n01] | Events/signatures specifies serialization; relay flow defines OK, CLOSED, EOSE and tie ordering | We add operation uncertainty, but depend on its event machinery; not strictly more precise in every dimension |
| [05][n05] | Security Constraints forbids redirects; Notes distinguishes identification from verification and preserves pubkey identity across remapping | Comparable explicit trust boundary; much narrower and economical protocol |
| [07][n07] | Two required browser methods, optional encryption methods, extension timing and implementation link | Our failure contract is fuller; its tiny API is easier to implement and explain for its task |
| [09][n09] | Client Usage requires author matching; warns deletion cannot be guaranteed; deleting a deletion has no effect | Both explicitly bound promises; no reason to demand an action journal from a deletion-request event |
| [10][n10] | Marked e tags distinguish parent/root and document legacy ambiguity | We reuse this idea rather than surpass it; its kind-1 scope is not our kind-9 profile |
| [29][n29] | Relay-scoped group identity, forks/migrations, independent subgroup membership, reconstruction events | Our retry namespace is explicit, but UUID-only actions and out-of-band provisioning are less general |
| [42][n42] | Connection-scoped challenges, request retry examples, auth-required/restricted distinction | We add revoked-result rules; our bearer provisioning is less specified than its challenge exchange |
| [44][n44] | Limitations, exact algorithm/pseudocode, external audit, published positive/negative vectors | A finite retry model is not a cryptographic audit; no verification superiority claimed |
| [46][n46] | Two connection flows, permission requests, secret validation, logout limitations, auth challenge examples | We add reads/execution safety; it specifies discovery and connection establishment that we leave out of band |
| [98][n98] | URL/method/time checks, optional body binding, wire example and reference implementation | We require retry-byte identity but solve a different authentication problem and are less compact |

NIP-BA's current strengths are explicit attempt-local failure semantics, durable
retry protection, a small executable safety model with negative witnesses, and
all-action wire examples. Its weaknesses remain companion runtime telemetry
semantics, out-of-band provisioning/policy, unbounded lifetime journal growth
controlled only by admission quotas, no production host, and no independent
client/host interoperability run. Economy must be judged relative to scope,
not by rewarding the document with the fewest absolute words.

**The requested “superior in all dimensions to at least 9/10” target is not
established.** No editing-only stopping rule can supply deployment maturity or
an independent implementation. This proposal should be judged on an auditable
contract and resolved review findings, not a fabricated league table.

[n01]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/01.md

[n05]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/05.md

[n07]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/07.md

[n09]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/09.md

[n10]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/10.md

[n29]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/29.md

[n42]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/42.md

[n44]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/44.md

[n46]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/46.md

[n98]: https://github.com/nostr-protocol/nips/blob/488b787848fcf1c6c3498c253264b8121b1a9692/98.md


## Reproduction result before independent review (2026-09-03)

Python 3.14, three-context baseline: **1,481,544 reachable states and 16,102,044
transitions**, exhausted with no invariant violation. All nine mutations yielded
a counterexample for their designated property. Wire suite: **9 test groups
passed**, including positive request/result vectors for every action.
Model source SHA-256: `0f87f39917e044c4767be3d9a75fe1aed65325be9316dbebdab7c6847c7447a5`.
Rerun the commands above; model stdout contains the complete mutation traces.

`just ci` was attempted on base `7a9a5233d9d755e715be0c585cf7850e935d28cf`
with only these documentation/model additions. The initial attempt timed out;
a subsequent invocation selected an old PATH compiler and failed the MSRV check.
With the installed Rust 1.95.0 toolchain selected explicitly, repository checks
and the full 463-test CLI package passed, but `test-unit` stopped in buzz-acp at
`acp::tests::keepalive_resets_idle_past_deadline` (82 passed, one failed before
fail-fast). Later CI stages did not run. **Full repository CI is not green**;
this note does not diagnose that failure as a flake or as caused by this change.

Independent redteam requested from Eva; findings and disposition remain pending.
This is a reviewable draft, not approval of a production broker.
