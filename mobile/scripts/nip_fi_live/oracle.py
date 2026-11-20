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
* any journey step marker is missing (the journey did not run to its end);
* a required route class does not show a success inside the step it belongs
  to (a success elsewhere in the log does not count);
* the expiry path did not actually run: the relay session was not quiet while
  the cached assertion aged out, that assertion had not expired by the time of
  the read, or no newer assertion was minted for the read after expiry.

Usage: oracle.py <workdir>   (exit 0 pass, 1 fail)
"""

import argparse
import datetime
import json
import sys
from pathlib import Path

DENIED = {"denied", "failed"}

# Every marker the journey writes, in order. A missing one means the journey
# stopped early or the test no longer matches this oracle.
JOURNEY_STEPS = (
    "control_start",
    "control_end",
    "sign_in",
    "connect",
    "query",
    "upload",
    "media",
    "native_media",
    "expiry_wait",
    "native_media_after_expiry",
    "expiry_end",
    "query_after_expiry",
    "tombstone",
    "journey_end",
)

# Route classes the journey must reach with a success, each bound to the step
# whose window it must fall in: (class, step, what it means).
REQUIRED = (
    (
        "ws auth",
        "connect",
        "WebSocket authentication succeeded for the first community",
    ),
    ("/query", "query", "POST /query answered 2xx"),
    ("/upload", "upload", "PUT /upload answered 2xx"),
    ("/media/*", "media", "GET /media/{sha256_ext} answered 2xx (app HTTP client)"),
    (
        "/media/*",
        "native_media",
        "GET /media/{sha256_ext} answered 2xx (native headers)",
    ),
    (
        "/media/*",
        "native_media_after_expiry",
        "GET /media/{sha256_ext} answered 2xx with a renewed assertion",
    ),
    ("/query", "query_after_expiry", "POST /query answered 2xx after expiry"),
    (
        "second ws auth",
        "tombstone",
        "WebSocket authentication succeeded for the second community",
    ),
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


def marker_times(markers):
    """First time of each step marker."""
    times = {}
    for marker in markers:
        times.setdefault(marker["step"], marker["t"])
    return times


def window(markers, step):
    """The span of one step: its marker to the next marker (or the end)."""
    times = marker_times(markers)
    if step not in times:
        return None
    start = times[step]
    later = [t for t in times.values() if t > start]
    return start, (min(later) if later else float("inf"))


def in_window(line, span):
    return span is not None and span[0] <= line["_t"] < span[1]


def classify(line, first, second):
    """The route class of a successful line, or None."""
    if not is_success(line):
        return None
    if line["event"] == "auth":
        if line.get("community") == first:
            return "ws auth"
        if line.get("community") == second:
            return "second ws auth"
    elif line["event"] == "http" and line.get("http_method") != "HEAD":
        route = line.get("route", "")
        if route == "/query":
            return "/query"
        if route in ("/upload", "/media/upload"):
            return "/upload"
        if route.startswith("/media/{"):
            return "/media/*"
    return None


def check_expiry_path(lines, markers, fixture, result):
    """The read after the assertion expired must have been a real expiry.

    Without this a pass could be vacuous: a live relay session reconnects at
    lease expiry and refreshes the cached assertion, and then the read after
    the wait succeeds without the fresh-assertion path ever running.
    """
    failures = []
    times = marker_times(markers)
    wait = times.get("expiry_wait")
    read = times.get("native_media_after_expiry")
    end = times.get("expiry_end")
    if wait is None or read is None or end is None:
        return failures  # reported as missing markers
    quiet = (wait, end)
    for line in lines:
        if not quiet[0] <= line["_t"] < quiet[1]:
            continue
        if line["event"] == "auth" or line.get("route") == "/query":
            failures.append(
                "the relay session was not down during the expiry wait, so the "
                f"cached assertion may have been renewed behind the read: {describe(line)}"
            )
    mints = [
        mint
        for mint in (result or {}).get("mints", [])
        if mint.get("aud") == fixture["relay_origin"]
    ]
    before = [mint for mint in mints if mint["t"] < wait]
    if not before:
        failures.append("no assertion was minted before the expiry wait")
        return failures
    previous = max(before, key=lambda mint: mint["iat"])
    if previous["exp"] > read:
        failures.append(
            "the cached assertion had not expired when the read after expiry "
            f"started (exp {previous['exp']}, read at {read:.0f})"
        )
    renewed = [
        mint
        for mint in mints
        if read <= mint["t"] < end and mint["iat"] > previous["iat"]
    ]
    if not renewed:
        failures.append(
            "no newer assertion (larger iat) was minted for the read after "
            "expiry: it went out with the expired cached one or none at all"
        )
    return failures


def evaluate(lines, markers, fixture, result):
    """Returns (failures, coverage, controls) for one run."""
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
    for needed in JOURNEY_STEPS:
        if needed not in names:
            failures.append(
                f"step marker `{needed}` is missing: the journey did not run to its end"
            )

    starts = [m["t"] for m in markers if m["step"] == "control_start"]
    ends = [m["t"] for m in markers if m["step"] == "control_end"]
    control_window = (starts[0], ends[-1]) if starts and ends else None

    controls = []
    for line in lines:
        if line.get("method") != "nip_fi" or line.get("outcome") not in DENIED:
            continue
        if control_window and control_window[0] <= line["_t"] <= control_window[1]:
            controls.append(line)
        else:
            failures.append(
                f"NIP-FI denial outside the negative control, during step "
                f"`{step_at(markers, line['_t'])}`: {describe(line)}"
            )
    if control_window and not any(
        line.get("route") == "/query" and line.get("status") == 401 for line in controls
    ):
        failures.append(
            "the negative control (unauthenticated POST /query) produced no NIP-FI 401: "
            "enforcement was not proven, so a pass would mean nothing"
        )

    first, second = fixture["community_id"], fixture["second_community_id"]
    coverage = {}
    for name, step, meaning in REQUIRED:
        span = window(markers, step)
        count = sum(
            1
            for line in lines
            if in_window(line, span) and classify(line, first, second) == name
        )
        coverage[(name, step)] = count
        if count == 0 and span is not None:
            failures.append(
                f"required route class never appeared in step `{step}`: "
                f"{name} ({meaning})"
            )
    failures.extend(check_expiry_path(lines, markers, fixture, result))
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
    for (name, step), count in coverage.items():
        print(
            f"  {'ok     ' if count else 'MISSING'} {name} in `{step}`: "
            f"{count} success line(s)"
        )
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
