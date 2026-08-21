# Buzz relay ingest ceiling harness

Measures where accepted-event throughput stops tracking the offered rate, and
whether the audit write path is what stops it.

It exists because the audit write path costs a fixed number of sequential
statements per accepted event, which predicts a hard ingest ceiling, and nothing
in this repo could measure it: there are no criterion benches, and `perf/`
otherwise covers only the Redis fan-out boundary. The structural claim is
verifiable from the source cited below; this harness is the part that can be wrong
out loud. (See `RESEARCH/BUZZ_BACKEND_PERF_FINDINGS.md` for the original survey
and its local checkout line references.)

## What is under test

`buzz-audit`'s `log` is six sequential client/server exchanges per entry —
advisory lock, BEGIN, head read, INSERT, COMMIT, unlock
(`crates/buzz-audit/src/service.rs`) — plus a synchronous durability wait inside
the COMMIT. It sits on the OK path: `dispatch_persistent_event` awaits
`audit_tx.send()` on a bounded channel before the rest of the dispatch is spawned.
So sustained ingest cannot exceed one audit entry per that fixed cost.

The exchanges and the durability wait are separate costs, and conflating them is
easy: disabling `synchronous_commit` removes the flush *wait* but not the COMMIT
exchange, so a model of "five network round trips plus a commit" overcounts what
that setting removes. Both are amortized by batching, which is why the direction
of the proposed fix does not depend on the split.

There are **two** ceilings and they coincide numerically:

* **Per-pod worker ceiling.** One `tokio::spawn` per `AppState` drains the audit
  channel serially for every community on the pod
  (`crates/buzz-relay/src/state.rs`). Aggregate across communities. More pods
  raise it.
* **Per-community lock ceiling.** The advisory lock is DB-global, so the six
  round trips serialize per community cluster-wide. More pods do not raise it.

**The worker ceiling is the lower of the two, so it masks the lock ceiling.** A
sweep that does not surface the lock is *structurally blind to it* — it is not
evidence the lock is fine. Exposing the lock needs a second measurement round
after the worker is fixed. Do not quote a passing run as clearing the lock.

## Run it

A full experiment is two half-runs, one per arm, judged together. Both arms bind
the same port, so they cannot run at once.

```bash
./scripts/start-perf-ingest-rig.sh --reset > /tmp/rig-on.json
./perf/relay_ingest_ceiling.py --rig /tmp/rig-on.json --json /tmp/on.json

./scripts/start-perf-ingest-rig.sh --audit off > /tmp/rig-off.json
./perf/relay_ingest_ceiling.py --rig /tmp/rig-off.json --json /tmp/off.json

./perf/relay_ingest_ceiling.py --combine /tmp/on.json /tmp/off.json
```

Each half-run exits non-zero on its own — a single arm is a partial experiment by
construction — and writes its `--json` first, so the workflow above still works.
`--skip-relay` attaches to a relay someone else supervises, which is required in
environments that reap detached processes.

The rig is one relay process serving two communities, resolved by `Host`:
`a.localhost:3030` and `b.localhost:3030` both resolve to 127.0.0.1, so the URL
host *is* the `Host` header and no proxy is involved. That is what lets the
harness drive two communities at independent rates through one worker. Backing
services run under the `buzz-harness` Compose project, so the shared `:3000` dev
stack is untouched.

Verdict logic without any services:

```bash
./perf/relay_ingest_ceiling.py --mode model
python3 -m unittest discover -s perf -p 'test_*.py'
```

## What it asserts

The contract is an **arm separation**, not a threshold. Each rate is run `--repeats`
times in both arms, and the verdict asks whether the difference in accepted/offered
between audit-off and audit-on excludes zero at any rate. No noise floor is needed
for that.

An earlier version derived its pass threshold from the spread of repeated runs
(`1 - 3s`). That is retired, and the reason is worth keeping: in the unsaturated
region accepted/offered is pinned at 1.0, so the spread is ~0 and the threshold
absorbs nothing; in the saturated region the spread is the system's own throughput
variability, which is the signal, not the noise. No placement of that control
rescues the formula.

Overlapping intervals are also not used as evidence of anything. "The arms'
intervals overlap" would not establish that they are equal — absence of a
significant difference is not evidence of absence — so the predicate is on the
interval of the *difference*.

Non-zero exit on any of these, with every failure reported rather than the first:

