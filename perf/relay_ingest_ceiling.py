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
import time
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

# The audit-off arm must hold its offer, not merely beat the audit-on arm. Both
# arms collapsing is not "audit removal restores ingest".
CONTROL_EQUIVALENCE_MARGIN = 0.05

# Counter deltas are read around the whole subprocess, but the generator's own
# window starts only after every connection is authenticated. A cell is rejected
# when that setup overhead is a material share of the window, because rates
# divided by mismatched windows can exceed 1.0 and overstate completions.
MAX_SETUP_OVERHEAD_FRACTION = 0.05

# Share of its scheduled slots the generator must actually have sent. Signing and
# scheduler delay are outside `service_ms` by design, so a CPU-bound generator can
# miss slots while the on-wire headroom gate still passes.
MIN_ATTEMPTED_FRACTION = 0.98

# The grid has to straddle the drain rate, or the separation contract has nothing
# to fire on and the run reports "the audit path is not shown to limit ingest" —
# a false negative phrased as a conclusion. Every drain figure measured on this
# rig falls in 390-495/s, so 100-200 anchor the unsaturated arm, 400 brackets the
# ceiling from below, and 800/1600 are in clear saturation. Re-derive this if the
# rig's drain rate moves: a grid whose top rate sits at the ceiling makes the
# only informative cell a coin flip.
DEFAULT_RATES = [100.0, 200.0, 400.0, 800.0, 1600.0]
DEFAULT_REPEATS = 5

