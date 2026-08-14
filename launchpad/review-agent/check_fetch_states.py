"""Control for #138: a non-UTF-8 gh response must classify as unparseable, not absent.

CONTAINMENT.md's degenerate-input table draws a line between "absent" (the fetch
itself failed — network, auth, missing) and "unparseable" (the fetch succeeded but the
bytes don't parse). Malformed JSON already lands on the right side of that line; an
undecodable byte stream did not, and #138 is the same taxonomy bug living one call
earlier in `_gh`. Also covers the identical mistake found in `_linked_issue`'s own
malformed-JSON handling while fixing this.
"""

from __future__ import annotations

import subprocess
import sys
from types import SimpleNamespace
from unittest import mock

import fetch

failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


def _fake_proc(returncode: int, stdout: bytes, stderr: bytes = b"") -> SimpleNamespace:
    return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)


# --- _gh itself: the three failure shapes must stay distinguishable ---------

with mock.patch.object(subprocess, "run", return_value=_fake_proc(0, b"\xff\xfe not utf-8")):
    state, out, reason = fetch._gh(["api", "whatever"])
    check(state == "unparseable", f"_gh: non-UTF-8 response classifies unparseable (got {state!r})")
    check("not valid UTF-8" in reason, f"_gh: reason names the real cause (got {reason!r})")

with mock.patch.object(subprocess, "run", return_value=_fake_proc(1, b"", b"HTTP 404")):
    state, out, reason = fetch._gh(["api", "whatever"])
    check(state == "absent", f"_gh: non-zero exit still classifies absent (got {state!r})")

with mock.patch.object(subprocess, "run", return_value=_fake_proc(0, b'{"ok": true}')):
    state, out, reason = fetch._gh(["api", "whatever"])
    check(state == "ok", f"_gh: a clean response still classifies ok (got {state!r})")


# --- _json_field: the state must propagate all the way to the Surface -------

with mock.patch.object(subprocess, "run", return_value=_fake_proc(0, b"\xff\xfe not utf-8")):
    surface = fetch._json_field("pr_title", ["api", "whatever"], lambda d: d["title"])
    check(
        surface.state == "unparseable",
        f"_json_field: non-UTF-8 propagates as unparseable, not absent (got {surface.state!r})",
    )
    check(not surface.readable, "_json_field: unparseable surface is still unreadable")

with mock.patch.object(subprocess, "run", return_value=_fake_proc(1, b"", b"network error")):
    surface = fetch._json_field("pr_title", ["api", "whatever"], lambda d: d["title"])
    check(
        surface.state == "absent",
        f"_json_field: a real fetch failure still classifies absent (got {surface.state!r})",
    )


# --- pr_diff's direct _gh call in fetch_all's style also propagates it right -

with mock.patch.object(subprocess, "run", return_value=_fake_proc(0, b"\xfe\xff binary diff?")):
    state, diff, reason = fetch._gh(["api", "whatever"], accept="application/vnd.github.v3.diff")
    surface = fetch._classify("pr_diff", state, diff, reason)
    check(
        surface.state == "unparseable",
        f"_classify: non-UTF-8 pr_diff response is unparseable, not absent (got {surface.state!r})",
    )


# --- _linked_issue: malformed JSON on a linked-issue fetch is unparseable, ---
# --- not absent — the same class of bug this control was written for -------

body = fetch.Surface("pr_body", "ok", text="Closes #999")
with mock.patch.object(subprocess, "run", return_value=_fake_proc(0, b"not json at all")):
    surface = fetch._linked_issue(body, "launchpad-26/buzz")
    check(
        surface.state == "unparseable",
        f"_linked_issue: malformed JSON on the target issue is unparseable, not absent (got {surface.state!r})",
    )

with mock.patch.object(subprocess, "run", return_value=_fake_proc(1, b"", b"404")):
    surface = fetch._linked_issue(body, "launchpad-26/buzz")
    check(
        surface.state == "absent",
        f"_linked_issue: a real fetch failure on the target issue is still absent (got {surface.state!r})",
    )


print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
