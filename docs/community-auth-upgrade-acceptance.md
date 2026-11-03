# Community authentication upgrade acceptance

Status: **procedure draft; live acceptance has not run**. This is the evidence
procedure for BUZZ-516. It does not establish that the upgrade implementation,
staging configuration, candidate artifacts, or failure-injection controls exist.

The test succeeds only when an existing community changes its authentication
requirement without changing its identity or another community's availability,
and operation readback reports completion after actual target drain.

## Prerequisites and test ownership

Resolve these gates before executing the corresponding scenario. Record a PR,
exact head, test name/command, and result for each prerequisite in the run record.
An open PR or a source configuration pin is not running-artifact evidence.

| Gate | Owning work | Evidence needed |
| --- | --- | --- |
| Durable enable and status | BUZZ-510 | Reviewed request/response contract; atomic identity, owner, lifecycle and issuer validation; idempotency and recovery tests |
| Admission and discovery | BUZZ-511 | Host-specific requirement; protected route inventory; mixed free/required and global-enforce tests |
| Existing sessions and activation | BUZZ-512 | Serving-incarnation inventory; real drain and effect barriers; generation-bound acknowledgements; documented retirement proof |
| Authenticated module | BUZZ-513 | Module authorization, response validation, fresh request signatures, retry and status tests |
| Isolated staging fixtures | BUZZ-514 | Verified runtime trust/configuration; exact candidate artifacts; target and control allowlist |
| One-shot harness | BUZZ-515 | Normal injection test; bounded authenticated enable/status entry points; approved rollout procedure |

Reuse the owning tests. Add cross-component failures here only when no owning
test exercises the same production path. Do not copy their fixtures into a new
mock implementation of the upgrade protocol. Name uncovered acceptance rows as
blocked until executable coverage and live observations are available.

Existing FI regressions to retain include:

| Source | Existing coverage; limits |
| --- | --- |
| `crates/buzz-auth/src/nip_fi/verifier/tests.rs` | Assertion validation; does not prove per-community upgrades |
| `crates/buzz-relay/src/nip_fi_http.rs` | HTTP evidence classes, key mismatch, repair precedence and legacy responses |
| `crates/buzz-relay/src/nip_fi_upgrade.rs` | WebSocket assertion admission; this filename does not mean community policy upgrade |
| `crates/buzz-relay/src/nip_fi_session.rs` | Pairing and expiry, including saturated control queues |
| `crates/buzz-relay/src/nip_fi_gate.rs` | Cancellation and effect quiescence |
| `crates/buzz-relay/src/nip_fi_resolver/tests/router_tests.rs` | Router-level host binding and stale-result regressions; some successes only reach the next failing dependency |
| `crates/buzz-db/src/store/nip_fi_binding.rs` | Issuer binding transactions and lifecycle races |

These files are a source inventory, not a claim that any test passed. Follow
[TESTING.md](../TESTING.md) for infrastructure and isolated local relay setup.
Activate Hermit before checks. Run `just ci`; run `just test` for relay/auth/DB
changes. The ignored `buzz-test-client` suites additionally need a running relay;
`just test` does not execute them. Record full package results, not only filtered
test passes. Use each cash-server package's own required checks for module and
harness coverage.

## Run record and fixture preparation

Create one run record with a unique run ID, UTC start/end, operator, independent
reviewer, and result (`blocked`, `failed`, or `passed`). Preserve redacted evidence
outside this repository. Record the exact command and exit status of every
check, the tested Git HEAD, and any working-tree changes. Never include raw
credentials, assertions, session cookies, private keys or mounted secret values.

Before mutations, capture:

1. Confirmed cluster, namespace, deployment, service routing and fixture allowlist.
   Record running image digests and source commits for every relay and harness
   artifact, schema version, nonsecret effective configuration and secret version
   references. Compare actual runtime state with the reviewed rollout plan.
2. Every serving process's deployment identity, pod UID, boot/incarnation ID,
   image and admission state. Include processes with established sockets even
   after readiness removal. Prove old binaries that ignore policy cannot serve.
   Record the deployment's audited baseline epoch and its enforcement gate;
   empty registration alone cannot prove fleet coverage. Without a valid baseline,
   activation must stay pending or enable must refuse before acceptance.
