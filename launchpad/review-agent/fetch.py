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

#: GitHub recognises three forms after a closing keyword: a bare ``#123`` (this
#: repo), a qualified ``owner/repo#123``, and a full issue URL. All three are
#: author-controlled, so all three are matched here rather than only the bare form.
_CLOSING_KEYWORD = re.compile(
    r"\b(?:close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)\s+"
    r"(?:"
    r"(?P<qualified_repo>[\w.-]+/[\w.-]+)#(?P<qualified_num>\d+)"
    r"|https?://github\.com/(?P<url_repo>[\w.-]+/[\w.-]+)/issues/(?P<url_num>\d+)"
    r"|#(?P<bare_num>\d+)"
    r")",
    re.IGNORECASE,
)


@dataclass
class Surface:
    entry_point: str
    state: str  # ok | empty | absent | oversized | unparseable
    text: str = ""
    reason: str = ""

    @property
    def readable(self) -> bool:
        return self.state not in UNREADABLE


def _gh(args: list[str], accept: str | None = None) -> tuple[str, str, str]:
    """Run gh. Returns (state, stdout, reason). Never raises on a failed call.

    ``state`` is one of "ok" (call succeeded and decoded), "absent" (the call
    itself failed — missing binary, timeout, non-zero exit), or "unparseable"
    (the call succeeded but the response cannot be decoded as UTF-8). Absent
    and unparseable are different facts — one is usually transient (network,
    auth, rate limit) and worth retrying, the other is permanent — so callers
    must not collapse them into a single boolean the way malformed JSON
    already isn't collapsed with a failed call.
    """
    cmd = ["gh", *args]
    if accept:
        cmd += ["-H", f"Accept: {accept}"]
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=60)
    except FileNotFoundError:
        return "absent", "", "gh is not installed"
    except subprocess.TimeoutExpired:
        return "absent", "", "gh timed out after 60s"
    if proc.returncode != 0:
        detail = proc.stderr.decode("utf-8", "replace").strip().splitlines()
        return "absent", "", detail[-1] if detail else f"gh exited {proc.returncode}"
    try:
        return "ok", proc.stdout.decode("utf-8"), ""
    except UnicodeDecodeError as exc:
        return "unparseable", "", f"response is not valid UTF-8: {exc}"


def _classify(entry_point: str, state: str, text: str, reason: str) -> Surface:
    if state != "ok":
        return Surface(entry_point, state, reason=reason)
    if len(text.encode("utf-8")) > CAP_PER_ENTRY_POINT:
        # ``text`` is preserved, for the reason ``apply_invocation_cap`` gives for the
        # aggregate cap: the content is never rendered for an unreadable surface —
        # ``contain.render`` emits a SKIP and continues before reaching the block — but
        # the containment findings are still computed from it. Discarding it here put
        # the refusal one layer ABOVE render(), so contain() and detect.detect() both
        # ran against an empty string and a payload padded past the cap produced no
        # findings at all. Withholding the content must not withhold the evidence that
        # someone probed the boundary; padding was the cheapest way to buy silence.
        return Surface(
            entry_point,
            "oversized",
            text=text,
            reason=f"{len(text.encode('utf-8'))} bytes exceeds the "
            f"{CAP_PER_ENTRY_POINT}-byte cap; refused rather than truncated",
        )
    if not text.strip():
        return Surface(entry_point, "empty")
    return Surface(entry_point, "ok", text=text)


def _json_field(entry_point: str, args: list[str], extract) -> Surface:
    state, out, reason = _gh(args)
    if state != "ok":
        return Surface(entry_point, state, reason=reason)
    try:
        payload = json.loads(out)
    except json.JSONDecodeError as exc:
        return Surface(entry_point, "unparseable", reason=f"malformed JSON: {exc}")
    try:
        text = extract(payload)
    except (KeyError, TypeError, AttributeError) as exc:
        return Surface(entry_point, "unparseable", reason=f"unexpected shape: {exc}")
    return _classify(entry_point, "ok", text or "", "")


def _joined(items: list[dict], key: str = "body") -> str:
    return "\n\n".join((item.get(key) or "").strip() for item in items if item.get(key))


