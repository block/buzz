# Captured admission failure: owner reconciliation — 54b0

## Decision

No semantic correction is justified by this trace. **Gate remains OPEN: 111/112,
223752.938ms natural FULL DEFAULT concurrent installed-enabled run.** Conflicting-ID
is 2612ms against unchanged `<2500ms`; the separate original 2572ms wrong-authority
failure remains unresolved. This is not another non-recurrence report.

Source inspected at `6697ad804318a6e87c7f6da091f54f13781d2493`. Its diff from the
executed `36e2eaca0008bad9156cdc53bba67ac7cd9941ab` is only CHECKPOINT and provider
result documentation. No source/test/deadline/authority change, new timing probe,
full-suite rerun, install, or provider-scope expansion was made. Offline trace
arithmetic was checked with Node 24.15.0; pnpm 11.4.0 verified. Existing strict and
suite results remain attributable to the executed candidate, not new validation.

## Exact event association and critical path

Raw evidence: workspace `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/PROVIDER_16B4/`,
`traces/83342-conflicting-id.json`, 472 rows, zero dropped; full-default.log contains
the accepted/conflict/accepted receipts and unchanged failed assertion.
All times below are monotonic milliseconds relative to test.timer.begin. Trace
owner is Node PID **83342**: test, development relay, host and UI share one event
loop. Supervisor and fixture runner are external children; their PIDs, CPU and
syscall timings are not in this trace. Two decode rows are UI then host; the latter
is immediately followed by host.route. No cross-process clock subtraction is used.

- Start and invalid Stop operation: `faa16ef1-ee24-49f0-b231-b0416785c491`.
  Envelope signature prefixes `50b325fc` and `88142874` distinguish them despite
  the shared operation ID (full signatures retained in the raw trace).
- Start receipt `cb560fb8-039c-4a95-b4e7-631a8a31fc40` (signature `26cf3f89`).
- Conflict receipt `49d73b26-ca2b-4f67-8936-0ae28d8a78d8` (`0fc11afe`).
- Valid Stop `88c40dde-9ab0-461c-90c4-6cddec7aa9a1` (`9c77de4b`).
- Stop receipt `2186daa5-4252-41ba-974d-4cd405b1a35d` (`bbc87557`).

| Non-overlapping critical-path section | Start–end ms | Owner/source |
|---|---:|---|
| Submit, initial relay backlog, durable batch, reconnect, delivery | 0–153.806 | client.ts:22–39, relay.ts:commit; Start batch durable 147.894 |
| Start reservation persist | 153.846–261.832 | host.ts:409; storage.ts:6–16 |
| Preparation/actual construction gap | 261.832–380.300 | host.ts:429–460, prepareLocal:200–222 |
| Start transitioning persist | 380.300–477.196 | host.ts:460 |
| Spawn/readiness/continuation gap | 477.196–761.769 | host.ts:461–465, owned.ts:6–30, supervisor.ts:22–30 |
| Start running/history/receipt commit | 761.769–1344.750 | host.ts:496–513 |
| Seal/send accepted + inventory, dequeue/seal/send conflict | 1344.754–1349.907 | host.ts:389–400,513; client.ts:send |
| Relay remaining inventory batch then receipt batch; UI decode | 1349.907–1625.343 | relay.ts:commit, relay-storage.ts |
| Poll, assert running/revision 1, submit valid Stop | 1625.343–1643.832 | admission-cancel.test.ts:101–113 |
| Stop relay commit/delivery | 1643.832–1759.741 | durable 1756.775 |
| Stop reservation + transitioning commits | 1759.765–1915.723 | host.ts:409,501 |
| Owned Stop | 1915.733–2447.602 | owned.ts:stop; supervisor.ts:10–15 |
| Stopped/null actual/revision 2/receipt commit | 2447.627–2499.709 | host.ts:501–513 |
| Seal/send/relay persist/UI decode | 2499.715–2593.462 | receipt batch durable 2592.003 |
| Poll + stopped assertions + timer | 2593.462–2611.454 | admission-cancel.test.ts:until,113–116,136 |

Small gaps between listed intervals are ordinary uninstrumented instructions;
this is not a claim that every instruction is timed. Specifically, the shared-ID
Stop is received at 153.781 but queued behind Start until 1348.277. Its handle-to-
seal is 0.012ms. The legitimate Start is not cancelled because receive's queued-ID
reservation already belongs to Start (`host.ts:155–166`). This is the intended
fingerprint/duplicate fence, not an authority bypass to optimize away.