1. **A cell was contaminated by admission control.** Either `reason="quota"` or
   `reason="unavailable"` moved. The second matters as much as the first: a
   rejected event takes the same NOTICE-without-OK path either way, and admission
   itself costs Redis round trips against the same rig the sweep is loading — so
   unavailability is load-correlated and can forge a knee that *persists* across
   repeats at exactly the rate a reader would trust.
2. **A cell saw relay rejections, generator transport errors, or audit-write or
   audit-enqueue failures.** An enqueue failure specifically means the worker is
   gone: a bounded `mpsc::Sender::send` awaits when full and errors only when the
   receiver is dropped.
3. **A cell was not in steady state** — `outstanding_delta` moved more than a small
   fraction of the audit channel's depth. The criterion is stability, *not*
   emptiness: a saturating cell settles with the channel full and backpressure
   engaged, an unsaturated one settles near zero, and both are legitimate. A gate
   written as "assert the queue is empty before the window" would make every
   saturated cell — every cell that matters for a ceiling — unmeasurable while
   looking like a working precondition.
4. **The generator had less than 1.5x headroom over the offered rate**, so the cell
   was partly measuring the generator rather than the relay.
5. **The audit-off control did not run.** A single-arm dataset is a partial
   experiment and reports `control.ran: false`; `--combine` judges the pair. The
   verdict carries an explicit ran/skipped marker, because "no knee on the
   audit-off arm" otherwise reads identically for "the control ran and the knee
   was gone" and "the control never ran".
6. **The two halves handed to `--combine` are not the same experiment.** Rates,
   duration, repeats, community hosts, both limiter settings, the generator path,
   and the source revision must all match.
7. **No rate separated the arms**, so the audit path is not shown to limit ingest.

`perf/test_relay_ingest_ceiling.py` pairs every passing case with a mutant that
must fail — a lone dip that must not count as a knee, a control that did not run,
an arm separation in the wrong direction, a cell that banked the whole channel, a
`--combine` across mismatched durations. A contract that cannot go red is
decoration. Nothing wires this suite into CI, a Justfile target, or a hook (the
same is true of the pre-existing bus-scaling tests), so run it by hand:

```bash
python3 -m unittest discover -s perf -p 'test_*.py'
```

## Two throughput series, and why neither replaces the other

- `accepted_per_s` — user-visible ingest. Arm separation is computed on this,
  because it is the series both arms have.
- `audit_completed_per_s` — audit-worker completions, from
  `buzz_audit_log_seconds`. **N/A in the audit-off arm**: with
  `BUZZ_AUDIT_ENABLED=false` there is no worker and no series, so substituting it
  for accepted throughput would make the positive control read as total collapse
  and invert the predicate.

Accepted throughput needs the steady-state gate because the audit channel is a
bounded `mpsc::channel(1000)`: a cell that starts with it empty accepts up to
1000 events before backpressure. Measured on this rig, `accepted - completed` came
to **exactly +1000** from a known-empty start and **exactly 0** from a full one.
That credit is a *bias*, identical across repeats, so an interval over n runs
converges tightly on a wrong number — precision and bias are different axes and n
only buys the first. It is exactly 1000 only in deep saturation; a transition cell
banks a partial amount depending on both offer and duration, and the knee lives in
the transition region, so the bias is least tractable exactly where the bracket is
decided.

`audit_completed_per_s` is free of that credit, and it licenses a capacity claim
only where `audit_busy_fraction` is near 1 and no error counter moved — below
saturation a completion rate just tracks the offer. Two further bounds: the
histogram is a per-pod aggregate, so it cannot be split per community in the
two-community cells; and it measures the audit worker, which is the subject only
while the audit path is the binding constraint. Once the worker is fixed, it and
ingest throughput part ways.

`audit_busy_fraction` is reported explicitly rather than left implicit. Completion
rate and `1/mean(service)` are `C/T` and `C/S` over the same count, so their ratio
is exactly `S/T` — they are one measurement reported two ways, and their agreeing
tells you the worker was busy, not that two instruments corroborate each other.

## The two-community arm, and what it can show

Each rate is also run split evenly across two communities. The expected relation
follows from which defect binds:

- If the **per-pod worker** is the ceiling, the two-community aggregate knee sits
  at roughly the same place as the one-community knee — the worker drains all
  communities serially, so splitting the offer buys nothing.
- If the **per-community lock** were the ceiling, two communities would reach
  roughly double the combined rate.

