"""Fetch the seven author-controlled surfaces of a pull request.

Every surface resolves to one of five states, per CONTAINMENT.md § Degenerate input.
The states are the point of this module: a fetch that failed and a fetch that returned
nothing are different facts, and collapsing them is how a review of nothing gets
published as a review of something clean.
"""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass

from contain import ENTRY_POINTS

DEFAULT_REPO = "launchpad-26/buzz"

#: CONTAINMENT.md § Degenerate input.
CAP_PER_ENTRY_POINT = 512 * 1024
CAP_PER_INVOCATION = 2 * 1024 * 1024

#: States that mean "nothing was read", as distinct from "nothing is there".
UNREADABLE = ("absent", "oversized", "unparseable")

_CLOSING_KEYWORD = re.compile(r"\b(?:closes|fixes|resolves)\s+#(\d+)\b", re.IGNORECASE)


@dataclass
class Surface:
    entry_point: str
    state: str  # ok | empty | absent | oversized | unparseable
    text: str = ""
    reason: str = ""

    @property
    def readable(self) -> bool:
        return self.state not in UNREADABLE


def _gh(args: list[str], accept: str | None = None) -> tuple[bool, str, str]:
    """Run gh. Returns (ok, stdout, reason). Never raises on a failed call."""
    cmd = ["gh", *args]
    if accept:
        cmd += ["-H", f"Accept: {accept}"]
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=60)
    except FileNotFoundError:
        return False, "", "gh is not installed"
    except subprocess.TimeoutExpired:
        return False, "", "gh timed out after 60s"
    if proc.returncode != 0:
        detail = proc.stderr.decode("utf-8", "replace").strip().splitlines()
        return False, "", detail[-1] if detail else f"gh exited {proc.returncode}"
    try:
        return True, proc.stdout.decode("utf-8"), ""
    except UnicodeDecodeError as exc:
        return False, "", f"response is not valid UTF-8: {exc}"


def _classify(entry_point: str, ok: bool, text: str, reason: str) -> Surface:
    if not ok:
        return Surface(entry_point, "absent", reason=reason)
    if len(text.encode("utf-8")) > CAP_PER_ENTRY_POINT:
        return Surface(
            entry_point,
            "oversized",
            reason=f"{len(text.encode('utf-8'))} bytes exceeds the "
            f"{CAP_PER_ENTRY_POINT}-byte cap; refused rather than truncated",
        )
    if not text.strip():
        return Surface(entry_point, "empty")
    return Surface(entry_point, "ok", text=text)


def _json_field(entry_point: str, args: list[str], extract) -> Surface:
    ok, out, reason = _gh(args)
    if not ok:
        return Surface(entry_point, "absent", reason=reason)
    try:
        payload = json.loads(out)
    except json.JSONDecodeError as exc:
        return Surface(entry_point, "unparseable", reason=f"malformed JSON: {exc}")
    try:
        text = extract(payload)
    except (KeyError, TypeError, AttributeError) as exc:
        return Surface(entry_point, "unparseable", reason=f"unexpected shape: {exc}")
    return _classify(entry_point, True, text or "", "")


def _joined(items: list[dict], key: str = "body") -> str:
    return "\n\n".join((item.get(key) or "").strip() for item in items if item.get(key))


def fetch_all(pr: int, repo: str = DEFAULT_REPO) -> dict[str, Surface]:
    """Fetch every surface. A failure on one surface never aborts the others."""
    base = f"repos/{repo}"
    surfaces: dict[str, Surface] = {}

    surfaces["pr_title"] = _json_field(
        "pr_title", ["api", f"{base}/pulls/{pr}"], lambda d: d["title"]
    )
    surfaces["pr_body"] = _json_field(
        "pr_body", ["api", f"{base}/pulls/{pr}"], lambda d: d.get("body") or ""
    )

    ok, diff, reason = _gh(
        ["api", f"{base}/pulls/{pr}"], accept="application/vnd.github.v3.diff"
    )
    surfaces["pr_diff"] = _classify("pr_diff", ok, diff, reason)

    surfaces["pr_issue_comments"] = _json_field(
        "pr_issue_comments", ["api", f"{base}/issues/{pr}/comments"], _joined
    )
    surfaces["pr_review_comments"] = _json_field(
        "pr_review_comments", ["api", f"{base}/pulls/{pr}/comments"], _joined
    )
    surfaces["pr_review_bodies"] = _json_field(
        "pr_review_bodies", ["api", f"{base}/pulls/{pr}/reviews"], _joined
    )

    surfaces["linked_issue"] = _linked_issue(surfaces["pr_body"], repo)

    missing = set(ENTRY_POINTS) - set(surfaces)
    if missing:  # pragma: no cover - guards against an entry point added upstream
        raise RuntimeError(f"entry points not fetched: {sorted(missing)}")
    return surfaces


