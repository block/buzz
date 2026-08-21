# Buzz relay ingest ceiling harness

Measures where accepted-event throughput stops tracking the offered rate, and
whether the audit write path is what stops it.

It exists because the backend perf findings in
`RESEARCH/BUZZ_BACKEND_PERF_FINDINGS.md` (round-trip counts read from the code)
predict a hard ingest ceiling, and nothing in this repo could measure it. That
doc's numbers are structural claims plus arithmetic; this harness is the part
that can be wrong out loud.

## What is under test

`buzz-audit`'s `log` is six sequential round trips per entry (advisory lock,
BEGIN, head read, INSERT, COMMIT, unlock — `crates/buzz-audit/src/service.rs`),
and it sits on the OK path: `dispatch_persistent_event` awaits
`audit_tx.send()` on a bounded channel before the rest of the dispatch is
spawned. So sustained ingest cannot exceed one audit entry per six round trips.

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

```bash
./scripts/start-perf-ingest-rig.sh --reset > /tmp/rig.json
./perf/relay_ingest_ceiling.py --rig /tmp/rig.json --json /tmp/ceiling.json
```

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

Non-zero exit on any of these, with every failure reported rather than the first:

1. **Admission quota rejections moved.** Then the limiter was measured, not the
   relay. See the trap below.
2. **`audit_log` did not grow with audit enabled.** The subject was never
   exercised.
3. **`audit_log` grew with audit disabled.** The attribution control did not take
   effect, so it would have agreed with the hypothesis for the wrong reason.
4. **No knee up to the highest offered rate.** The predicted ceiling did not
   appear at these rates. This is a finding, not a harness defect, and it has to
   be loud.
5. **The knee did not move when audit was disabled.** Something other than the
   audit path is the ceiling.

`perf/test_relay_ingest_ceiling.py` pairs every passing case with a mutant that
must fail — a lone dip that must not be called a knee, a control that did not
take effect, a limiter-contaminated run. A contract that cannot go red is
decoration.

## How the knee is defined

`achieved / offered` falls below `1 − 3s`, where `s` is the relative spread the
**null control** measured on this machine: the lowest sweep rate run twice, back
to back. The threshold is calibrated to the rig rather than asserted, so a noisy
machine widens it instead of manufacturing a knee. A fixed constant like 95%
would be a number nobody measured.

A knee must also persist at the next higher rate. Saturation is monotone; a
single dip is noise. The highest rate may stand alone because it has no
successor.

The report gives `ceiling_bracket_*` as `[last passing rate, knee]`. A sweep only
ever brackets the ceiling between the last rate it met and the first it did not —
quoting the knee alone reads a grid point as a measurement. A finer grid narrows
the bracket.

**Latency is corroboration, not part of the predicate.** p99 is reported next to
every point and never gates the verdict: it is an extreme order statistic, while
`achieved/offered` is a ratio of two aggregate rates, so a conjunctive gate would
let the noisier signal hide a real knee.

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
sweep is valid evidence for "the audit path caps a community at ~1/(6·RTT)". It
is *not* evidence that a real community reaches that rate. At production defaults
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

`load_per_cpu` is recorded per run because the null control only absorbs load
that is steady across two adjacent runs. A drifting background load is invisible
to it and looks like a ceiling. Run sweeps on an otherwise idle machine.

## Not yet measured

* **The knee-versus-RTT slope.** The claim is that the ceiling is six round
  trips, so the knee should fall roughly linearly in RTT with slope ~1/6.
  Testing that needs a fixed delay injected between relay and Postgres, swept
  across at least three values; a single injected value agreeing with one
  predicted number is a coincidence that cannot be distinguished from a correct
  prediction. Local Postgres is a loopback socket, so absolute rates from this
  rig are not comparable to a same-VPC deployment — the harness prints measured
  latency beside every rate for that reason, and no absolute events/s figure from
  it should be quoted as a production number.
* **The lock ceiling**, per the blindness note above.