Initial reconnect inventory `52ea3937-77e2-4702-88b6-0b036fc3c452` (`2bdea81b`)
occupies the relay writer from 148.013 until durable 1467.313. The receipt batch
starts 1467.419, durable 1620.866. Thus conflict publication is held for the tail
of an already-started FIFO inventory snapshot, then its own snapshot, not a
second 615ms write after the Start journal. Its own snapshot includes rename
await 1508.400–1603.977 (95.577ms); this is JS await elapsed, not syscall service.

## What the measured journal regions do and do not establish

The final Start write lasts **582.982ms**. `storage.ts:6–16` has these exact regions:

| Region | Duration ms | Included work |
|---|---:|---|
| private.begin → file-sync.begin | 392.744 | trace bookkeeping, mkdirSync recursive/0700, dirname, UUID/temp naming, openSync wx/0600, JSON.stringify, writeFileSync string encoding/write(s) |
| file-sync.begin → file-sync.end | 19.850 | trace bookkeeping + fsyncSync wrapper |
| file-sync.end → dir-sync.begin | 152.055 | trace bookkeeping, closeSync(file), renameSync (nonexclusive journal path), dirname/openSync(directory) |
| dir-sync.begin → dir-sync.end | 18.321 | trace bookkeeping + fsyncSync wrapper |
| dir-sync.end → private.end | 0.011 | trace bookkeeping + closeSync(directory) |

No chmod/lstat permission-check call occurs in this write path; modes are supplied
to mkdir/open, and actual kernel permission checks are inside filesystem calls.
The initial exclusive provisioning link/unlink branch is not this terminal write.
Serialization, encoding, writeFile, open, mkdir, rename and close have no individual
stamps. CPU execution, GC, descheduling and unknown kernel waits cannot be separated.
Even the fsync figures are entry-to-return wall wrappers, not CPU or pure device time.

Instrumentation (`latency-trace.ts:8–12`) takes Date.now/performance.now, allocates a
row and pushes it into memory. Inner stamps are nested inside outer intervals:
do not sum their spans twice. No per-row console/file logging occurs. The only
trace JSON serialization/write/fsync is endTrace AFTER cleanup and the failed timer.
The fixture storage hook also pushes a bounded timestamp record and returns a
promise; its awaits and callback scheduling are not independently measured. No
measurement bounds the cost of each trace call or proves instrumentation negligible.

There is no intervening application callback between private.begin/end. A loop tick
is 582.785ms late immediately afterward: this proves the shared loop cannot progress
during synchronous journal work, not why a particular syscall took time. The relay
file.write await 734.507–1349.964 overlaps the whole terminal write. Its 615.456ms
cannot be added to 582.982ms or identified as disk time. Relay file.open await
478.140–734.498 occurs while ticks continue: 256.358ms not accounted for by that
later blocking region, but still ambiguous worker queue/kernel/callback scheduling.

The 118.468ms preparation gap includes local setup validation, access/realpath/stat,
script reads/hashes and executable reads/hashes, selection cloning and actual-run
construction. prepareLocal hashes the executable and actual construction reads it
again; that duplication is visible in source, but no trace assigns its cost or
justifies caching executable identity across mutation. The 284.572ms readiness gap
includes supervisor spawn, IPC launch, runner OS spawn notification, continuation,
cloning run history and receipt construction; it does not await the fixture's
four-second prompt. Child-side costs are unknown. The 531.869ms Stop includes the
500ms live-anchor containment timer plus IPC/group-absence polling/reaping/scheduling;
the excess 31.869ms is not a measured single subsystem delay.

## Same-capture comparison, not a counterfactual fix

The earlier wrong-authority control in PID 83342 passes in 1711.609 monotonic ms
(1712 wall). Start handle-to-seal is 431.619ms versus 1190.948ms: **759.329ms**
difference. Its three journal spans are 76.666/52.645/110.127ms versus
107.986/96.896/582.982ms. Preparation gap is nearly unchanged,
119.568 versus 118.468ms; readiness/continuation gap grows 72.588 → 284.572ms.
The final Start journal difference alone is 472.854ms; readiness difference is
211.984ms. Owned Stop differs only 3.207ms (528.663 → 531.869).
This localizes the variation beyond 'contention', but the two controls have different
rejection owners and do not prove a latency bound under async journal conversion.

