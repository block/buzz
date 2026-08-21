#!/usr/bin/env python3
"""Ingest-ceiling harness for the Buzz relay's audit write path.

Answers one causal question — does the audit log limit ingest? — and reports a
worker-rate estimate alongside it. Stdlib only; `ingest_load` (Rust) does the
measuring and this script owns the experiment and the verdict.

The contract is an **arm separation**, not a threshold. With the audit log
enabled, accepted/offered falls away from 1.0 as the offer rises; with
`BUZZ_AUDIT_ENABLED=false` it does not. Repeats give each arm an interval, and
the verdict asks whether the difference between the arms excludes zero at any
rate. No noise floor is needed for that, which matters: an earlier design derived
its pass threshold from the spread of repeated runs, and in the saturated region
that spread is the system's own throughput variability — the signal, not noise.

Two throughput series are reported and neither replaces the other:

  * `accepted_per_s` — user-visible ingest, and the quantity arm separation is
    computed on, because it is the series both arms have.
  * `audit_completed_per_s` — audit-worker completions, from
    `buzz_audit_log_seconds`. Free of the acceptance credit below, and **N/A in
    the audit-off arm**, where there is no worker and no series. It licenses a
    capacity claim only where the worker was demonstrably busy
    (`audit_busy_fraction` near 1) and no error counter moved.

Why accepted throughput needs a validity gate: the audit channel is a bounded
`mpsc::channel(1000)`, so a cell starting with it empty accepts up to 1000 events
before backpressure — measured on this rig as exactly +1000 from an empty start
and 0 from a full one. That credit is a *bias*, identical across repeats, so an
interval over n runs converges tightly on a wrong number. And it is exactly 1000
only in deep saturation: a transition cell banks a partial amount depending on
both offer and duration, and the knee lives in the transition region, so the bias
is least tractable exactly where the bracket is decided. `outstanding_delta`
reports it per cell.

Two bounds on `audit_completed_per_s`, since it invites over-reading: the
histogram is a per-pod aggregate, so it cannot be split per community in the
two-community cells; and it measures the audit worker, which is the subject only
while the audit path is the binding constraint. Once the worker is fixed, it and
ingest throughput part ways.

Two ceilings are under test and they coincide numerically. The per-pod audit
worker drains all communities serially; the per-community advisory lock
serializes cluster-wide. The minimum always wins and it is always the worker, so
a first sweep is *structurally blind* to the lock. A run that does not surface the
lock is not evidence the lock is fine. See perf/RELAY_INGEST_CEILING.md.

Usage:
  ./scripts/start-perf-ingest-rig.sh --reset > /tmp/rig-on.json
  ./perf/relay_ingest_ceiling.py --rig /tmp/rig-on.json --json /tmp/on.json
  # restart the rig with --audit off, sweep again into /tmp/off.json, then:
  ./perf/relay_ingest_ceiling.py --combine /tmp/on.json /tmp/off.json

  ./perf/relay_ingest_ceiling.py --mode model     # verdict logic, no services

Exits non-zero when any cell is invalid or the contract is not met.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import urllib.request
from typing import Callable

# Connections per unit of offered rate. Each connection is closed-loop, so it
# cannot exceed one send per mean service time; this keeps the generator's own
# capacity clear of the offer. `generator_headroom` is the check that it worked.
RATE_PER_CONN = 25.0
MIN_CONNS = 4

# A cell is rejected unless the generator could have offered this multiple of the
# requested rate. Below it, the sweep is partly measuring the generator.
GENERATOR_HEADROOM_MARGIN = 1.5

# Depth of the relay's audit channel, and the share of it that `outstanding_delta`
# may move before a cell counts as not in steady state.
AUDIT_CHANNEL_DEPTH = 1000
OUTSTANDING_TOLERANCE_FRACTION = 0.05

# Below this, the worker idled inside the window, so its completion rate tracks
# the offer rather than its own limit.
BUSY_FRACTION_FOR_CAPACITY = 0.95

DEFAULT_RATES = [20.0, 50.0, 100.0, 200.0, 400.0]
DEFAULT_REPEATS = 5

# Two-tailed 95% critical values by degrees of freedom; the fallback is the
# large-sample limit. Enough for the repeat counts this harness runs.
_T95 = {
    1: 12.706, 2: 4.303, 3: 3.182, 4: 2.776, 5: 2.571, 6: 2.447, 7: 2.365,
    8: 2.306, 9: 2.262, 10: 2.228, 12: 2.179, 15: 2.131, 20: 2.086, 25: 2.060,
    30: 2.042,
}


# -- statistics (pure) ------------------------------------------------------


def mean(values: list[float]) -> float:
    if not values:
        raise ValueError("mean of no observations")
    return sum(values) / len(values)


def sample_stddev(values: list[float]) -> float:
    """Sample standard deviation, n-1.

    Not `(max - min) / mean`: the range's expectation grows with n (1.128 sigma
    at n=2, 2.059 sigma at n=4), so a range-based spread rises with sample size
    on its own and cannot be compared across cells with different repeat counts.
    """
    if len(values) < 2:
        raise ValueError("standard deviation needs at least two observations")
    avg = mean(values)
    return math.sqrt(sum((v - avg) ** 2 for v in values) / (len(values) - 1))


def t95(df: float) -> float:
    if df < 1:
        raise ValueError("t95 needs at least one degree of freedom")
    key = int(math.floor(df))
    for cutoff in sorted(_T95):
        if key <= cutoff:
            return _T95[cutoff]
    return 1.960


def confidence_interval(values: list[float]) -> dict:
    """Mean with a 95% t-interval. Bounds are None when n < 2."""
    avg = mean(values)
    if len(values) < 2:
        return {"n": len(values), "mean": avg, "lo": None, "hi": None}
    half = t95(len(values) - 1) * sample_stddev(values) / math.sqrt(len(values))
    return {"n": len(values), "mean": avg, "lo": avg - half, "hi": avg + half}


def difference_interval(a: list[float], b: list[float]) -> dict:
    """95% Welch interval for mean(a) - mean(b), unequal variances.

    Arm separation asks whether this interval excludes zero. "The two arms'
    intervals do not overlap" would be a weaker test and "they do overlap" would
    prove nothing at all: absence of a significant difference is not evidence of
    equality.
    """
    if len(a) < 2 or len(b) < 2:
        raise ValueError("difference interval needs two observations per arm")
    va, vb = sample_stddev(a) ** 2, sample_stddev(b) ** 2
    na, nb = len(a), len(b)
    se = math.sqrt(va / na + vb / nb)
    diff = mean(a) - mean(b)
    if se == 0.0:
        return {"diff": diff, "lo": diff, "hi": diff, "excludes_zero": diff != 0.0}
    df = (va / na + vb / nb) ** 2 / (
        (va / na) ** 2 / (na - 1) + (vb / nb) ** 2 / (nb - 1)
    )
    half = t95(df) * se
    lo, hi = diff - half, diff + half
    return {"diff": diff, "lo": lo, "hi": hi, "excludes_zero": lo > 0.0 or hi < 0.0}


# -- knee, reporting only ---------------------------------------------------


def find_knee(points: list[tuple[float, float]], threshold: float) -> float | None:
    """Lowest offered rate whose shortfall persists at the next rate too.

    Reporting only - nothing gates on it. A knee is a grid point, not a
    measurement, so `threshold` here is a presentation choice and not a contract.
    Saturation is monotone, so a lone dip is noise; the highest rate may stand
    alone because it has no successor.
    """
    ordered = sorted(points)
    for idx, (rate, fraction) in enumerate(ordered):
        if fraction >= threshold:
            continue
        if idx == len(ordered) - 1 or ordered[idx + 1][1] < threshold:
            return rate
    return None


def knee_bracket(
    points: list[tuple[float, float]], threshold: float
) -> tuple[float | None, float | None]:
    """The interval the ceiling lies in: (highest passing rate, knee)."""
    knee = find_knee(points, threshold)
    if knee is None:
        return (None, None)
    passing = [r for r, f in sorted(points) if r < knee and f >= threshold]
    return (passing[-1] if passing else None, knee)


# -- validity and verdict (pure) --------------------------------------------


def cell_problems(cell: dict) -> list[str]:
    """Everything that disqualifies one cell from being evidence."""
    problems = []
    rate = cell["offered_per_s"]

    for key, why in (
        ("quota_rejections_delta",
         "admission quota rejections: the limiter was measured, not the relay"),
        ("unavailable_rejections_delta",
         "admission reported unavailable: those events take the same "
         "NOTICE-without-OK path as quota, and admission itself costs Redis round "
         "trips against the rig the sweep is loading, so it is load-correlated "
         "and can forge a knee that persists"),
        ("audit_log_errors_delta", "audit writes failed"),
        ("audit_send_errors_delta", "audit enqueue failed, which means the worker is gone"),
        ("rejected", "the relay rejected events"),
        ("transport_errors", "generator transport errors"),
    ):
        if cell.get(key):
            problems.append("{:g}/s: {} ({}={})".format(rate, why, key, cell[key]))

    headroom = cell.get("generator_headroom")
    if headroom is not None and headroom < GENERATOR_HEADROOM_MARGIN:
        problems.append(
            "{:g}/s: generator headroom {:.2f}x is under {}x, so the cell is "
            "partly measuring the generator".format(
                rate, headroom, GENERATOR_HEADROOM_MARGIN
            )
        )
    return problems


# A dead audit worker invalidates every cell after it, not just its own, so this
# one problem fails the run instead of dropping a cell.
FATAL_PROBLEM_KEYS = ("audit_send_errors_delta",)


def fatal_problems(cells: list[dict]) -> list[str]:
    out = []
    for cell in cells:
        for key in FATAL_PROBLEM_KEYS:
            if cell.get(key):
                out.append(
                    "{:g}/s: audit enqueue failed, which means the worker is gone "
                    "and every later cell is suspect ({}={})".format(
                        cell["offered_per_s"], key, cell[key]
                    )
                )
    return out


def cell_exclusions(cells: list[dict]) -> list[dict]:
    """Cells that cannot be evidence, with why.

    Excluded rather than run-fatal. A saturating cell that starts with the audit
    channel empty banks its whole depth in accepted events — measured as exactly
    +1000 — so the *first* repeat of the first saturating rate in any sweep is
    non-steady by construction, and a transition rate accumulates over several
    repeats. Failing the run on that would fail precisely the datasets this
    harness exists to judge; the bias also means the cell does not belong in the
    interval, since the credit inflates accepted/offered.
    """
    excluded = []
    for cell in cells:
        reasons = cell_problems(cell)
        if steady_state(cell) is False:
            reasons.append(
                "{:g}/s: outstanding audit work moved by {}, so the cell carries "
                "acceptance credit rather than a steady rate".format(
                    cell["offered_per_s"], cell["outstanding_delta"]
                )
            )
        if reasons:
            excluded.append(
                {
                    "offered_per_s": cell["offered_per_s"],
                    "audit_enabled": cell["audit_enabled"],
                    "reasons": reasons,
                }
            )
    return excluded


def cell_is_evidence(cell: dict) -> bool:
    return not cell_problems(cell) and steady_state(cell) is not False


def steady_state(cell: dict) -> bool | None:
    """Whether outstanding audit work held level across the window.

    The criterion is stability, not emptiness. A saturating cell settles with the
    channel full and backpressure engaged; an unsaturated one settles near zero.
    Both are steady. Requiring "empty at start" would make every saturated cell -
    every cell that matters for a ceiling - permanently unmeasurable.
    """
    delta = cell.get("outstanding_delta")
    if delta is None:
        return None
    return abs(delta) <= OUTSTANDING_TOLERANCE_FRACTION * AUDIT_CHANNEL_DEPTH


def arm_separation(on_cells: list[dict], off_cells: list[dict]) -> dict:
    """Per-rate difference in accepted/offered between the arms.

    Separation holds when at least one rate's difference interval lies wholly
    above zero with audit-off higher.
    """
    by_rate: dict = {}
    for cells, arm in ((on_cells, "on"), (off_cells, "off")):
        for cell in cells:
            entry = by_rate.setdefault(
                cell["offered_per_s"], {"on": [], "off": [], "dropped": 0}
            )
            if cell_is_evidence(cell):
                entry[arm].append(cell["accepted_over_offered"])
            else:
                entry["dropped"] += 1

    rates = []
    separated = False
    comparable = 0
    for rate in sorted(by_rate):
        on_vals, off_vals = by_rate[rate]["on"], by_rate[rate]["off"]
        entry = {
            "offered_per_s": rate,
            "evidence_cells": {"audit_on": len(on_vals), "audit_off": len(off_vals)},
            "dropped_cells": by_rate[rate]["dropped"],
            "audit_on": confidence_interval(on_vals) if on_vals else None,
            "audit_off": confidence_interval(off_vals) if off_vals else None,
        }
        if len(on_vals) >= 2 and len(off_vals) >= 2:
            comparable += 1
            diff = difference_interval(off_vals, on_vals)
            entry["off_minus_on"] = diff
            if diff["excludes_zero"] and diff["diff"] > 0.0:
                separated = True
                entry["separated_here"] = True
        else:
            entry["off_minus_on"] = None
            entry["note"] = "fewer than two evidence cells in one arm"
        rates.append(entry)
    return {"separated": separated, "comparable_rates": comparable, "by_rate": rates}


def worker_rate(on_cells: list[dict]) -> dict:
    """Audit-worker completion rate, from cells where it means capacity.

    Only cells with a busy worker, steady outstanding work and no errors qualify:
    below saturation the completion rate tracks the offer, not the worker's limit.
    """
    usable = [
        c for c in on_cells
        if cell_is_evidence(c)
        and steady_state(c)
        and (c.get("audit_busy_fraction") or 0.0) >= BUSY_FRACTION_FOR_CAPACITY
    ]
    if not usable:
        return {
            "cells": 0,
            "estimate": None,
            "note": "no cell had a demonstrably busy worker in steady state",
        }
    return {
        "cells": len(usable),
        "estimate": confidence_interval([c["audit_completed_per_s"] for c in usable]),
        "service_ms": confidence_interval(
            [c["audit_service_mean_ms"] for c in usable]
        ),
        "note": (
            "audit-worker completion rate; not ingest capacity, and not the "
            "subject at all once the worker is fixed"
        ),
    }


def verdict(
    on_cells: list[dict], off_cells: list[dict] | None, control_ran: bool
) -> dict:
    """Whether the dataset supports the audit-attribution claim.

    Cells that cannot be evidence are excluded and reported, not treated as run
    failures. Only three things fail a run: a dead audit worker, a missing
    control, and too little surviving evidence to compare the arms.
    """
    all_cells = list(on_cells) + list(off_cells or [])
    failures = fatal_problems(all_cells)
    excluded = cell_exclusions(all_cells)

    if not control_ran:
        failures.append(
            "the audit-off control did not run: this dataset is a partial "
            "experiment and cannot attribute a ceiling to the audit path"
        )

    separation = (
        arm_separation(on_cells, off_cells)
        if control_ran and off_cells
        else {"separated": False, "comparable_rates": 0, "by_rate": []}
    )
    if control_ran and not separation["comparable_rates"]:
        failures.append(
            "no rate kept two evidence cells in both arms, so the arms cannot be "
            "compared at all: too much of this dataset was excluded"
        )
    elif control_ran and not separation["separated"]:
        failures.append(
            "no rate where audit-off exceeded audit-on with the difference "
            "interval excluding zero: the audit path is not shown to limit ingest"
        )

    return {
        "ok": not failures,
        "control": {"ran": control_ran, "arm": "audit_off"},
        "failures": failures,
        "excluded_cells": excluded,
        "arm_separation": separation,
        "worker_rate": worker_rate(on_cells),
        "lock_ceiling": (
            "structurally blind - the per-pod worker ceiling is lower and masks "
            "the per-community lock, so this dataset says nothing about the lock"
        ),
    }


# -- measurement ------------------------------------------------------------


def load_per_cpu() -> float:
    """1-minute load average per CPU.

    A lagging aggregate: sampled inside a short cell it is autocorrelated with
    the previous cell, so it can catch a sweep-long compile storm and cannot
    attribute load to any one cell.
    """
    return os.getloadavg()[0] / (os.cpu_count() or 1)


def conns_for(rate: float) -> int:
    return max(MIN_CONNS, int(math.ceil(rate / RATE_PER_CONN)))


def scrape(metrics_url: str) -> dict:
    """Audit and admission counters.

    Every series is created on first increment, so absent reads as zero. That is
    only safe because the quota series has been positive-controlled on this rig:
    it does appear and increment when the limiter binds.
    """
    with urllib.request.urlopen(metrics_url, timeout=10) as response:
        body = response.read().decode("utf-8", "replace")
    wanted = [
        ("buzz_audit_log_seconds_count", "audit_count"),
        ("buzz_audit_log_seconds_sum", "audit_sum"),
        ("buzz_audit_log_errors_total", "audit_log_errors"),
        ("buzz_audit_send_errors_total", "audit_send_errors"),
        ('buzz_admission_rejections_total{transport="websocket",reason="quota"}', "quota"),
        ('buzz_admission_rejections_total{transport="websocket",reason="unavailable"}',
         "unavailable"),
    ]
    out = {name: 0.0 for _, name in wanted}
    for line in body.splitlines():
        for needle, name in wanted:
            if line.startswith(needle + " "):
                out[name] = float(line.split()[-1])
    return out


def run_generator(rig: dict, duration: int, offers: list) -> dict:
    specs = []
    for index, rate in offers:
        target = rig["targets"][index]
        specs.append(
            "url={},channel={},rate={},conns={}".format(
                target["url"], target["channel"], rate, conns_for(rate)
            )
        )
    env = dict(os.environ, BENCH_PRIVATE_KEY=rig["bench_private_key"])
    for stale in ("BUZZ_AUTH_TAG", "BUZZ_RELAY_URL", "BUZZ_PRIVATE_KEY"):
        env.pop(stale, None)
    try:
        # stderr is deliberately left on the inherited handle: capturing it would
        # bury the generator's error context inside the exception, and a bare
        # CalledProcessError is not diagnosable in the field.
        completed = subprocess.run(
            [rig["generator"], str(duration)] + specs,
            env=env,
            cwd=rig.get("repo_root") or ".",
            stdout=subprocess.PIPE,
            check=True,
        )
    except subprocess.CalledProcessError as e:
        raise SystemExit(
            "generator failed (exit {}); its output is above".format(e.returncode)
        )
    return json.loads(completed.stdout)


def run_cell(rig: dict, duration: int, offers: list, audit_on: bool) -> dict:
    before = scrape(rig["metrics_url"])
    load_before = load_per_cpu()
    result = run_generator(rig, duration, offers)
    after = scrape(rig["metrics_url"])

    agg = result["aggregate"]
    window = result["elapsed_secs"]
    completed = after["audit_count"] - before["audit_count"]
    service_sum = after["audit_sum"] - before["audit_sum"]
    accepted = agg["accepted"]

    service_means = [
        t["service_mean_ms"] for t in result["targets"] if t.get("service_mean_ms")
    ]
    conns = sum(t["conns"] for t in result["targets"])
    headroom = None
    if service_means and agg["offered_per_s"]:
        # Closed-loop bound on what the generator could have offered, so the
        # sweep does not end up measuring the generator. Mean service demand,
        # not a median: closed-loop throughput depends on the mean, and a median
        # understates it on a skewed distribution.
        headroom = conns * (1000.0 / mean(service_means)) / agg["offered_per_s"]

    cell = {
        "offered_per_s": agg["offered_per_s"],
        "audit_enabled": audit_on,
        "duration_secs": duration,
        "elapsed_secs": window,
        "accepted": accepted,
        "accepted_per_s": accepted / window if window else None,
        "accepted_over_offered": agg["achieved_over_offered"],
        "rejected": agg["rejected"],
        "transport_errors": sum(
            1 for t in result["targets"] if t["first_transport_error"]
        ),
        "quota_rejections_delta": int(after["quota"] - before["quota"]),
        "unavailable_rejections_delta": int(
            after["unavailable"] - before["unavailable"]
        ),
        "audit_log_errors_delta": int(
            after["audit_log_errors"] - before["audit_log_errors"]
        ),
        "audit_send_errors_delta": int(
            after["audit_send_errors"] - before["audit_send_errors"]
        ),
        "generator_headroom": headroom,
        "load_per_cpu_before": load_before,
        "load_per_cpu_after": load_per_cpu(),
        "service_ms": agg["service_ms"],
        "scheduled_ms": agg["scheduled_ms"],
    }

    if audit_on:
        cell["audit_completed"] = int(completed)
        cell["audit_completed_per_s"] = completed / window if window else None
        cell["audit_service_mean_ms"] = (
            service_sum / completed * 1000.0 if completed else None
        )
        # Share of the window the worker spent inside a timed `audit.log`. This is
        # what "the completion rate agrees with 1/mean" actually measures: those
        # are C/T and C/S over the same count, so their ratio is exactly S/T.
        cell["audit_busy_fraction"] = service_sum / window if window else None
        # accepted - completed = queued + in flight, valid only with both audit
        # error deltas at zero and this generator as the sole producer.
        cell["outstanding_delta"] = int(accepted - completed)
    else:
        cell["audit_completed_per_s"] = None
        cell["audit_service_mean_ms"] = None
        cell["audit_busy_fraction"] = None
        cell["outstanding_delta"] = None
        cell["audit_note"] = "audit disabled: no worker, so no completion series"
    return cell


def sweep(
    rig: dict,
    rates: list[float],
    duration: int,
    repeats: int,
    two_community: bool,
    audit_on: bool,
    log: Callable[[str], None],
) -> list:
    cells = []
    for rate in rates:
        offers = [(0, rate / 2.0), (1, rate / 2.0)] if two_community else [(0, rate)]
        for repeat in range(repeats):
            cell = run_cell(rig, duration, offers, audit_on)
            cells.append(cell)
            log(
                "  {:>6.0f}/s r{}  accepted {:.4f}  completed {}  outstanding {}"
                "  busy {}  svc_p50 {}".format(
                    rate,
                    repeat + 1,
                    cell["accepted_over_offered"],
                    _fmt(cell["audit_completed_per_s"], "{:.1f}/s"),
                    _fmt(cell["outstanding_delta"], "{}"),
                    _fmt(cell["audit_busy_fraction"], "{:.3f}"),
                    _fmt(cell["service_ms"]["p50"], "{:.2f}ms"),
                )
            )
    return cells


def _fmt(value, spec: str) -> str:
    """Format a value that is legitimately absent.

    Percentiles are null when every connection died before its first settled
    send, and the audit series is null on the audit-off arm. Formatting those
    directly aborts the sweep mid-run with no report written.
    """
    return "n/a" if value is None else spec.format(value)


def experiment_identity(rig: dict, args: argparse.Namespace) -> dict:
    """What two half-runs must agree on before they may be combined."""
    return {
        "rates": args.rates,
        "duration_secs": args.duration,
        "repeats": args.repeats,
        "targets": [t["community_host"] for t in rig["targets"]],
        "ws_events_per_sec_limit": rig["ws_events_per_sec_limit"],
        "messages_per_min_limit": rig["messages_per_min_limit"],
        "generator": rig["generator"],
        "source_revision": rig.get("source_revision"),
    }


def measure(args: argparse.Namespace, log: Callable[[str], None]) -> dict:
    with open(args.rig) as handle:
        rig = json.load(handle)
    audit_on = bool(rig["audit_enabled"])

    log("Sweep, audit {}, one community".format("enabled" if audit_on else "disabled"))
    cells = sweep(rig, args.rates, args.duration, args.repeats, False, audit_on, log)

    two_cells = []
    if audit_on and not args.skip_two_community:
        log("Sweep, audit enabled, two communities at half rate each")
        two_cells = sweep(rig, args.rates, args.duration, args.repeats, True, True, log)
        # Report-only by agreed scope, but annotated: an unannotated contaminated
        # cell in a table nobody judges is how a bad number gets quoted later.
        for cell in two_cells:
            cell["problems"] = cell_problems(cell)
            cell["steady"] = steady_state(cell)

    # A single-arm dataset is never a verdict; --combine judges the pair.
    if audit_on:
        result = verdict(cells, None, control_ran=False)
    else:
        result = {
            "ok": False,
            "control": {"ran": False, "arm": "this dataset is itself the audit-off arm"},
            "failures": [
                "audit-off half-run: combine it with the audit-on half to get a verdict"
            ],
        }
    return {
        "identity": experiment_identity(rig, args),
        "audit_enabled": audit_on,
        "partial": True,
        "cells": cells,
        "two_community_cells": two_cells,
        "verdict": result,
    }


def combine(first: dict, second: dict) -> dict:
    """Judge one audit-on and one audit-off half-run together."""
    if first["audit_enabled"] == second["audit_enabled"]:
        raise ValueError(
            "combine needs one audit-on and one audit-off report; both say "
            "audit_enabled={}".format(first["audit_enabled"])
        )
    on_report, off_report = (
        (first, second) if first["audit_enabled"] else (second, first)
    )

    mismatched = sorted(
        key for key in on_report["identity"]
        if on_report["identity"][key] != off_report["identity"].get(key)
    )
    if mismatched:
        raise ValueError(
            "the two halves are not the same experiment; differing: "
            + ", ".join(mismatched)
        )

    threshold = 0.99
    return {
        "mode": "combine",
        "identity": on_report["identity"],
        "verdict": verdict(on_report["cells"], off_report["cells"], control_ran=True),
        "knee_reporting_only": {
            "threshold": threshold,
            "audit_on": knee_bracket(
                [(c["offered_per_s"], c["accepted_over_offered"])
                 for c in on_report["cells"]],
                threshold,
            ),
            "audit_off": knee_bracket(
                [(c["offered_per_s"], c["accepted_over_offered"])
                 for c in off_report["cells"]],
                threshold,
            ),
            "note": "presentation only; nothing gates on the knee",
        },
    }


def model() -> dict:
    """Deterministic queueing arithmetic, no services.

    Documents the contract's shape, and deliberately reproduces the physics the
    exclusion rule exists for: the audit channel starts empty, so a saturating
    rate banks acceptance credit on its first repeats and only reaches steady
    state once the channel is full. An earlier version of this model set
    `outstanding_delta` to zero at every rate, including one above its own
    ceiling — which made the green path a test of steady cells rather than of the
    behaviour the harness meets in the field.

    For review and for the unit tests, never as evidence.
    """
    ceiling = 1000.0 / 3.0
    duration = 20.0
    repeats = 5
    # 350/s sits just above the ceiling, so it fills the channel over several
    # repeats; 400/s overshoots far enough to bank the whole depth at once.
    rates = [20.0, 50.0, 100.0, 200.0, 350.0, 400.0]

    def jitter(repeat: int) -> float:
        return (repeat - (repeats - 1) / 2.0) * 0.002

    def audit_on_cells() -> list:
        cells = []
        fill = 0.0
        for rate in rates:
            offered = rate * duration
            drain_capacity = ceiling * duration
            for repeat in range(repeats):
                headroom = AUDIT_CHANNEL_DEPTH - fill
                accepted = min(offered, drain_capacity + headroom)
                drained = min(drain_capacity, fill + accepted)
                delta = accepted - drained
                fill += delta
                cells.append(
                    {
                        "offered_per_s": rate,
                        "audit_enabled": True,
                        "accepted_over_offered": min(
                            1.0, accepted / offered * (1.0 + jitter(repeat))
                        ),
                        "accepted_per_s": accepted / duration,
                        "rejected": 0,
                        "transport_errors": 0,
                        "quota_rejections_delta": 0,
                        "unavailable_rejections_delta": 0,
                        "audit_log_errors_delta": 0,
                        "audit_send_errors_delta": 0,
                        "generator_headroom": 4.0,
                        "audit_completed_per_s": drained / duration,
                        "audit_service_mean_ms": 1000.0 / ceiling,
                        "audit_busy_fraction": min(1.0, drained / drain_capacity),
                        "outstanding_delta": int(round(delta)),
                    }
                )
        return cells

    def audit_off_cells() -> list:
        return [
            {
                "offered_per_s": rate,
                "audit_enabled": False,
                "accepted_over_offered": min(1.0, 1.0 * (1.0 + jitter(repeat))),
                "accepted_per_s": rate,
                "rejected": 0,
                "transport_errors": 0,
                "quota_rejections_delta": 0,
                "unavailable_rejections_delta": 0,
                "audit_log_errors_delta": 0,
                "audit_send_errors_delta": 0,
                "generator_headroom": 4.0,
                "audit_completed_per_s": None,
                "audit_service_mean_ms": None,
                "audit_busy_fraction": None,
                "outstanding_delta": None,
            }
            for rate in rates
            for repeat in range(repeats)
        ]

    return {
        "mode": "model",
        "verdict": verdict(audit_on_cells(), audit_off_cells(), control_ran=True),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("measure", "model"), default="measure")
    parser.add_argument(
        "--combine",
        nargs=2,
        metavar="REPORT",
        help="judge one audit-on and one audit-off report written by --json",
    )
    parser.add_argument("--rig", help="rig JSON from scripts/start-perf-ingest-rig.sh")
    parser.add_argument("--duration", type=int, default=20)
    parser.add_argument("--repeats", type=int, default=DEFAULT_REPEATS)
    parser.add_argument("--rates", default=",".join(str(r) for r in DEFAULT_RATES))
    parser.add_argument("--skip-two-community", action="store_true")
    parser.add_argument("--json", help="write the full report here")
    args = parser.parse_args(argv)
    args.rates = [float(r) for r in args.rates.split(",")]

    def log(message: str) -> None:
        print(message, file=sys.stderr)

    if args.combine:
        with open(args.combine[0]) as a, open(args.combine[1]) as b:
            report = combine(json.load(a), json.load(b))
    elif args.mode == "model":
        report = model()
    else:
        if not args.rig:
            parser.error("--mode measure needs --rig")
        if args.repeats < 2:
            parser.error("--repeats must be at least 2; an interval needs a spread")
        report = measure(args, log)

    if args.json:
        with open(args.json, "w") as handle:
            json.dump(report, handle, indent=2)
    print(json.dumps(report["verdict"], indent=2))

    if not report["verdict"]["ok"]:
        for failure in report["verdict"]["failures"]:
            print("NOT ESTABLISHED: " + failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