def _linked_issue(body: Surface, repo: str) -> Surface:
    """The issue a closing keyword names. Author-controlled: anyone can open an issue."""
    if not body.readable:
        return Surface(
            "linked_issue", "absent", reason=f"pr_body was {body.state}, so no keyword could be read"
        )
    match = _CLOSING_KEYWORD.search(body.text)
    if not match:
        return Surface("linked_issue", "empty")
    number = match.group(1)
    return _json_field(
        "linked_issue",
        ["api", f"repos/{repo}/issues/{number}"],
        lambda d: d.get("body") or "",
    )


def from_payload(path: str) -> dict[str, Surface]:
    """Load a captured PR from disk, so a control does not depend on live GitHub."""
    with open(path, encoding="utf-8") as handle:
        raw = json.load(handle)
    surfaces: dict[str, Surface] = {}
    for entry_point in ENTRY_POINTS:
        if entry_point not in raw:
            surfaces[entry_point] = Surface(
                entry_point, "absent", reason=f"captured payload has no {entry_point} field"
            )
            continue
        surfaces[entry_point] = _classify(entry_point, True, raw[entry_point] or "", "")
    return surfaces


def invocation_total(surfaces: dict[str, Surface]) -> int:
    """Bytes across every surface that carries text.

    Counts all surfaces rather than only readable ones, so the total is unchanged by
    :func:`apply_invocation_cap` marking them oversized — the refusal has to be
    idempotent, or applying it twice would silently un-refuse the invocation.
    """
    return sum(len(s.text.encode("utf-8")) for s in surfaces.values())


def apply_invocation_cap(surfaces: dict[str, Surface]) -> dict[str, Surface]:
    """Mark every readable surface ``oversized`` when the aggregate cap is breached.

    **The state is the refusal.** An earlier version refused inside ``contain.render``
    and left every ``Surface.state`` as ``ok``, so the refusal existed only in the
    rendered document. ``review.render_review`` derives its "this review is incomplete"
    banner from the states, found none unreadable, and published a review of a wholly
    withheld pull request that read "No containment findings" — exactly the
    "absence of evidence reported as evidence" that CONTAINMENT.md § Degenerate input
    forbids. Deciding the state here, beside every other state decision, is what makes
    the refusal visible to each of the four consuming stages.

    ``text`` is deliberately preserved: the content is never rendered for an unreadable
    surface, but the containment findings still have to be computed from it. Withholding
    the content must not withhold the evidence that someone probed the boundary.
    """
    total = invocation_total(surfaces)
    if total <= CAP_PER_INVOCATION:
        return surfaces
    reason = (
        f"{total} bytes across all surfaces exceeds the {CAP_PER_INVOCATION}-byte "
        f"per-invocation cap; every surface is refused rather than truncated"
    )
    return {
        ep: Surface(ep, "oversized", text=s.text, reason=reason) if s.readable else s
        for ep, s in surfaces.items()
    }


def degrade(surfaces: dict[str, Surface], spec: str) -> dict[str, Surface]:
    """Force a surface into a degenerate state: ``--degrade pr_diff=oversized``."""
    entry_point, _, state = spec.partition("=")
    if entry_point not in ENTRY_POINTS:
        raise ValueError(f"unknown entry point: {entry_point!r}")
    if state not in ("absent", "empty", "oversized", "unparseable"):
        raise ValueError(f"unknown degenerate state: {state!r}")
    surfaces = dict(surfaces)
    surfaces[entry_point] = Surface(
        entry_point, state, reason=f"forced by --degrade {spec}"
    )
    return surfaces