## Why not patch the fixture or mechanically await persistence?

The fixture deliberately waits for accepted Start and conflict evidence before
valid revision-1 Stop, then verifies stopped/null/revision 2 before timing. The
same-ID receipt helper aliases the two IDs, but the explicit conflict-result wait
at line 109 resolves the ambiguity here; both waits complete at 1641.794/1641.841.
No missing prerequisite/order race caused this failure. Excluding Stop, moving its
wait outside the bound, or shortening containment would change the asserted journey.

Async persistence may improve management responsiveness, but must first preserve:

1. **One serialized commit owner and stable snapshots.** Today save is synchronous
   in each queued handler. Merely replacing it with promises permits heartbeat,
   replay and receive to see mutated undurable state. Exact immutable bytes and a
   committed read view must be separated from working state; no concurrent commits.
2. **Durable before effects/report.** Reserve operation before spawn/kill; preserve
   transitioning crash state; commit receipt/outbox/history before publication.
   Start completion at revision 1 must not be retrospectively erased by a later
   Stop, which has its own revision-2 outcome and immutable prior run history.
3. **Receive-time cancellation reservations and duplicate/CAS fences.** Receive
   cannot be moved wholly behind the mutation queue: it interrupts in-flight
   admission. Pending commit windows need an explicit linearization rule, especially
   a valid Stop arriving after the final retraction check but before async durability.
4. **Consumed Move.** host.ts:325–369 consumes source authority plus exact grant
   outbox before sending; target durably accepts assignment before launch checks,
   preserves later candidate edits, and records incoming/outbox replies for replay.
   Async publication cannot resurrect source authority or acknowledge an undurable
   destination. Nested handle(start) must not deadlock its commit owner.
5. **Failure/uncertainty/close.** save sets persistenceFailed on any failure; handle
   refuses later effects. Queue error quarantine, heartbeat, replay and close are
   additional owners, not just handle's save sites. An async design needs explicit
   pre-replacement failure versus uncertain replacement, visible-state fences and
   close draining before lock release; unknown ownership stays quarantined/locked.

These are prerequisites, not claims that current failure paths have been newly
fully verified. Switching APIs alone cannot remove the sequential journal prerequisite
from this whole-sequence bound. Reordering the relay's already-started inventory
snapshot would also change its committed-prefix visibility guarantee.

## Next decision and surviving evidence

**Do not use another full capture as the next discriminator.** The precise unresolved
components are the 392.744/152.055ms mixed synchronous subregions and the 284.572ms
spawn/readiness gap (including the independently outstanding relay open). No existing
stamp can recover their syscall/CPU decomposition, and a deterministic synthetic
stall would only prove already-established shared-loop blocking, not that cause.
No new timing control was spent for that reason.

If responsiveness is an independently accepted product requirement, the next work is
an explicitly scoped host commit-owner design implementing the five invariants above,
with deterministic held-commit tests for receive/replay/heartbeat/close/failure/Move.
It must not be sold as a fix for this aggregate wall bound without candidate evidence.
Otherwise preserve the open gate rather than changing semantics to obtain green.
No further owner source reread or kernel-permission escalation is needed to reach
this decision. Provider review and unfinished provider local-add/conversion,
Databricks token/OAuth, mesh/compute/live acceptance remain separate work.

Workspace `ADMISSION_54B0/RECONCILE.ts` and `DECOMPOSITION.txt` reproduce the offline
six-write partition, counts, PID and zero-drop checks for both captured controls.
Original traces/full log/source+binary pin records remain untouched in PROVIDER_16B4.
SHA256: failing trace `c567ac1e2360bf4d3fdd0c8295f0b85ca3ef403a7c8233e6b41e30710e65c285`;
passing trace `c09c7aed945019c8c904bcb6f1aa7d25c1842c1d50e29fd025d5654b5487b641`;
full log `0dd8d364cc7ff96fdf01da33ba95a1816439011f7e499f8f83fa4292c91c33d9`.
Installed binary pins remain in PROVIDER_16B4/final-pins.txt (not source-built here).