3. Target immutable community ID and canonical host, owner public key, registered
   actor public key, members/roles and saved community identity. Use public-key
   equality and a signed challenge to prove key preservation; never export keys.
4. Distinct free controls exercised with and without otherwise valid FI evidence,
   and an already-enforced control. Record their existing admission behavior,
   sessions, subscriptions and public-key/membership snapshots.
5. Target issuer binding and trusted issuer configuration. Keep identity origin,
   JWT issuer and assertion/adapter endpoints distinct; take their exact values
   from the reviewed BUZZ-514 profile. Provision trust separately from upgrade.
6. The BUZZ-514 login probe with a well-formed S256 challenge and approved loopback
   return URI; capture status/cache headers without callback secrets. A bare
   login GET returning 400 is not an activation test.
7. A legacy target root/audio session and a subscription on **each** serving
   incarnation. Establish them before enable, prove they carry fixture traffic,
   and keep per-connection receive traces. Pin traffic to individual processes
   while preserving canonical Host, SNI, NIP-98 URL and NIP-42 relay binding.

Do not infer process coverage from repeated requests through a load balancer.
Do not treat unavailable audio or Git fixtures as successful denials. Each
protected route needs a positive control proving its normal authenticated path
works and a negative proving the FI gate, rather than a later 404/500, rejected it.

## Acceptance scenarios

Run one scenario at a time. Use a new isolated target for independent upgrade
experiments because the requirement is monotonic. Retry/recovery scenarios reuse
the original operation ID and target. Recheck free and enforced controls before,
during and after every injected failure, including traffic on existing sockets.

| ID | Stimulus | Required observation |
| --- | --- | --- |
| A1 | Invoke the authenticated module through the normal BUZZ-515 graph with the owner fixture | Durable tuple identifies the requested community, actor, operation and generation; saved policy is distinct from active |
| A2 | Rediscover and reconnect with valid FI using the original Nostr key | Root and `/info` agree on requirement/capability/descriptor, are `no-store`, and authenticated access works; keys and membership match the baseline |
| A3 | Omit FI; then use invalid signature, issuer, audience, expired assertion and wrong key | Expected 401/403 refusals across protected HTTP, bridge, root/audio WS, Git and applicable V1/media routes; no admitted effect or delivery |
| A4 | Reconnect, rediscover and renew assertions twice | Three distinct admission/renewal observations with issue/expiry times and unchanged public key; prove traffic after each renewal |
| A5 | Attempt wrong-key identity binding, then use the original key | `binding_mismatch`; original binding unchanged and original identity still works |
| A6 | Exercise both free controls and already-enforced control | Their pre-upgrade admission, delivery and membership behavior remains unchanged, including free access with and without FI |
| B1 | Nonowner, unauthorized capability/standing, missing or expired session, unusable/unregistered actor key | Refused before policy mutation; durable state and membership unchanged |
| B2 | Wrong immutable ID/host pair; missing, inactive or mismatched issuer; unreadable trust | Refused or unavailable, with no implicit trust provisioning and no upgrade side effect |
| B3 | Race enable against ownership transfer, archive and deletion separately | Linearized valid outcome; losing request cannot mutate the wrong owner/lifecycle; readback matches durable state |
| C1 | Repeat identical operation and payload; then change one bound field | Same tuple recovered; changed payload conflicts without a second generation |
| C2 | Concurrent identical and distinct operation IDs | Contract-defined convergence/conflict; no duplicate upgrade or mixed actor/community tuple |
| C3 | Drop response after commit; retry; restart kgoose and request status | Recover the original tuple/generation without clearing policy or inferring acceptance from timeout |
| D1 | Drop notifications to one serving incarnation | Durable reconciliation applies target policy and actually drains its sessions; pending persists until its obligation is satisfied |
| D2 | Race registration, enable and completion; restart/replace processes | No unregistered serving window; a prior boot's acknowledgement cannot discharge a new boot's obligation |
| D3 | Pause a process with live target sockets across enable; resume it | Pending while unresolved; first resumed admission/effect/delivery cannot use old policy; active requires real quiescence |
| D4 | Hold a policy read, advance generation, then release the stale response | Late response cannot overwrite the required floor or authorize an effect; readback retains the newer generation |
| D5 | Isolate a serving process from policy DB while retaining sockets | Activation stays pending/unavailable as appropriate; target cannot falsely report drained; unrelated free controls retain baseline availability |
| D6 | Hold HTTP effects, output queues, subscriptions, audio and pending handshakes across enable | Completion waits for actual target teardown/effect barriers, not queued cancellation; valid FI reconnect remains possible |
| D7 | Crash before/after drain and before/after acknowledgement persistence | Durable recovery accounts for each boot; replacement/readiness/heartbeat expiry alone never retires the old obligation |
| D8 | Attempt completion with an absent/invalid serving baseline; register before enable but open the listener afterward | No false active from an empty registry; startup, participant capture and serving transition share an ordering |

