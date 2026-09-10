# Admission latency reconciliation — 0bea2de8

## Status: original latency cause unresolved; no production correction justified

The failed executable candidate is `dd97d91405dcbe85f45c12d0ca17271875088110`.
Verified diff to published `f8556d157cde259fc016f6c063dd8f4e43b9cef5` contains
only CHECKPOINT/README additions, so there is no executable attribution difference.
Original final gate remains **105/106, 161.721s**, wrong-authority control elapsed
2572ms against unchanged `<2500ms`. Earlier historical admission/broker/tool
observations remain separately unclassified. This investigation does not close them.

## What the failed assertion actually measured

`test/admission-cancel.test.ts:69–140` runs its relay, host and UI in the SAME Node
test process; the lifecycle runner/supervisor are external children. For the first
control it terminates the host socket, starts the timer, signs/sends Start and a
Stop addressed to a fresh unknown agent, then:

1. Waits for BOTH the accepted Start and not-authority receipts.
2. Asserts the legitimate journal is running/revision 1.
3. Sends another, valid Stop at revision 1 and waits for its accepted receipt.
4. Asserts stopped/null actual/revision 2, then checks total elapsed `<2500ms`.

Thus 2572ms is NOT a measured wrong-agent rejection delay, nor a four-second ACP
probe timeout. The legitimate fixture Start and real owned teardown are intentionally
inside this control's timer. All the above semantic assertions passed before the
latency assertion failed; later loop controls were not reached.

The host's connection-level router (`src/host.ts:564–571` on the failed candidate)
publishes unknown-agent not-authority directly. It does not enqueue that request
behind the legitimate slot's Start, nor write a rejection to that slot journal.
The unknown-agent rejection has relay durability, not a per-slot host outbox entry.
Same-slot invalid-body/conflicting-ID/revision controls have different natural
owners and must not be conflated with this first control.

Actual failed durable order:

- inventory r0; legitimate Start r0; unknown-agent Stop r0; inventory r0;
- not-authority receipt r0; accepted Start receipt r1; inventory r1;
- valid Stop r1; inventory r1; accepted Stop receipt r2; inventory r2.

Receipt operations are respectively `30cb5ca9-6bfb-449f-b998-cfafddce1ca2`,
`28ef3c13-0e84-4712-927a-258d1319ff26`, and
`c69099ed-dd6d-4f57-a0ca-eb294a8c469a`. Durable order proves rejection precedes
Start receipt publication, not how many milliseconds either took.

## Original timestamp limits

Seven relay snapshots have callback entry timestamps (Unix milliseconds):

| Batch ordinal (not event association) | Before write | Before rename | Before directory sync |
|---|---:|---:|---:|
| 1 | 1789028830899 | 1789028830926 | 1789028830948 |
| 2 | 1789028830973 | 1789028830992 | 1789028831099 |
| 3 | 1789028831423 | 1789028831434 | 1789028831474 |
| 4 | 1789028831592 | 1789028831622 | 1789028832304 |
| 5 | 1789028832391 | 1789028832411 | 1789028832432 |
| 6 | 1789028832923 | 1789028832943 | 1789028832995 |
| 7 | 1789028833346 | 1789028833374 | 1789028833438 |

`src/relay-storage.ts` calls each hook BEFORE its named operation. The largest
adjacent gap, **682ms** from rename entry to directory-sync entry, covers rename,
directory open and JS/libuv scheduling. It is NOT measured directory-fsync latency.
Hooks lack batch signatures, completion timestamps, timer boundaries, send/sign,
host authorization/journal, receipt publication and client polling timestamps.
They include setup before the timer. No valid decomposition of the original
2572ms into these owners is recoverable. In particular, neither “disk contention”
nor “fixture race” follows from these records.

## One isolated diagnostic capture

Exported the immutable failed package using `git archive`; left the branch's
executable source untouched. Standalone frozen-lock install with scripts/workspace
disabled used the approved mirror. Actual Node **24.15.0**, pnpm **11.4.0**.
No second test runner, repeat-to-fail, focused rerun, final unchanged rerun, serial
mode, altered deadline/assertion or forced exit was used.

Instrumentation is isolated to the export, gated by `BEEHIVE_LATENCY_TRACE` AND
explicit admission-control activation. It buffers at most 50,000 metadata rows per
control in memory, records wall/monotonic clocks, writes once after owned fixture
cleanup, and uses an unref'ed 25ms event-loop lateness probe. No message body,
credential, env value, private journal content or filesystem path is traced.
It distinguishes seal/send, relay enqueue/batch/durable/broadcast, async IO entry
and completion, decoded receipt, host routing/slot handling, synchronous private
journal fsync spans, Stop ownership span and test polling. No extra async await
is added to the writer. Instrumentation still perturbs execution and cannot turn
a non-recurrence into a historical diagnosis.