def _joined_paginated(pages: list[list[dict]], key: str = "body") -> str:
    """Flatten ``--paginate --slurp``'s one-array-per-page shape, then join.

    Without ``--paginate``, GitHub's issue-comment, review-comment and review-list
    endpoints return only their first page — 30 items by default — so a PR with more
    than 30 comments or reviews in any one category silently lost everything past that
    point. Nothing detects the loss: a truncated list is a normally-shaped JSON array,
    so it fetches, parses and joins exactly like a complete one, and an injection
    attempt sitting in record 31 is never fetched at all, not misdetected. ``--slurp``
    wraps the pages themselves in an outer array rather than merging them, so this
    flattens one level before ``_joined`` sees a plain list of items.
    """
    return _joined([item for page in pages for item in page], key)


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

    state, diff, reason = _gh(
        ["api", f"{base}/pulls/{pr}"], accept="application/vnd.github.v3.diff"
    )
    surfaces["pr_diff"] = _classify("pr_diff", state, diff, reason)

    surfaces["pr_issue_comments"] = _json_field(
        "pr_issue_comments",
        ["api", "--paginate", "--slurp", f"{base}/issues/{pr}/comments"],
        _joined_paginated,
    )
    surfaces["pr_review_comments"] = _json_field(
        "pr_review_comments",
        ["api", "--paginate", "--slurp", f"{base}/pulls/{pr}/comments"],
        _joined_paginated,
    )
    surfaces["pr_review_bodies"] = _json_field(
        "pr_review_bodies",
        ["api", "--paginate", "--slurp", f"{base}/pulls/{pr}/reviews"],
        _joined_paginated,
    )

    surfaces["linked_issue"] = _linked_issue(surfaces["pr_body"], repo)

    missing = set(ENTRY_POINTS) - set(surfaces)
    if missing:  # pragma: no cover - guards against an entry point added upstream
        raise RuntimeError(f"entry points not fetched: {sorted(missing)}")
    return surfaces


def _linked_issue(body: Surface, repo: str) -> Surface:
    """The issue(s) a closing keyword names. Author-controlled: anyone can open one.

    ``.search()`` on the first match only covered a bare ``#123`` and only the
    first closing keyword in the body — "Fixes #10, Closes owner/other#20" left
    the second reference entirely unfetched, and a qualified or URL reference
    matched nothing at all, silently omitting an author-controlled surface. Every
    reference is now resolved, deduplicated and fetched.

    A failed fetch on ANY target marks the whole surface ``absent``, rather than
    being silently skipped while the others' text is joined. An earlier version
    skipped a failed target and still returned "ok" with whatever text the other
    targets yielded -- exactly the "absence of evidence reported as evidence"
    CONTAINMENT.md forbids: a reference to a deleted, private or malformed issue
    is indistinguishable from "nothing more to read", and the failed target is
    precisely where an author could put the text this module exists to catch.
    Failing the whole surface, named after the one target it could not read, is
    the same conservative choice `_classify` already makes for every other
    single-fetch surface.
    """
    if not body.readable:
        return Surface(
            "linked_issue", "absent", reason=f"pr_body was {body.state}, so no keyword could be read"
        )
    targets: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()
    for match in _CLOSING_KEYWORD.finditer(body.text):
        target_repo = match.group("qualified_repo") or match.group("url_repo") or repo
        number = match.group("qualified_num") or match.group("url_num") or match.group("bare_num")
        key = (target_repo, number)
        if key not in seen:
            seen.add(key)
            targets.append(key)
    if not targets:
        return Surface("linked_issue", "empty")
    bodies: list[str] = []
    for target_repo, number in targets:
        state, out, reason = _gh(["api", f"repos/{target_repo}/issues/{number}"])
        if state != "ok":
            return Surface(
                "linked_issue",
                state,
                reason=f"{target_repo}#{number} could not be read: {reason}",
            )
        try:
            payload = json.loads(out)
        except json.JSONDecodeError as exc:
            return Surface(
                "linked_issue",
                "unparseable",
                reason=f"{target_repo}#{number}: malformed JSON: {exc}",
            )
        text = (payload.get("body") or "").strip()
        if text:
            bodies.append(text)
    return _classify("linked_issue", "ok", "\n\n".join(bodies), "")


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
        surfaces[entry_point] = _classify(entry_point, "ok", raw[entry_point] or "", "")
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
