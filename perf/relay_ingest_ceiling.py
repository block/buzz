#!/usr/bin/env python3
"""Ingest-ceiling harness for the Buzz relay's audit write path.

Measures where accepted-event throughput stops tracking the offered rate, and
whether the audit log is what stops it. Stdlib only; the measurement itself is
done by `ingest_load` (Rust) and this script owns the experiment and the verdict.

Two ceilings are under test and they coincide numerically (both ~1/(6*RTT)):

  * the per-pod audit worker  — one task draining all communities serially
  * the per-community lock    — 6 round trips under a DB-global advisory lock

The minimum of the two always wins, and it is always the worker, so a first
sweep is *structurally blind* to the lock ceiling. A run that does not surface
the lock is not evidence the lock is fine. Exposing it needs a second round
after the worker is fixed. See perf/RELAY_INGEST_CEILING.md.

Usage:
  ./scripts/start-perf-ingest-rig.sh --reset > /tmp/rig.json
  ./perf/relay_ingest_ceiling.py --rig /tmp/rig.json

  ./perf/relay_ingest_ceiling.py --mode model     # verdict logic, no services

Exits non-zero when a run is invalid or the contract is violated.
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
# cannot exceed one send per service latency; this keeps the generator's own
# capacity well clear of the offer. `conn_capacity_per_s` in the output is the
# check that it worked.
RATE_PER_CONN = 25.0
MIN_CONNS = 4

DEFAULT_RATES = [20.0, 50.0, 100.0, 200.0, 400.0]


# ── verdict logic (pure; unit-tested in test_relay_ingest_ceiling.py) ────────


def relative_spread(values: list[float]) -> float:
    """Spread of repeated identical runs, relative to their mean.

    This is the harness's noise floor. The knee threshold is derived from it
    rather than asserted, so a noisy machine widens the tolerance instead of
    manufacturing a knee.
    """
    if len(values) < 2:
        raise ValueError("relative spread needs at least two runs")
    mean = sum(values) / len(values)
    if mean <= 0.0:
        raise ValueError("relative spread needs a positive mean")
    return (max(values) - min(values)) / mean


def knee_threshold(spread: float) -> float:
    """Delivered-fraction floor below which a point counts as saturated."""
    return 1.0 - 3.0 * spread


def find_knee(points: list[tuple[float, float]], threshold: float) -> float | None:
    """Lowest offered rate whose shortfall persists.

    `points` is [(offered_rate, delivered_fraction)], ascending by rate. A knee
    must hold at the next higher rate too: saturation is monotone, so a lone dip
    is noise rather than a ceiling. The highest rate is allowed to stand alone
    because it has no successor to confirm it.
    """
    ordered = sorted(points)
    for idx, (rate, fraction) in enumerate(ordered):
        if fraction >= threshold:
            continue
        is_last = idx == len(ordered) - 1
        if is_last or ordered[idx + 1][1] < threshold:
            return rate
    return None


def knee_bracket(
    points: list[tuple[float, float]], threshold: float
) -> tuple[float | None, float | None]:
    """The interval the ceiling lies in: (highest passing rate, knee).

    A sweep only brackets the ceiling between the last rate it met and the first
    it did not. Reporting the knee alone invites reading a grid point as a
    measurement; a finer grid narrows the bracket.
    """
    knee = find_knee(points, threshold)
    if knee is None:
        return (None, None)
    passing = [rate for rate, fraction in sorted(points) if rate < knee and fraction >= threshold]
    return (passing[-1] if passing else None, knee)


def verdict(
    audit_on: list[tuple[float, float]],
    audit_off: list[tuple[float, float]] | None,
    spread: float,
    quota_moved: bool,
    audit_rows_grew_on: bool,
    audit_rows_grew_off: bool,
) -> dict[str, object]:
    """Decide whether the run supports the audit-ceiling claim.

    `audit_on`/`audit_off` are [(rate, delivered_fraction)]. Returns a dict with
    `ok` plus every reason it failed, so one run reports all its problems.
    """
    threshold = knee_threshold(spread)
    knee_on = find_knee(audit_on, threshold)
    knee_off = find_knee(audit_off, threshold) if audit_off else None

    failures = []
    if quota_moved:
        failures.append(
            "admission quota rejections increased during the run: the limiter "
            "was measured, not the relay"
        )
    if not audit_rows_grew_on:
        failures.append("audit_log did not grow with audit enabled: the subject was not exercised")
    if audit_off is not None and audit_rows_grew_off:
        failures.append("audit_log grew with audit disabled: the control did not take effect")
    if knee_on is None:
        failures.append(
            "no knee with audit enabled up to the highest offered rate: the audit "
            "path is not the ingest ceiling at these rates"
        )
    if audit_off is not None and knee_on is not None:
        if knee_off is not None and knee_off <= knee_on:
            failures.append(
                f"knee did not move when audit was disabled ({knee_off} <= {knee_on}): "
                "something other than the audit path is the ceiling"
            )

    return {
        "ok": not failures,
        "failures": failures,
        "null_control_spread": spread,
        "knee_threshold": threshold,
        "knee_audit_on": knee_on,
        "knee_audit_off": knee_off,
        "ceiling_bracket_audit_on": knee_bracket(audit_on, threshold),
        "ceiling_bracket_audit_off":
            knee_bracket(audit_off, threshold) if audit_off else (None, None),
        "lock_ceiling": "structurally blind — the worker ceiling is lower and masks it",
    }


# ── measurement ─────────────────────────────────────────────────────────────


def load_per_cpu() -> float:
    """1-minute load average per CPU.

    A sweep shares the machine with whatever else is running on it. The null
    control only absorbs load that is steady across two adjacent runs, so record
    this per run: a drifting load is invisible to the control and looks like a
    ceiling.
    """
    return os.getloadavg()[0] / (os.cpu_count() or 1)


def conns_for(rate: float) -> int:
    return max(MIN_CONNS, int(math.ceil(rate / RATE_PER_CONN)))


def run_generator(
    rig: dict, duration: int, offers: list[tuple[int, float]]
) -> dict:
    """Run one measurement. `offers` is [(target_index, rate)]."""
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
    completed = subprocess.run(
        [rig["generator"], str(duration)] + specs,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return json.loads(completed.stdout)


def quota_rejections(metrics_url: str) -> int:
    """Websocket quota rejections so far, or 0 while the series is absent.

    Scoped to reason="quota": reason="unavailable" means the limiter itself was
    unreachable, which is a different diagnosis and should not be reported as
    limiter contamination.
    """
    needle = 'buzz_admission_rejections_total{transport="websocket",reason="quota"}'
    with urllib.request.urlopen(metrics_url, timeout=10) as response:
        body = response.read().decode("utf-8", "replace")
    for line in body.splitlines():
        if line.startswith(needle):
            return int(float(line.split()[-1]))
    return 0


def audit_log_rows(rig: dict) -> int:
    completed = subprocess.run(
        [
            "docker", "compose", "-p", rig["compose_project"],
            "-f", "docker-compose.harness.yml", "exec", "-T", "postgres",
            "psql", "-U", "buzz", "-d", "buzz", "-qtA", "-c",
            "SELECT count(*) FROM audit_log",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return int(completed.stdout.decode().strip())


def sweep(
    rig: dict,
    rates: list[float],
    duration: int,
    two_community: bool,
    log: Callable[[str], None],
) -> tuple[list[tuple[float, float]], list[dict]]:
    """Run each rate once and return [(rate, delivered_fraction)] plus raw runs."""
    points, raw = [], []
    for rate in rates:
        offers = [(0, rate / 2.0), (1, rate / 2.0)] if two_community else [(0, rate)]
        before = quota_rejections(rig["metrics_url"])
        result = run_generator(rig, duration, offers)
        after = quota_rejections(rig["metrics_url"])
        fraction = result["aggregate"]["achieved_over_offered"]
        result["quota_rejections_delta"] = after - before
        result["load_per_cpu"] = load_per_cpu()
        points.append((rate, fraction))
        raw.append(result)
        log(
            "  offered {:>6.0f}/s  delivered {:.4f}  svc_p50 {:.2f}ms  "
            "svc_p99 {:.2f}ms  quota_delta {}  load/cpu {:.2f}".format(
                rate,
                fraction,
                result["aggregate"]["service_ms"]["p50"],
                result["aggregate"]["service_ms"]["p99"],
                after - before,
                result["load_per_cpu"],
            )
        )
    return points, raw


def measure(args: argparse.Namespace, log: Callable[[str], None]) -> dict:
    with open(args.rig) as handle:
        rig_on = json.load(handle)

    log("Null control: the lowest rate twice, to measure this machine's spread")
    control = [
        run_generator(rig_on, args.duration, [(0, args.rates[0])])["aggregate"]["achieved_per_s"]
        for _ in range(2)
    ]
    spread = relative_spread(control)
    log("  achieved {:.3f}/s and {:.3f}/s -> spread {:.5f}, threshold {:.5f}".format(
        control[0], control[1], spread, knee_threshold(spread)
    ))

    rows_before = audit_log_rows(rig_on)
    log("Sweep, audit enabled, one community")
    on_points, on_raw = sweep(rig_on, args.rates, args.duration, False, log)
    rows_after = audit_log_rows(rig_on)
    log("  audit_log rows {} -> {}".format(rows_before, rows_after))

    log("Sweep, audit enabled, two communities at half rate each")
    two_points, two_raw = sweep(rig_on, args.rates, args.duration, True, log)

    off_points, off_raw, off_grew = None, [], False
    if not args.skip_audit_off:
        log("Restarting the rig with audit disabled (attribution control)")
        rig_off = json.loads(
            subprocess.run(
                ["./scripts/start-perf-ingest-rig.sh", "--audit", "off"],
                stdout=subprocess.PIPE,
                check=True,
            ).stdout
        )
        off_rows_before = audit_log_rows(rig_off)
        log("Sweep, audit disabled, one community")
        off_points, off_raw = sweep(rig_off, args.rates, args.duration, False, log)
        off_rows_after = audit_log_rows(rig_off)
        off_grew = off_rows_after > off_rows_before
        log("  audit_log rows {} -> {}".format(off_rows_before, off_rows_after))

    quota_moved = any(
        run["quota_rejections_delta"] > 0 for run in on_raw + two_raw + off_raw
    )

    return {
        "rig": {key: rig_on[key] for key in
                ("audit_enabled", "ws_events_per_sec_limit", "messages_per_min_limit")},
        "audit_enabled": rig_on["audit_enabled"],
        "audit_rows_grew": rows_after > rows_before,
        "quota_moved": quota_moved,
        "duration_secs": args.duration,
        "load_per_cpu_at_start": load_per_cpu(),
        "rates": args.rates,
        "null_control_achieved_per_s": control,
        "audit_on": on_points,
        "audit_on_two_community": two_points,
        "audit_off": off_points,
        "runs": {"audit_on": on_raw, "two_community": two_raw, "audit_off": off_raw},
        "verdict": verdict(
            on_points,
            off_points,
            spread,
            quota_moved=quota_moved,
            audit_rows_grew_on=rows_after > rows_before,
            audit_rows_grew_off=off_grew,
        ),
    }


def model() -> dict:
    """Deterministic arithmetic, no services — documents the contract's shape.

    A 3ms audit write serialized behind one worker caps accepted throughput near
    333/s; with audit off the same offers are met. Used for review and by the
    unit tests, never as evidence.
    """
    ceiling = 1000.0 / 3.0
    on = [(rate, min(1.0, ceiling / rate)) for rate in DEFAULT_RATES]
    off = [(rate, 1.0) for rate in DEFAULT_RATES]
    spread = 0.002
    return {
        "mode": "model",
        "audit_on": on,
        "audit_off": off,
        "verdict": verdict(
            on, off, spread,
            quota_moved=False,
            audit_rows_grew_on=True,
            audit_rows_grew_off=False,
        ),
    }


def combine(on_report: dict, off_report: dict) -> dict:
    """Re-verdict two half-runs measured against separately supervised relays.

    Both relays bind the same port, so audit-on and audit-off cannot be up at
    once. Splitting the run also lets a saved pair be re-judged without
    re-measuring.
    """
    if on_report["audit_enabled"] == off_report["audit_enabled"]:
        raise SystemExit(
            "combine needs one audit-on and one audit-off report; both say "
            f"audit_enabled={on_report['audit_enabled']}"
        )
    if not on_report["audit_enabled"]:
        on_report, off_report = off_report, on_report
    spread = relative_spread(on_report["null_control_achieved_per_s"])
    return {
        "mode": "combine",
        "duration_secs": on_report["duration_secs"],
        "rates": on_report["rates"],
        "audit_on": [tuple(point) for point in on_report["audit_on"]],
        "audit_off": [tuple(point) for point in off_report["audit_on"]],
        "verdict": verdict(
            [tuple(point) for point in on_report["audit_on"]],
            [tuple(point) for point in off_report["audit_on"]],
            spread,
            quota_moved=on_report["quota_moved"] or off_report["quota_moved"],
            audit_rows_grew_on=on_report["audit_rows_grew"],
            audit_rows_grew_off=off_report["audit_rows_grew"],
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("measure", "model"), default="measure")
    parser.add_argument(
        "--combine",
        nargs=2,
        metavar=("REPORT", "REPORT"),
        help="re-verdict one audit-on and one audit-off report from --json",
    )
    parser.add_argument("--rig", help="rig JSON from scripts/start-perf-ingest-rig.sh")
    parser.add_argument("--duration", type=int, default=20)
    parser.add_argument("--rates", type=str, default=",".join(str(r) for r in DEFAULT_RATES))
    parser.add_argument("--skip-audit-off", action="store_true")
    parser.add_argument("--json", help="write the full report here")
    args = parser.parse_args(argv)
    args.rates = [float(r) for r in args.rates.split(",")]

    def log(message: str) -> None:
        print(message, file=sys.stderr)

    if args.combine:
        with open(args.combine[0]) as first, open(args.combine[1]) as second:
            report = combine(json.load(first), json.load(second))
    elif args.mode == "model":
        report = model()
    else:
        if not args.rig:
            parser.error("--mode measure needs --rig")
        report = measure(args, log)

    if args.json:
        with open(args.json, "w") as handle:
            json.dump(report, handle, indent=2)
    print(json.dumps(report["verdict"], indent=2))

    if not report["verdict"]["ok"]:
        for failure in report["verdict"]["failures"]:
            print("CONTRACT VIOLATED: " + failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