Diagnostic strict TypeScript PASS. Exactly one FULL DEFAULT concurrent installed
capture completed naturally: **106/106, zero skips/failures/cancellations,
160932.313291ms**. Original wrong-authority failure did not recur: 1744ms;
conflicting-ID 1702ms, invalid-body 1733ms, stop-first 461ms, duplicate-replay
1821ms. All five traces retained without dropped rows. This is diagnostic
non-recurrence, NOT a final unchanged-suite retry or closure of the original gate.
The launch chain and export-scoped subprocesses were absent after natural
completion; no signals or historical PID cleanup were used.

Wrong-authority diagnostic timeline (relative monotonic ms; original failure is a
DIFFERENT execution):

| Owner boundary | ms |
|---|---:|
| Start sent / wrong-agent Stop sent | 2.004 / 3.484 |
| Host routes Start / unknown agent | 211.241 / 212.501 |
| Rejection seal begins / sent | 212.523 / 214.085 |
| Legitimate Start slot handle begins | 214.106 |
| Accepted Start receipt sent | 676.836 |
| UI decodes rejection / Start receipt | 765.323 / 766.503 |
| Test observes both | 784.294 |
| Valid Stop sent / host slot begins | 786.326 / 901.988 |
| Owned Stop begins / finishes | 1069.672 / 1600.470 |
| Accepted Stop receipt sent / UI decodes | 1662.758 / 1733.665 |
| Test observes Stop / timer ends | 1743.993 / 1744.394 |

This directly shows prompt unknown-agent routing (0.022ms to rejection sealing),
but delayed observation of that receipt in the shared test process. Synchronous
Start journal writes at 214.126–308.807 and 441.908–534.691ms overlap a relay
mkdir entry-to-JS-completion span of 207.007–536.653ms. A 25ms loop tick is
303.671ms late. These intervals prove that the pending async operation's elapsed
span is not pure filesystem service time. Additional synchronous prerequisite
work is inside slot handling; this probe does not break down all of it.

Owned Stop spans **530.798ms**. Its natural owner (`src/supervisor.ts:8–15`)
retains the group anchor through TERM and schedules group KILL at 500ms. This is
safety containment, not a wrong-authority request waiting behind Start, and must
not be shortened or skipped to meet the aggregate timer. Host accepted receipts
are sent only after synchronous journal persistence; relay visibility follows
file AND directory durability. A relay echo is not host completion.

## Decision and next discriminator

**Classification: original failure unresolved.** The non-failing capture identifies
shared fixture event-loop blocking and required durability/containment time in
its own execution; it does not prove which one caused the failed 2572ms outcome.
No demonstrated source-level erroneous queue/serialization, durability algorithm
bug or fixture-premise race warrants a production/test correction. No authority,
fingerprint, revision, operation-ID, cancellation, reconnect or persistence rule
was changed. No claim of full gate acceptance follows from a diagnostic pass.

The useful next discriminator is event-associated timing **at the failure**, not
another uninstrumented suite: distinguish the interval from unknown-agent host
routing to signed rejection send, send to relay durable batch, durable broadcast
to UI decode, and UI decode to test observation; separately attribute legitimate
Start's synchronous journal/prerequisite spans and valid Stop's owned exit. The
retained patch now captures those major boundaries. To distinguish an async IO
span's worker-queue/syscall time from JS callback scheduling requires additional
worker/kernel-level timing; JS await endpoints alone cannot establish that.
No further capture is performed within this bounded assignment, and no new human
input is needed to recognize this evidence limit. Do not stress/retry until failure
or change deadlines to manufacture closure.

## Surviving evidence and identities

Workspace `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/ADMISSION_0BEA/` retains the early
checkpoint, immutable `candidate.tar`, base/diagnostic source manifests, standalone
install/strict/full logs, five raw control traces, derived `trace-summary.txt`,
analysis script, process provenance/cleanup record and complete patch. Original
`RECOVERY_FCD9/full-default.log` and RESULT remain untouched.

- Immutable archive SHA256:
  `7c2f8dfdd27ff21328f334f2e97a8aaf2bab809d81ae5d0947c4481f99aeb80a`.
- Complete instrumentation patch SHA256:
  `37de1d9fd5110017a8be3caf739a5b3a4cf56ee5e512b77ea1d7619580f84129`.
  Use `instrumentation-complete.patch`, not the retained initial generator/diff;
  it includes the new utility and final writer probes, applied at export root.
- Installed read-only `/Applications/Buzz.app/Contents/MacOS/buzz-acp`:
  `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`.
- Installed read-only sibling `buzz`:
  `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`.

Installed binaries are separately pinned artifacts, not built from this source.
No provider login/inference, owner stores, production/current-kind40002, native/Rust
or existing-client changes, repository CI, merge, release or full-product claim.
Publication for this reconciliation is documentation only; exact head is recorded
in the workspace RESULT and terminal report. Logan Johnson author/committer + DCO
are verified before publication; no signing/configuration edits.
