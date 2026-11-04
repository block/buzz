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
| Durable enable and status | BUZZ-510, merged as #8985 | Current-main [operator contract](community-auth-upgrades.md); atomic identity, owner, lifecycle and issuer validation; idempotency and recovery tests |
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
| `crates/buzz-db/src/store/community_auth/postgres_tests.rs` | Durable acceptance, retries, conflicts and transaction-lock races from merged BUZZ-510 |

These files are a source inventory, not a claim that any test passed. Follow
[TESTING.md](../TESTING.md) for infrastructure and isolated local relay setup.
Activate Hermit before checks. Run `just ci`; run `just test` for relay/auth/DB
changes. The ignored `buzz-test-client` suites additionally need a running relay;
`just test` does not execute them. Record full package results, not only filtered
test passes. Use each cash-server package's own required checks for module and
harness coverage.

The Postgres-backed cases marked `#[ignore = "requires Postgres"]` also need the
separate lane below; neither `just ci` nor `just test` executes them. Follow
[CONTRIBUTING.md's PostgreSQL setup](../CONTRIBUTING.md#postgresql-backed-tests) for
isolated native Postgres/Redis, maintenance-role privileges and protected
environment configuration, then run the full discovered lane from the repo root:

```sh
. ./bin/activate-hermit
./scripts/postgres-test-run.sh
```

Retain its exit status and per-test results, including exact executed test names
from `nip_fi_binding.rs` and Postgres-backed `router_tests.rs` cases. Verify these
tests actually passed: compilation, discovery, filtered-out tests and CI jobs
skipped by path selection do not count. An unavailable lane is a check gap, not
a pass. Keep database credentials out of the evidence record.

## Run record and fixture preparation

Create one run record with a unique run ID, UTC start/end, operator, independent
reviewer, and result (`blocked`, `failed`, or `passed`). Preserve redacted evidence
outside this repository. Record the exact command and exit status of every
check, the tested Git HEAD, and any working-tree changes. Never include raw
credentials, assertions, session cookies, private keys or mounted secret values.
Before execution, record the access-controlled artifact location and publish its
index URI in the ticket. Use this run-scoped layout; an unset evidence location
blocks execution:

```text
<evidence-location>/<run-id>/
  index.md          # actor/reviewer, scope, result, gaps and artifact links
  artifacts.json    # running images, commits, config/schema and baseline epoch
  checks/           # exact commands, exit status and executed test-name results
  scenarios/        # one record per matrix ID; stimulus, observations, verdict
  obligations/      # deployment/boot/generation and drain/retirement evidence
  cleanup.md        # resource owners, inverse actions, results and retained items
```

Before mutations, capture:

1. Confirmed cluster, namespace, deployment, service routing and fixture allowlist.
   Record running image digests and source commits for every relay and harness
   artifact, schema version, nonsecret effective configuration and secret version
   references. Compare actual runtime state with the reviewed rollout plan.
   Include effective `BUZZ_NIP_FI_MODE`, stored requirement/generation and the
   separate `BUZZ_NIP_FI_UPGRADES_ENABLED` admission switch in each scenario.
2. Every serving process's deployment identity, pod UID, boot/incarnation ID,
   image and admission state. Include processes with established sockets even
   after readiness removal. Prove old binaries that ignore policy cannot serve.
   Compare every registered or socket-holding process's digest with the reviewed
   candidate set; retain termination/socket-closure evidence for excluded old
   processes. An image match alone does not prove registration or drain.
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
8. Reconcile the BUZZ-515 routing companion,
   [squareup/builderbot-platform-core-infrastructure#3491](https://github.com/squareup/builderbot-platform-core-infrastructure/pull/3491),
   with the final BUZZ-514 staged base
   before using its Job renderer. Compare rendered and admitted Service and
   ServiceEntry selectors, Deployment/PDB selectors and security policies.
   Prove serving pods still match routing, while the Job avoids routing and
   workload/PDB selection but retains the labels required by security policies.
   Removing a policy label to escape routing is not a valid substitute. Record
   and link the reviewed final companion commit in the run record, together with
   the render diff and resources admitted by the separately approved server-side
   dry run. A moving PR reference is insufficient; source-only selector tests do
   not establish the live result.
9. Use BUZZ-515's separately approved server-side dry run to verify admitted
   `envFrom` imports the intended `platform-config`. Review later environment
   sources and explicit overrides. Missing render, selector/policy or
   platform-config evidence blocks Job creation. During the separately approved
   Job's startup, the harness must reject a cluster mismatch before any relay
   call and emit a bounded `clusterCheck=PASSED|MISMATCH` receipt. Record only
   that check outcome and the reviewed cluster identity, never other environment
   values. A silent check or generic exception is insufficient; missing runtime
   evidence blocks acceptance. The receipt implementation is pinned to
   [cash-server commit a8a01d50070348051e021432c3b6868da263b7c4](https://github.com/squareup/cash-server/blob/a8a01d50070348051e021432c3b6868da263b7c4/kgoose/service/src/main/kotlin/com/squareup/cash/kgoose/CommunityAuthUpgradeHarnessMain.kt)
   in BUZZ-515 / #145686. It emits
   `{"evidence":"startup-cluster","clusterCheck":"PASSED"}` (or `MISMATCH`)
   before input/credential loading, injector construction or relay calls, and
   retains the outcome in final evidence. Missing or wrong cluster emits
   `MISMATCH` and stops. The receipt proves the runtime equality check only;
   admitted ConfigMap/import provenance remains a separate gate. Full-package
   validation and execution gates remain unverified here. Do not infer a pass
   from a template or desired config. Neither dry run nor Job creation is
   authorized by this procedure.
   The immutable source link identifies the receipt contract, not a deployable
   candidate. Record the actual candidate head/image and verify that it retains
   this contract after any rebase or revision.

Do not infer process coverage from repeated requests through a load balancer.
Do not treat unavailable audio or Git fixtures as successful denials. Each
protected route needs a positive control proving its normal authenticated path
works and a negative proving the FI gate, rather than a later 404/500, rejected it.

## Acceptance scenarios

Run one scenario at a time. Use a new isolated target for independent upgrade
experiments because the requirement is monotonic. Retry/recovery scenarios reuse
the original operation ID and target. Recheck free and enforced controls before,
during and after every injected failure, including traffic on existing sockets.
FI-denial, drain and activation expectations in A–D assume an evaluating mode
with the target required. E covers Off and mode changes; do not apply those
FI-enforcement expectations to Off.

### Dedicated staging scope for deployment-wide switches

A6 and E1–E5 run on a dedicated relay deployment in bb-public staging, serving
only run-owned fixture communities. Both `BUZZ_NIP_FI_MODE` and
`BUZZ_NIP_FI_UPGRADES_ENABLED` are deployment-wide settings; isolated community
names alone do not isolate their effects. Never change these settings on the
shared staging relay to satisfy a row. Without the dedicated deployment and its
separately authorized scope, these rows remain blocked staging evidence.

Before execution, the operator must authorize and record the deployment ID,
namespace, candidate images, canonical hosts, community allowlist, routing and
DB/Redis isolation boundaries, initial settings, planned setting transitions and
expected control admission/drain outcomes for each mode. Prove shared staging
traffic cannot route to it and its faults cannot affect shared dependencies.
Keep shared staging controls under observation without changing their settings.
Account for every dedicated serving incarnation in its own audited baseline and
obligation ledger. Record pre/post-transition config and boot IDs; restarts or
replacement during a switch must preserve outstanding generation/boot obligations
and require reviewed drain or permanent-non-serving evidence, never implicit
retirement. Returning to Shadow cannot reuse an old ACK as proof of completion.

Only effects explicitly predicted for allowlisted fixtures by that authorized
mode transition are intentional (for example, Enforce denying a proofless
optional control). Compare these controls with the mode-specific baseline.
Unexpected effects, any drain outside that allowlist, or any shared staging
control regression still trigger the stop rules. This procedure grants no
permission to provision, restart or reconfigure a live deployment.

| ID | Stimulus | Required observation |
| --- | --- | --- |
| A1 | Invoke the authenticated module through the normal BUZZ-515 graph with the owner fixture | Durable tuple identifies the requested community, actor, operation and generation; saved policy is distinct from active |
| A2 | Rediscover and reconnect with valid FI using the original Nostr key | Root and `/info` agree on requirement/capability/descriptor, are `no-store`, and authenticated access works; keys and membership match the baseline |
| A3 | In Shadow for a stored-required target, and in Enforce for every target, omit FI; then use invalid signature, issuer, audience, expired assertion and wrong key | Expected 401/403 refusals across protected HTTP, bridge, root/audio WS, Git and applicable V1/media routes; no admitted effect or delivery |
| A4 | Reconnect, rediscover and renew assertions twice | Three distinct admission/renewal observations with issue/expiry times and unchanged public key; prove traffic after each renewal |
| A5 | Attempt wrong-key identity binding, then use the original key | `binding_mismatch`; original binding unchanged and original identity still works |
| A6 | Exercise both free controls and already-enforced control in each mode | Admission, delivery and membership match that mode's baseline: free access with and without FI in Off and for optional Shadow controls; Enforce requires FI on every community |
| B1 | Nonowner, unauthorized capability/standing, missing or expired session, unusable/unregistered actor key | Refused before policy mutation; durable state and membership unchanged |
| B2 | Wrong immutable ID/host pair; missing, inactive or mismatched issuer; unreadable trust | Refused or unavailable, with no implicit trust provisioning and no upgrade side effect |
| B3 | Race enable against ownership transfer, archive and deletion separately | Linearized valid outcome; losing request cannot mutate the wrong owner/lifecycle; readback matches durable state |
| C1 | Repeat identical operation and payload; then change one bound field | Same tuple recovered; changed payload conflicts without a second generation |
| C2 | Concurrent identical and distinct operation IDs | Contract-defined convergence/conflict; no duplicate upgrade or mixed actor/community tuple |
| C3 | Drop response after commit; retry; restart the kgoose harness process and request status | Recover the original tuple/generation without clearing policy or inferring acceptance from timeout |
| D1 | Drop notifications to one serving incarnation | Durable reconciliation applies target policy and actually drains its sessions; pending persists until its obligation is satisfied |
| D2 | Race registration, enable and completion; restart/replace processes | No unregistered serving window; a prior boot's acknowledgement cannot discharge a new boot's obligation |
| D3 | Pause a process with live target sockets across enable; resume it | Pending while unresolved; first resumed admission/effect/delivery cannot use old policy; active requires real quiescence |
| D4 | Hold a policy read, advance generation, then release the stale response | Late response cannot overwrite the required floor or authorize an effect; readback retains the newer generation |
| D5 | Isolate a serving process from policy DB while retaining sockets | Activation stays pending/unavailable as appropriate; target cannot falsely report drained; unrelated free controls retain baseline availability |
| D6 | Hold HTTP effects, output queues, subscriptions, audio and pending handshakes across enable | Completion waits for actual target teardown/effect barriers, not queued cancellation; valid FI reconnect remains possible |
| D7 | Crash before/after drain and before/after acknowledgement persistence | Durable recovery accounts for each boot; replacement/readiness/heartbeat expiry alone never retires the old obligation |
| D8 | Attempt completion with an absent/invalid serving baseline; register before enable but open the listener afterward | No false active from an empty registry; startup, participant capture and serving transition share an ordering |
| E1 | Run protected admission/discovery in Off against both optional and stored-required targets, including missing verifier/config and unavailable FI policy dependencies | No NIP-FI gate, verifier or FI policy lookup is required; discovery reports no effective FI requirement; ordinary Nostr and membership checks still apply |
| E2 | Run Shadow against optional and stored-required targets, including missing FI dependencies | Only stored-required targets enforce FI and fail closed; optional controls retain shadow behavior |
| E3 | Run Enforce against optional and stored-required targets | Every community requires FI, including controls whose stored requirement is false |
| E4 | After durable acceptance, switch to Off, retry/read status, then return to Shadow | Policy/generation/receipt persist; Off never reports active FI enforcement; restored Shadow enforces the stored requirement and needs valid drain/completion evidence before active |
| E5 | Disable the upgrade-admission switch before a fresh request and after a committed acceptance | New calls return 503 `upgrades_disabled` without a receipt; authorized status and identical receipt recovery remain available, subject to mode-aware completion |

The following is the **expected BUZZ-511 contract**, pending its published and
reviewed implementation. Confirm it against that exact head before a live run.
For A3, record the exact request/route and denial body from that contract.
The admission/discovery mode decision is:

| Effective relay mode | Stored optional | Stored required | FI dependency failure |
| --- | --- | --- | --- |
| Off | No FI enforcement | No FI enforcement | Must not introduce an FI denial or require FI policy/verifier dependencies |
| Shadow | Observe without FI enforcement | Enforce FI | Fail closed where FI is required |
| Enforce | Enforce FI | Enforce FI | Fail closed for every community |

This replaces the earlier expectation that stored requirements overrode Off.
It follows the [owner's mode decision](buzz://message?channel=51940054-5a7a-4773-a337-8004496630fa&id=fbafeb399947af3f8840ec9c5789bab29ae2d080567d474c13a7b619c9502ca0).
Off preserves the stored policy and durable receipt but cannot establish active
FI enforcement, even if an earlier drain acknowledgement exists. Returning to
Shadow restores the stored requirement; do not infer completion from a receipt
alone. In modes that evaluate FI, repair `deny_protected` retains 503 precedence.
Nostr authentication, tenant resolution and membership remain required in every
mode; Off bypasses FI checks, not these independent access controls.
The operator enable/status API retains its separate signature, ownership,
issuer-trust, lifecycle and durable-receipt preconditions in every mode.

Root NIP-11 and `/info` must agree on the mode's effective requirement: false in
Off, the stored requirement in Shadow, and true in Enforce. Unknown/non-serving
hosts remain generic 404. Unreadable required FI policy is 503 in evaluating
modes; an unavailable FI dependency must not turn Off into FI enforcement.

## Failure injection and recovery

Before each injection, record its target boot, affected connection, operation and
generation, the exact injector command, its scope and its inverse. Use isolated
test infrastructure for shared DB/Redis faults unless staging has a reviewed way
to constrain the fault. Deployment-wide setting changes are confined to the dedicated staging scope
above; do not change shared-fleet policy or drain unrelated communities as a
fallback. If an injector cannot preserve these boundaries,
record the scenario blocked and use a deterministic local integration fixture.
The local fixture supplies supplemental evidence only. Every matrix row is
mandatory for this ticket, including the required staging fault observations.
An unexercised staging row keeps live acceptance blocked; reviewer agreement
cannot waive it or relabel it not applicable.

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

## Stop and cleanup

Name every isolated fixture with a run-specific prefix such as
`auth-upgrade-<run-id>-<scenario>`. Before creating it, record its owner, immutable
ID, host, dependent trust and intended supported retirement action in the run
record. Keep a resource ledger of fault injectors, sessions, Jobs and fixtures;
never reuse a normal community as a disposable target.

Stop issuing upgrades or starting new injections immediately if a free/enforced
control deviates from its authorized mode-specific baseline, a community outside
the dedicated allowlist drains, scope no longer matches the allowlist, or evidence
shows a false active result. Only the explicitly predicted fixture effects of an
authorized mode transition are exempt; shared staging regressions never are. Apply the recorded inverse
of each reversible fault, stop the fixture traffic and record the run as failed.
Notify the operator and reviewer with affected scope, operation/generation and
the current recovery status. Recovery does not erase the failed observation.
If the inverse fails, preserve its error and escalate the unresolved fault;
do not proceed to another scenario.

Fault restoration is separate from the permanent authentication upgrade. Never
clear the requirement or edit operation/generation/ACK rows to reset a fixture.
Retire isolated upgraded communities through their supported archive/deletion
lifecycle, preserving required audit records. Close test sessions and remove
only run-owned temporary Jobs and fault resources. Do not remove issuer trust
while any active fixture or other community depends on it. Record each cleanup
command and result, and list retained resources with owner, reason and next
action. Stop before cleanup actions outside the run's separately authorized
scope and report them as retained; drafting this document authorizes no cleanup
against live infrastructure.

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

A run passes only when every matrix row has executable test coverage and staging
evidence, all process obligations are accounted for, package checks pass and an
independent reviewer has reviewed the exact auth/persistence/
activation heads. A passing module invocation proves module-to-relay integration;
it does not prove an HTTP call through a running kgoose pod or V0/V1 onboarding.
Keep those acceptance claims separate.
C3 restarts the candidate harness with the normal kgoose injection graph and
reuses durable relay state; it requires no running kgoose service pod or new
HTTP endpoint. Record a separate result if a service-pod restart is also tested.

Leave BUZZ-516 open when prerequisites, route fixtures, fault controls, checks,
independent review or live observations are missing. Report each gap, its owner
and the concrete next action. A drafted procedure or green unit suite is not
live acceptance. This procedure authorizes no deployment or production rollout.