# Drain rate observed on this rig, used only to keep `model()` honest about where
# its ceiling sits relative to the grid. Not a capacity claim.
MEASURED_DRAIN_BAND_PER_S = 450.0

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
    """Two-tailed 95% critical value, rounded to the conservative side.

    The table is sparse, so an exact df is often missing. Taking the next *higher*
    stored df would pick a *smaller* critical value and under-cover: df 11 would
    get 2.179 against a true 2.201, and df 13 would get 2.131 against 2.160. Take
    the largest stored df at or below the real one instead, which errs wide.
    """
    if df < 1:
        raise ValueError("t95 needs at least one degree of freedom")
    key = int(math.floor(df))
    usable = [cutoff for cutoff in _T95 if cutoff <= key]
    return _T95[max(usable)] if usable else _T95[min(_T95)]


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

    if cell.get("counters_window_aligned") is False:
        problems.append(
            "{:g}/s: audit counters were sampled around the whole subprocess "
            "rather than the timed window, so completion and outstanding-work "
            "readings are not comparable with the rates".format(rate)
        )

    overhead = cell.get("setup_overhead_fraction")
    if overhead is not None and overhead > MAX_SETUP_OVERHEAD_FRACTION:
        problems.append(
            "{:g}/s: setup and teardown were {:.1%} of the window, over the {:.0%} "
            "bound, so window-edge readings are unreliable".format(
                rate, overhead, MAX_SETUP_OVERHEAD_FRACTION
            )
        )

    if cell.get("audit_activity_in_control_arm"):
        problems.append(
            "{:g}/s: the audit series moved by {} in the audit-off arm, so that "
            "relay was still auditing and the control is not a control".format(
                rate, cell["audit_activity_in_control_arm"]
            )
        )

    attempted = cell.get("attempted_over_offered")
    if attempted is not None and attempted < MIN_ATTEMPTED_FRACTION:
        problems.append(
            "{:g}/s: the generator sent only {:.1%} of its scheduled slots, so it "
            "missed its own offer before the relay saw it".format(rate, attempted)
        )

    headroom = cell.get("generator_headroom")
    if headroom is not None and headroom < GENERATOR_HEADROOM_MARGIN:
        problems.append(
            "{:g}/s: generator headroom {:.2f}x is under {}x, so the cell is "
            "partly measuring the generator".format(
                rate, headroom, GENERATOR_HEADROOM_MARGIN
            )
        )
    return problems


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
        # Fatal problems are reported as run failures; repeating them here would
        # print the same sentence twice under two different headings.
        reasons = [
            r for r in cell_problems(cell)
            if not any(key in r for key in FATAL_PROBLEM_KEYS)
        ]
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
    contradicted = []
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
            elif diff["excludes_zero"] and diff["diff"] < 0.0:
                # Audit-off did significantly *worse*. That contradicts the
                # hypothesis rather than failing to support it.
                contradicted.append(rate)
                entry["contradicted_here"] = True
        else:
            entry["off_minus_on"] = None
            entry["note"] = "fewer than two evidence cells in one arm"
        rates.append(entry)

    # One predeclared primary contrast, at the highest rate that produced a
    # comparison. Passing on "any of N rates" runs an unadjusted test per rate:
    # with five rates at a two-sided 95% interval the false-pass rate is ~10%,
    # measured on this code with identical populations, not the nominal 5%.
    comparisons = [e for e in rates if e.get("off_minus_on")]
    primary = comparisons[-1] if comparisons else None
    primary_separated = bool(primary and primary.get("separated_here"))

    control_holds = None
    if primary and primary["audit_off"]:
        lo = primary["audit_off"]["lo"]
        control_holds = lo is not None and lo >= 1.0 - CONTROL_EQUIVALENCE_MARGIN

    return {
        "separated": separated,
        "comparable_rates": comparable,
        "primary_rate": primary["offered_per_s"] if primary else None,
        "primary_separated": primary_separated,
        "primary_control_holds_offer": control_holds,
        "contradicted_rates": contradicted,
        "secondary_separated_rates": [
            e["offered_per_s"] for e in rates
            if e.get("separated_here") and e is not primary
        ],
        "by_rate": rates,
    }


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
    by_rate: dict = {}
    for c in usable:
        by_rate.setdefault(c["offered_per_s"], []).append(c)
    return {
        "cells": len(usable),
        # Per rate, not pooled: different offered rates are different load and
        # database regimes, not repeats of one estimand.
        "per_rate": [
            {
                "offered_per_s": rate,
                "completed_per_s": confidence_interval(
                    [c["audit_completed_per_s"] for c in cells]
                ),
                "service_ms": confidence_interval(
                    [c["audit_service_mean_ms"] for c in cells]
                ),
            }
            for rate, cells in sorted(by_rate.items())
        ],
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

    busy = [c.get("audit_busy_fraction") or 0.0 for c in on_cells if cell_is_evidence(c)]
    max_busy = max(busy) if busy else 0.0
    any_saturated = max_busy >= BUSY_FRACTION_FOR_CAPACITY

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
    # These states carry different program consequences, so the run must say
    # which one it is in. "Compared and found nothing" exonerates the audit path
    # and would stop the work; "never saturated" or "lost the informative cells"
    # mean re-run. A single message covering all three makes the loudest line the
    # one a reader acts on hardest, and it is only correct for the first.
    uncomparable = [
        entry["offered_per_s"]
        for entry in separation.get("by_rate", [])
        if entry.get("off_minus_on") is None and entry["dropped_cells"]
    ]
    if control_ran:
        if not separation["comparable_rates"]:
            failures.append(
                "no rate kept two evidence cells in both arms, so the arms were "
                "never compared: too much of this dataset was excluded to conclude "
                "anything"
            )
        elif not any_saturated:
            failures.append(
                "inconclusive, not negative: no audit-on cell reached a busy "
                "worker (max busy fraction {:.2f} against the {:.2f} gate), so "
                "this grid never saturated the audit path and cannot exonerate "
                "it. Extend the rates above the drain rate and re-run".format(
                    max_busy, BUSY_FRACTION_FOR_CAPACITY
                )
            )
        elif separation["contradicted_rates"]:
            failures.append(
                "audit-off was significantly *worse* than audit-on at {}: that "
                "contradicts the hypothesis rather than failing to support it, "
                "and no separation elsewhere can be read past it".format(
                    ", ".join(
                        "{:g}/s".format(r) for r in separation["contradicted_rates"]
                    )
                )
            )
        elif uncomparable and not separation["separated"]:
            failures.append(
                "inconclusive rather than negative: {} rate(s) ({}) lost their "
                "evidence to exclusions, so a missing separation cannot be told "
                "apart from missing data. A bracket-refinement run near the "
                "ceiling drops its most informative cells exactly this way".format(
                    len(uncomparable),
                    ", ".join("{:g}".format(r) for r in uncomparable),
                )
            )
        elif not separation["separated"]:
            failures.append(
                "every rate was comparable and none separated: the audit path is "
                "not shown to limit ingest at these rates"
            )
        elif not separation["primary_separated"]:
            failures.append(
                "only secondary rates separated; the predeclared primary contrast "
                "at {:g}/s did not. Passing on any-of-N rates runs an unadjusted "
                "test per rate: with five rates at a two-sided 95% interval that "
                "is a ~10% false-pass rate, measured on this code with identical "
                "populations".format(separation["primary_rate"])
            )
        elif separation["primary_control_holds_offer"] is False:
            failures.append(
                "the audit-off arm did not hold its offer at the primary contrast "
                "({:g}/s): a control that also collapsed does not show that "
                "removing the audit path restores ingest, however large the gap "
                "between the arms".format(separation["primary_rate"])
            )
        elif uncomparable:
            failures.append(
                "separation was found, but {} rate(s) ({}) lost their evidence to "
                "exclusions and contributed nothing".format(
                    len(uncomparable),
                    ", ".join("{:g}".format(r) for r in uncomparable),
                )
            )

    return {
        "ok": not failures,
        "control": {"ran": control_ran, "arm": "audit_off"},
        "failures": failures,
        "excluded_cells": excluded,
        "saturation": {
            "max_busy_fraction": max_busy,
            "gate": BUSY_FRACTION_FOR_CAPACITY,
            "any_rate_saturated": any_saturated,
        },
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
    load_before = load_per_cpu()
    outer_before = scrape(rig["metrics_url"])
    outer_start = time.monotonic()
    result = run_generator(rig, duration, offers)
    outer_elapsed = time.monotonic() - outer_start
    outer_after = scrape(rig["metrics_url"])

    # Prefer the counters the generator sampled at its own timed-window edges.
    # The runner's own pair brackets the whole subprocess — connection setup and
    # teardown included — while every rate is divided by the post-connect window,
    # so backlog draining during setup lands in the delta but not the divisor.
    # That can push a busy fraction above 1.0 and overstate completions, and it
    # feeds the exclusion decision, so it can change the verdict rather than only
    # the estimate.
    aligned = bool(result.get("counters_before") and result.get("counters_after"))
    before = result["counters_before"] if aligned else outer_before
    after = result["counters_after"] if aligned else outer_after

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

    setup_overhead = max(0.0, outer_elapsed - window) / window if window else None

    cell = {
        "offered_per_s": agg["offered_per_s"],
        "audit_enabled": audit_on,
        "duration_secs": duration,
        "elapsed_secs": window,
        "outer_elapsed_secs": outer_elapsed,
        "setup_overhead_fraction": setup_overhead,
        "counters_window_aligned": aligned,
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
        "attempted_over_offered": (
            agg["attempted"] / (agg["offered_per_s"] * duration)
            if agg["offered_per_s"] else None
        ),
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
        # Recorded rather than assumed: the rig JSON saying audit is off is a
        # claim about how the relay was started, and with --skip-relay nobody
        # verified it. A moving audit series here means the control arm was
        # auditing after all, which would make the whole comparison meaningless.
        cell["audit_activity_in_control_arm"] = int(
            (after["audit_count"] - before["audit_count"])
            + (after["audit_log_errors"] - before["audit_log_errors"])
        )
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
        # Two dirty trees at the same commit are two different builds.
        "source_diff_digest": rig.get("source_diff_digest"),
        # A fixed audit-on-then-audit-off order against a database that grew in
        # between confounds arm with time, cache and index size. Restoring the
        # same snapshot at both arm boundaries makes the arms comparable; the
        # identity records it so a pair where only one arm was reset is rejected.
        "database_reset": rig.get("database_reset"),
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

    for report, arm in ((on_report, True), (off_report, False)):
        identity = report["identity"]
        expected = sorted(float(r) for r in identity["rates"])
        seen: dict = {}
        for cell in report["cells"]:
            if cell["audit_enabled"] != arm:
                raise ValueError(
                    "a cell labelled audit_enabled={} appears in the audit_enabled={} "
                    "report".format(cell["audit_enabled"], arm)
                )
            if cell.get("duration_secs") not in (None, identity["duration_secs"]):
                raise ValueError(
                    "a {:g}/s cell ran for {}s but the identity declares {}s".format(
                        cell["offered_per_s"],
                        cell["duration_secs"],
                        identity["duration_secs"],
                    )
                )
            seen[cell["offered_per_s"]] = seen.get(cell["offered_per_s"], 0) + 1
        if sorted(seen) != expected:
            raise ValueError(
                "the cells do not cover the declared rate grid: declared {}, "
                "present {}".format(expected, sorted(seen))
            )
        wrong = {r: n for r, n in seen.items() if n != identity["repeats"]}
        if wrong:
            raise ValueError(
                "the declared {} repeats are not present at every rate: {}".format(
                    identity["repeats"], wrong
                )
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


def model(
    ceiling_per_s: float = MEASURED_DRAIN_BAND_PER_S,
    rates: list[float] | None = None,
) -> dict:
    """Deterministic queueing arithmetic, no services.

    Documents the contract's shape, and deliberately reproduces the physics the
    exclusion rule exists for: the audit channel starts empty, so a saturating
    rate banks acceptance credit on its first repeats and only reaches steady
    state once the channel is full. An earlier version of this model set
    `outstanding_delta` to zero at every rate, including one above its own
    ceiling — which made the green path a test of steady cells rather than of the
    behaviour the harness meets in the field.

    The default ceiling is the drain rate measured on this rig, *not* a round
    number below the grid. An earlier version fixed it at 333/s while the grid
    topped out at 400/s, so the model ran at 120% utilization and separated
    cleanly while the rig would have sat at 81-102% and separated by coin flip.
    A fixture that cannot produce the failing input turns green into a statement
    about the fixture. `ceiling_per_s` is a parameter so a test can push it above
    the top grid rate and exercise the no-separation path.

    For review and for the unit tests, never as evidence.
    """
    ceiling = ceiling_per_s
    duration = 20.0
    repeats = 5
    rates = list(DEFAULT_RATES if rates is None else rates)

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
