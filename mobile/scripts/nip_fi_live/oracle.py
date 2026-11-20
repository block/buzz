#!/usr/bin/env python3
"""Oracle for the mobile NIP-FI live check: the relay's own access log.

The relay writes one JSON line per HTTP request (`event=http`) and per
WebSocket authentication (`event=auth`), each naming the auth `method`, the
`outcome` and a `reason`. This oracle needs no hooks in the app, so it covers
every request the app makes, including ones no test thought to drive.

The check fails when:

* the log is missing or holds no access lines, the relay crashed, or the test
  never wrote its step markers (no vacuous pass);
* a `method=nip_fi` denial happens outside the negative-control window;
* the negative control produced no denial (enforcement was never proven);
* a required route class never shows a success.

Usage: oracle.py <workdir>   (exit 0 pass, 1 fail)
"""

import argparse
import datetime
import json
import sys
from pathlib import Path

DENIED = {"denied", "failed"}

# Route classes the journey must reach with a success. Each is a predicate over
# one parsed access line; `community` is filled in from the fixture.
REQUIRED = (
    ("ws auth", "WebSocket authentication succeeded for the first community"),
    ("/query", "POST /query answered 2xx"),
    ("/upload", "PUT /upload answered 2xx"),
    ("/media/*", "GET /media/{sha256_ext} answered 2xx"),
    ("second ws auth", "WebSocket authentication succeeded for the second community"),
)


def parse_time(value):
    """RFC 3339 with up to nanosecond precision, to epoch seconds."""
    text = value.rstrip("Z")
    head, _, fraction = text.partition(".")
    stamp = datetime.datetime.fromisoformat(head).replace(tzinfo=datetime.UTC)
    return stamp.timestamp() + (float("0." + fraction) if fraction else 0.0)


def read_access_lines(path):
    """Access lines in log order; anything that is not one is skipped."""
    lines = []
    for raw in Path(path).read_text(errors="replace").splitlines():
        try:
            record = json.loads(raw)
        except json.JSONDecodeError:
            continue
        # The relay flattens an event's fields into the record; keep reading a
        # nested `fields` object too, for a subscriber that does not.
        fields = {**record, **(record.get("fields") or {})}
        if fields.get("event") not in ("http", "auth") or "timestamp" not in record:
            continue
        try:
            fields = {**fields, "_t": parse_time(record["timestamp"])}
        except ValueError:
            continue
        lines.append(fields)
    return lines


def read_markers(path):
    markers = []
    for raw in Path(path).read_text().splitlines():
        if raw.strip():
            markers.append(json.loads(raw))
    return sorted(markers, key=lambda marker: marker["t"])


def step_at(markers, when):
    current = "before the first step"
    for marker in markers:
        if marker["t"] <= when:
            current = marker["step"]
    return current


def describe(line):
    parts = [f"event={line.get('event')}", f"route={line.get('route', '-')}"]
    for key in ("http_method", "method", "outcome", "reason", "status", "community"):
        if line.get(key) is not None:
            parts.append(f"{key}={line[key]}")
    return " ".join(parts)


def is_success(line):
    status = line.get("status")
    if line.get("event") == "http":
        return isinstance(status, int) and 200 <= status < 300
    return line.get("outcome") == "success"


def evaluate(lines, markers, fixture, result):
    """Returns (failures, coverage) for one run."""
    failures = []
    names = {marker["step"] for marker in markers}
    if not lines:
        failures.append("the relay log holds no access lines (missing or empty log)")
    if result is None:
        failures.append(
            "fixture-result.json is missing: the fixture did not shut down cleanly"
        )
    elif result.get("relay_alive_at_stop") is not True:
        failures.append(
            f"the relay was not running at the end of the run (exit code {result.get('relay_returncode')})"
        )
    for needed in ("control_start", "control_end", "journey_end"):
        if needed not in names:
            failures.append(
                f"step marker `{needed}` is missing: the journey did not run to its end"
            )

    starts = [m["t"] for m in markers if m["step"] == "control_start"]
    ends = [m["t"] for m in markers if m["step"] == "control_end"]
    window = (starts[0], ends[-1]) if starts and ends else None

    controls = []
    for line in lines:
        if line.get("method") != "nip_fi" or line.get("outcome") not in DENIED:
            continue
        if window and window[0] <= line["_t"] <= window[1]:
            controls.append(line)
        else:
            failures.append(
                f"NIP-FI denial outside the negative control, during step "
                f"`{step_at(markers, line['_t'])}`: {describe(line)}"
            )
    if window and not any(
        line.get("route") == "/query" and line.get("status") == 401 for line in controls
    ):
        failures.append(
            "the negative control (unauthenticated POST /query) produced no NIP-FI 401: "
            "enforcement was not proven, so a pass would mean nothing"
        )

    first, second = fixture["community_id"], fixture["second_community_id"]
    coverage = {}
    for name, _meaning in REQUIRED:
        coverage[name] = 0
    for line in lines:
        if not is_success(line):
            continue
        if line["event"] == "auth":
            if line.get("community") == first:
                coverage["ws auth"] += 1
            elif line.get("community") == second:
                coverage["second ws auth"] += 1
        elif line["event"] == "http" and line.get("http_method") != "HEAD":
            route = line.get("route", "")
            if route == "/query":
                coverage["/query"] += 1
            elif route in ("/upload", "/media/upload"):
                coverage["/upload"] += 1
            elif route.startswith("/media/{"):
                coverage["/media/*"] += 1
    for name, meaning in REQUIRED:
        if coverage[name] == 0:
            failures.append(f"required route class never appeared: {name} ({meaning})")
    return failures, coverage, controls


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("workdir", type=Path)
    args = parser.parse_args()
    fixture = json.loads((args.workdir / "fixture.json").read_text())

    def load(name, reader):
        path = args.workdir / name
        return reader(path) if path.exists() else None

    lines = load("relay.log", read_access_lines) or []
    markers = load("markers.jsonl", read_markers) or []
    result = load("fixture-result.json", lambda p: json.loads(p.read_text()))
    failures, coverage, controls = evaluate(lines, markers, fixture, result)

    print(f"relay access lines: {len(lines)}")
    print(f"negative control denials: {len(controls)}")
    for line in controls:
        print(f"  control: {describe(line)}")
    print("required route classes:")
    for name, count in coverage.items():
        print(f"  {'ok     ' if count else 'MISSING'} {name}: {count} success line(s)")
    if failures:
        print("\nFAIL")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print(
        "\nPASS: no NIP-FI denial outside the negative control, every required route reached"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