Round 1 observed the first shape. The cells are recorded but **not** judged: a
CI-backed equivalence test on the difference between the arms, with a predeclared
margin, is the designed follow-up. Overlapping marginal intervals would not
establish that the two ceilings are equal, so nothing here asserts that they are.


## Two latencies, and why both

`ingest_load` reports each send twice:

* `service_ms` — signed event on the wire until the relay's OK. Relay time.
* `scheduled_ms` — the send's *intended* slot until the OK.

The schedule advances by a fixed interval and is never rebased on the response.
A generator that paces with a self-correcting timer and measures from the actual
send silently redefines its own offer downward when the relay slows: throughput
caps at `connections / latency` and the queueing delay never enters the
percentiles. That is coordinated omission, and it hides the damage exactly when
the damage is the point. Signing happens before the service clock starts, so
BIP340 cost lands in `scheduled_ms` and never inflates the relay's number.

`conn_capacity_per_s` is the ceiling the generator imposes on itself — each
connection is closed-loop, so it cannot exceed one send per service latency.
Compare `achieved_per_s` against it before believing any knee; raise `conns` when
they are close.

## The trap this harness exists to avoid

At default settings one identity is capped at **50 events per 5 seconds**:
`human_ws_events_per_sec` defaults to 10 (`crates/buzz-auth/src/rate_limit.rs`)
and `ws_admission_budget` turns that into a fixed 5s window with a limit of 50
(`crates/buzz-relay/src/admission.rs`), keyed on `(tenant, pubkey)`.

A rejected EVENT gets a **NOTICE**, and a NOTICE carries no event id
(`request_rejection_message` with no `sub_id` — `crates/buzz-relay/src/connection.rs`).
A NIP-01 client waiting for an `OK` therefore never sees the rejection and blocks
for its whole publish timeout (30s in `buzz-ws-client`). Measured on this rig: 50
events land in 2.4s, the next send stalls 30s, and the run self-truncates. A
reader seeing the resulting ~1.5/s would have a limiter artifact that looks like
a textbook saturation knee.

So the rig raises both admission limits — both matter, because `WsEvents` is a 5s
window and `Messages` a 60s one, and a short run only ever exercises the first.
The configured values are printed in the run metadata, and the harness invalidates
any run where the quota-rejection counter moves. Scoped to `reason="quota"`:
`reason="unavailable"` means the limiter itself was unreachable, which is a
different diagnosis.

## What this harness can and cannot support

**It characterizes the mechanism, not deployability at scale.** A raised-limit
sweep is valid evidence that the audit path caps a community's ingest. It is
*not* evidence that a real community reaches that rate. At production defaults
one identity sustains far less, so reaching a few hundred events/s inside one
community needs hundreds of concurrent identities — each carrying its own
WebSocket and its own per-event admission round trip that this sweep never pays.

The **N-identity variant** is the queued fidelity check for exactly that gap.
Two constraints on whoever builds it:

* **Self-hosted isolated stack only.** `SECURITY.md` asks reporters not to
  disrupt production systems. Never point it at a hosted or shared relay.
* Nothing on the relay side will stop it opening hundreds of connections from one
  host: `LimitType::IpConnections` is defined in the enum but wired up nowhere,
  and the only connection cap is a global semaphore. Convenient here, and not to
  be mistaken for a control that exists.

`load_per_cpu` is recorded per cell, but read it with its resolution in mind: it
is a 1-minute average sampled inside a much shorter cell, so consecutive cells are
autocorrelated and every cell in a sweep reads about the same. It can catch a
sweep-long compile storm on a shared machine; it cannot exonerate one cell. Run
sweeps on an otherwise idle machine.

## Not yet measured

* **Sensitivity to round-trip latency.** Injected delay between relay and
  Postgres should move the ceiling, and a sweep across at least three injected
  values can measure how much — a single value agreeing with one predicted number
  is a coincidence indistinguishable from a correct prediction. But predeclare
  the expected shape as *six* added exchange delays plus a durability intercept,
  not five: the COMMIT exchange is still a round trip. Attributing the resulting
  slope needs per-statement timing, since the measured interval also contains pool
  acquisition, SQL execution, hash chaining and WAL generation. No clean
  round-trip decomposition is available from anything run so far. Local Postgres is a loopback socket, so absolute rates from this
  rig are not comparable to a same-VPC deployment — the harness prints measured
  latency beside every rate for that reason, and no absolute events/s figure from
  it should be quoted as a production number.
* **The lock ceiling**, per the blindness note above.