For A3, record the exact request/route and denial body from BUZZ-511's contract.
Unavailable verification/policy dependencies must fail closed on required hosts;
repair `deny_protected` retains 503 precedence. Repeat relevant cases in off,
shadow and enforce configurations using isolated fixtures. Off/shadow must not
disable a stored requirement, and global enforce remains an enforcement floor.
Discovery for unknown/non-serving hosts is generic 404; unreadable policy is 503.

## Failure injection and recovery

Before each injection, record its target boot, affected connection, operation and
generation, the exact injector command, its scope and its inverse. Use isolated
test infrastructure for shared DB/Redis faults unless staging has a reviewed way
to constrain the fault. Do not change whole-fleet policy or drain unrelated
communities as a fallback. If an injector cannot preserve these boundaries,
record the scenario blocked and use a deterministic local integration fixture.

Synchronize race tests on observed production boundaries (transaction commit,
participant registration, held DB response, effect permit, drain barrier), not
arbitrary delays. Repeat with both orderings and retain the histories. A pause
longer than an assumed timer does not prove a barrier. Distinguish process pause
from host suspend if the implementation depends on a clock's behavior.

On timeout or uncertainty, retain the same operation ID and monotonic policy.
Restore the injected dependency and invoke authenticated status/retry through
the harness. Never edit policy, generation, participant or acknowledgement rows
to manufacture activation. If a dead boot cannot acknowledge, use only BUZZ-512's
reviewed permanent-non-serving recovery procedure and retain its audited proof.
An absent pod, a replacement, a readiness change and elapsed time are insufficient.

## Completion evidence

For every operation retain its community/host, actor, operation ID, policy
generation, commit confirmation and ordered status responses. Preserve the full
participant set and its changes, and for every deployment/boot/generation tuple:

- Last legacy permit/effect and last queued/delivered target output.
- Applied generation, socket/handler/subscription/audio teardown and barrier
  completion, with observer and process identity attached.
- Ownership of old-generation HTTP/Git/media/V1 response production and detached
  child tasks through their exits, including cancellation, drop and error paths.
  New valid-FI work must not hold the old generation's drain barrier open.
- Durable applied-and-drained acknowledgement or reviewed permanent-non-serving
  evidence; include its generation and boot identity.
- Post-activation per-process proofless rejection and valid-FI admission, with
  positive route controls and unchanged free/enforced control observations.

Correlate server evidence with independent client receive traces. Explain clock
sources and uncertainty; use causal boundaries where clocks are not comparable.
Drain proves that old-generation work cannot acquire another permit or enqueue
protected output and all tracked work has exited. It does not prove that a client
has consumed bytes sent before the barrier; distinguish those bytes in the trace.
Report normal-case commit-to-drain and commit-to-active durations for each run,
with sample count and observed range. Report partition duration and pending
observations separately; do not claim a partition completion bound.

A run passes only when every applicable matrix row has executable test coverage
and live evidence, all process obligations are accounted for, package checks
pass and an independent reviewer has reviewed the exact auth/persistence/
activation heads. A passing module invocation proves module-to-relay integration;
it does not prove an HTTP call through a running kgoose pod or V0/V1 onboarding.
Keep those acceptance claims separate.

Leave BUZZ-516 open when prerequisites, route fixtures, fault controls, checks,
independent review or live observations are missing. Report each gap, its owner
and the concrete next action. A drafted procedure or green unit suite is not
live acceptance. This procedure authorizes no deployment or production rollout.
