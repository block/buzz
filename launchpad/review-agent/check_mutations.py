"""Mutation harness: break containment on purpose, and require the suite to notice.

**Why this exists.** The original mutation control flipped one seam — `enabled=False`,
which removes the envelope wholesale — and passing it was read as "the controls fail if
containment is removed". An independent review disproved that: deleting the *escaping*
from `contain()` while leaving the envelope intact passed every control, including the
mutation control itself. One removal had been tested and the result generalised.

So the seam is now one mutant among several. Each mutation below is a plausible
regression a future edit could introduce; each must be caught by at least one control,
and the harness names which one caught it. A mutation nothing catches is a hole in the
suite, reported as a failure here rather than discovered by an attacker.

Mutations are applied to a scratch copy. The repository is never modified.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).parent

#: (name, file, find, replace, why it matters)
MUTATIONS = [
    (
        "no-escaping",
        "contain.py",
        'block = f"{open_marker}\\n{escape(raw)}\\n{close_marker}"',
        'block = f"{open_marker}\\n{raw}\\n{close_marker}"',
        "payload wrapped but not escaped — a forged close marker survives verbatim",
    ),
    (
        "constant-nonce",
        "contain.py",
        "    return secrets.token_hex(16)",
        '    return "0" * 32',
        "nonce is predictable, so an author can write a closing marker that matches",
    ),
    (
        "short-nonce",
        "contain.py",
        "    return secrets.token_hex(16)",
        "    return secrets.token_hex(2) + '0' * 28",
        "entropy collapses to 16 bits while the nonce still looks the right shape",
    ),
    (
        "no-lookalike-detection",
        "contain.py",
        "    findings = find_lookalikes(raw, entry_point)",
        "    findings = []",
        "boundary probes are neutralised silently — the swallowed attack #120 forbids",
    ),
    (
        "unescaped-evidence",
        "review.py",
        "                    escape(finding.evidence),",
        "                    finding.evidence,",
        "the published review quotes attacker text raw, becoming a fresh vector",
    ),
    (
        "fixed-fence",
        "review.py",
        "                    fence_for(finding.evidence),\n"
        "                    escape(finding.evidence),\n"
        "                    fence_for(finding.evidence),",
        '                    "```",\n'
        "                    escape(finding.evidence),\n"
        '                    "```",',
        "backticks in evidence break the fence and corrupt the rest of the review",
    ),
    (
        "cap-warns-only",
        "fetch.py",
        "    total = invocation_total(surfaces)\n    if total <= CAP_PER_INVOCATION:",
        "    total = invocation_total(surfaces)\n    if True:",
        "oversized input is warned about but still rendered in full",
    ),
    (
        "cap-leaves-states-ok",
        "fetch.py",
        '        ep: Surface(ep, "oversized", text=s.text, reason=reason) if s.readable else s',
        "        ep: s",
        "content is withheld but every state stays `ok`, so the review of a wholly "
        "withheld pull request renders with no incomplete banner and reads as clean",
    ),
    (
        "empty-reads-as-absent",
        "fetch.py",
        '        return Surface(entry_point, "empty")',
        '        return Surface(entry_point, "absent", reason="no content")',
        "a fetched-and-empty surface becomes indistinguishable from one never read",
    ),
    (
        "no-homoglyph-detection",
        "contain.py",
        "    skeleton = _skeleton(text)",
        '    skeleton = ""',
        "a cross-script look-alike delimiter is neither escaped nor flagged, so a "
        "boundary probe that is pixel-identical to the real marker passes unreported",
    ),
    (
        "restore-homoglyph-gate",
        "contain.py",
        "    for candidate in (skeleton, skeleton_squeezed):",
        "    if not findings:\n"
        "      for candidate in (skeleton, skeleton_squeezed):",
        "the homoglyph pass is gated on 'nothing found anywhere in the text' again, so "
        "a benign decoy mention ahead of a forged cross-script marker reports the "
        "decoy and the real marker never reaches the review",
    ),
    (
        "homoglyph-first-occurrence-only",
        "contain.py",
        "            for m in re.finditer(re.escape(TOKEN), candidate):\n"
        "                findings.append(\n"
        "                    Finding(\n"
        '                        "delimiter_lookalike",\n'
        "                        entry_point,\n"
        "                        _excerpt(candidate, m.start()),\n"
        "                    )\n"
        "                )",
        "            findings.append(\n"
        "                Finding(\n"
        '                    "delimiter_lookalike",\n'
        "                    entry_point,\n"
        "                    _excerpt(candidate, candidate.index(TOKEN)),\n"
        "                )\n"
        "            )",
        "only the first forged marker in a candidate is reported, so a second, "
        "distinct forgery of the same script in the same text is silently dropped",
    ),
    (
        "restore-skeleton-candidate-break",
        "contain.py",
        "                )\n"
        "\n"
        "    return _dedupe(findings)",
        "                )\n"
        "            break\n"
        "\n"
        "    return _dedupe(findings)",
        "stops scanning after the first winning candidate (skeleton vs "
        "skeleton_squeezed), so a decoy that lands in one hides a genuine "
        "letter-spaced forgery that only the OTHER candidate can see",
    ),
    (
        "linked-issue-bare-only",
        "fetch.py",
        '#: GitHub recognises three forms after a closing keyword: a bare ``#123`` (this\n#: repo), a qualified ``owner/repo#123``, and a full issue URL. All three are\n#: author-controlled, so all three are matched here rather than only the bare form.\n_CLOSING_KEYWORD = re.compile(\n    r"\\b(?:close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)\\s+"\n    r"(?:"\n    r"(?P<qualified_repo>[\\w.-]+/[\\w.-]+)#(?P<qualified_num>\\d+)"\n    r"|https?://github\\.com/(?P<url_repo>[\\w.-]+/[\\w.-]+)/issues/(?P<url_num>\\d+)"\n    r"|#(?P<bare_num>\\d+)"\n    r")",\n    re.IGNORECASE,\n)\n\n\n@dataclass\nclass Surface:\n    entry_point: str\n    state: str  # ok | empty | absent | oversized | unparseable\n    text: str = ""\n    reason: str = ""\n\n    @property\n    def readable(self) -> bool:\n        return self.state not in UNREADABLE\n\n\ndef _gh(args: list[str], accept: str | None = None) -> tuple[bool, str, str]:\n    """Run gh. Returns (ok, stdout, reason). Never raises on a failed call."""\n    cmd = ["gh", *args]\n    if accept:\n        cmd += ["-H", f"Accept: {accept}"]\n    try:\n        proc = subprocess.run(cmd, capture_output=True, timeout=60)\n    except FileNotFoundError:\n        return False, "", "gh is not installed"\n    except subprocess.TimeoutExpired:\n        return False, "", "gh timed out after 60s"\n    if proc.returncode != 0:\n        detail = proc.stderr.decode("utf-8", "replace").strip().splitlines()\n        return False, "", detail[-1] if detail else f"gh exited {proc.returncode}"\n    try:\n        return True, proc.stdout.decode("utf-8"), ""\n    except UnicodeDecodeError as exc:\n        return False, "", f"response is not valid UTF-8: {exc}"\n\n\ndef _classify(entry_point: str, ok: bool, text: str, reason: str) -> Surface:\n    if not ok:\n        return Surface(entry_point, "absent", reason=reason)\n    if len(text.encode("utf-8")) > CAP_PER_ENTRY_POINT:\n        # ``text`` is preserved, for the reason ``apply_invocation_cap`` gives for the\n        # aggregate cap: the content is never rendered for an unreadable surface —\n        # ``contain.render`` emits a SKIP and continues before reaching the block — but\n        # the containment findings are still computed from it. Discarding it here put\n        # the refusal one layer ABOVE render(), so contain() and detect.detect() both\n        # ran against an empty string and a payload padded past the cap produced no\n        # findings at all. Withholding the content must not withhold the evidence that\n        # someone probed the boundary; padding was the cheapest way to buy silence.\n        return Surface(\n            entry_point,\n            "oversized",\n            text=text,\n            reason=f"{len(text.encode(\'utf-8\'))} bytes exceeds the "\n            f"{CAP_PER_ENTRY_POINT}-byte cap; refused rather than truncated",\n        )\n    if not text.strip():\n        return Surface(entry_point, "empty")\n    return Surface(entry_point, "ok", text=text)\n\n\ndef _json_field(entry_point: str, args: list[str], extract) -> Surface:\n    ok, out, reason = _gh(args)\n    if not ok:\n        return Surface(entry_point, "absent", reason=reason)\n    try:\n        payload = json.loads(out)\n    except json.JSONDecodeError as exc:\n        return Surface(entry_point, "unparseable", reason=f"malformed JSON: {exc}")\n    try:\n        text = extract(payload)\n    except (KeyError, TypeError, AttributeError) as exc:\n        return Surface(entry_point, "unparseable", reason=f"unexpected shape: {exc}")\n    return _classify(entry_point, True, text or "", "")\n\n\ndef _joined(items: list[dict], key: str = "body") -> str:\n    return "\\n\\n".join((item.get(key) or "").strip() for item in items if item.get(key))\n\n\ndef _joined_paginated(pages: list[list[dict]], key: str = "body") -> str:\n    """Flatten ``--paginate --slurp``\'s one-array-per-page shape, then join.\n\n    Without ``--paginate``, GitHub\'s issue-comment, review-comment and review-list\n    endpoints return only their first page — 30 items by default — so a PR with more\n    than 30 comments or reviews in any one category silently lost everything past that\n    point. Nothing detects the loss: a truncated list is a normally-shaped JSON array,\n    so it fetches, parses and joins exactly like a complete one, and an injection\n    attempt sitting in record 31 is never fetched at all, not misdetected. ``--slurp``\n    wraps the pages themselves in an outer array rather than merging them, so this\n    flattens one level before ``_joined`` sees a plain list of items.\n    """\n    return _joined([item for page in pages for item in page], key)\n\n\ndef fetch_all(pr: int, repo: str = DEFAULT_REPO) -> dict[str, Surface]:\n    """Fetch every surface. A failure on one surface never aborts the others."""\n    base = f"repos/{repo}"\n    surfaces: dict[str, Surface] = {}\n\n    surfaces["pr_title"] = _json_field(\n        "pr_title", ["api", f"{base}/pulls/{pr}"], lambda d: d["title"]\n    )\n    surfaces["pr_body"] = _json_field(\n        "pr_body", ["api", f"{base}/pulls/{pr}"], lambda d: d.get("body") or ""\n    )\n\n    ok, diff, reason = _gh(\n        ["api", f"{base}/pulls/{pr}"], accept="application/vnd.github.v3.diff"\n    )\n    surfaces["pr_diff"] = _classify("pr_diff", ok, diff, reason)\n\n    surfaces["pr_issue_comments"] = _json_field(\n        "pr_issue_comments",\n        ["api", "--paginate", "--slurp", f"{base}/issues/{pr}/comments"],\n        _joined_paginated,\n    )\n    surfaces["pr_review_comments"] = _json_field(\n        "pr_review_comments",\n        ["api", "--paginate", "--slurp", f"{base}/pulls/{pr}/comments"],\n        _joined_paginated,\n    )\n    surfaces["pr_review_bodies"] = _json_field(\n        "pr_review_bodies",\n        ["api", "--paginate", "--slurp", f"{base}/pulls/{pr}/reviews"],\n        _joined_paginated,\n    )\n\n    surfaces["linked_issue"] = _linked_issue(surfaces["pr_body"], repo)\n\n    missing = set(ENTRY_POINTS) - set(surfaces)\n    if missing:  # pragma: no cover - guards against an entry point added upstream\n        raise RuntimeError(f"entry points not fetched: {sorted(missing)}")\n    return surfaces\n\n\ndef _linked_issue(body: Surface, repo: str) -> Surface:\n    """The issue(s) a closing keyword names. Author-controlled: anyone can open one.\n\n    ``.search()`` on the first match only covered a bare ``#123`` and only the\n    first closing keyword in the body — "Fixes #10, Closes owner/other#20" left\n    the second reference entirely unfetched, and a qualified or URL reference\n    matched nothing at all, silently omitting an author-controlled surface. Every\n    reference is now resolved and fetched, deduplicated, and joined; a fetch\n    failure on one target is skipped rather than discarding every other target\'s\n    text, since ``_gh`` already degrades a single failed call to "no content" and\n    an author cannot make ONE bad reference number hide every other one.\n    """\n    if not body.readable:\n        return Surface(\n            "linked_issue", "absent", reason=f"pr_body was {body.state}, so no keyword could be read"\n        )\n    targets: list[tuple[str, str]] = []\n    seen: set[tuple[str, str]] = set()\n    for match in _CLOSING_KEYWORD.finditer(body.text):\n        target_repo = match.group("qualified_repo") or match.group("url_repo") or repo\n        number = match.group("qualified_num") or match.group("url_num") or match.group("bare_num")\n        key = (target_repo, number)\n        if key not in seen:\n            seen.add(key)\n            targets.append(key)\n    if not targets:\n        return Surface("linked_issue", "empty")\n    bodies: list[str] = []\n    for target_repo, number in targets:\n        ok, out, _reason = _gh(["api", f"repos/{target_repo}/issues/{number}"])\n        if not ok:\n            continue\n        try:\n            payload = json.loads(out)\n        except json.JSONDecodeError:\n            continue\n        text = (payload.get("body") or "").strip()\n        if text:\n            bodies.append(text)\n    return _classify("linked_issue", True, "\\n\\n".join(bodies), "")',
        '_CLOSING_KEYWORD = re.compile(r"\\b(?:closes|fixes|resolves)\\s+#(\\d+)\\b", re.IGNORECASE)\n\n\ndef _linked_issue(body: Surface, repo: str) -> Surface:\n    """The issue a closing keyword names. Author-controlled: anyone can open an issue."""\n    if not body.readable:\n        return Surface(\n            "linked_issue", "absent", reason=f"pr_body was {body.state}, so no keyword could be read"\n        )\n    match = _CLOSING_KEYWORD.search(body.text)\n    if not match:\n        return Surface("linked_issue", "empty")\n    number = match.group(1)\n    return _json_field(\n        "linked_issue",\n        ["api", f"repos/{repo}/issues/{number}"],\n        lambda d: d.get("body") or "",\n    )',
        "linked_issue only matches a bare #N and only the first closing "
        "keyword, so a qualified owner/repo#N reference, a full issue URL, "
        "or a second referenced issue is silently never fetched",
    ),
    (
        "no-pagination",
        "fetch.py",
        '    surfaces["pr_issue_comments"] = _json_field(\n'
        '        "pr_issue_comments",\n'
        '        ["api", "--paginate", "--slurp", f"{base}/issues/{pr}/comments"],\n'
        "        _joined_paginated,\n"
        "    )\n"
        '    surfaces["pr_review_comments"] = _json_field(\n'
        '        "pr_review_comments",\n'
        '        ["api", "--paginate", "--slurp", f"{base}/pulls/{pr}/comments"],\n'
        "        _joined_paginated,\n"
        "    )\n"
        '    surfaces["pr_review_bodies"] = _json_field(\n'
        '        "pr_review_bodies",\n'
        '        ["api", "--paginate", "--slurp", f"{base}/pulls/{pr}/reviews"],\n'
        "        _joined_paginated,\n"
        "    )",
        '    surfaces["pr_issue_comments"] = _json_field(\n'
        '        "pr_issue_comments", ["api", f"{base}/issues/{pr}/comments"], _joined\n'
        "    )\n"
        '    surfaces["pr_review_comments"] = _json_field(\n'
        '        "pr_review_comments", ["api", f"{base}/pulls/{pr}/comments"], _joined\n'
        "    )\n"
        '    surfaces["pr_review_bodies"] = _json_field(\n'
        '        "pr_review_bodies", ["api", f"{base}/pulls/{pr}/reviews"], _joined\n'
        "    )",
        "comment/review list endpoints lose --paginate --slurp, so a PR with more "
        "than 30 comments or reviews in any category silently drops everything "
        "past the first page — an injection attempt past record 30 is never "
        "fetched at all",
    ),
    (
        "states-ignore-the-cap",
        "contain.py",
        "    states = {ep: surfaces[ep].state for ep in ENTRY_POINTS}\n"
        '    return "\\n".join(lines), findings, all_readable, states',
        '    states = {ep: "ok" for ep in ENTRY_POINTS}\n'
        '    return "\\n".join(lines), findings, all_readable, states',
        "render() returns states that never reflect the aggregate cap, so a "
        "caller building the Incomplete banner from render()'s own return value "
        "sees every surface as ok even when every surface was withheld",
    ),
    (
        "unbounded-findings-render",
        "review.py",
        '        ordered = sorted(findings, key=lambda f: SEVERITY_ORDER.get(f.severity, 9))\n        rendered_bytes = 0\n        omitted = 0\n        for index, finding in enumerate(ordered):\n            block = "\\n".join(\n                [\n                    f"### {finding.severity} — {finding.kind}",\n                    "",\n                    f"Entry point: `{finding.entry_point}`",\n                    "",\n                    # Post-escape, deliberately: see the module docstring. The\n                    # fence is sized longer than any backtick run in the\n                    # evidence, because attacker text containing ``` would\n                    # otherwise close the fence early and spill the rest of the\n                    # review into an unterminated code block.\n                    fence_for(finding.evidence),\n                    escape(finding.evidence),\n                    fence_for(finding.evidence),\n                    "",\n                ]\n            )\n            block_bytes = len(block.encode("utf-8"))\n            if rendered_bytes and rendered_bytes + block_bytes > MAX_FINDINGS_BYTES:\n                omitted = len(ordered) - index\n                break\n            lines.append(block)\n            rendered_bytes += block_bytes\n        if omitted:\n            lines.append(\n                f"**{omitted} further finding(s) omitted.** Rendering every finding "\n                f"would exceed a {MAX_FINDINGS_BYTES}-byte budget. A pull request "\n                "producing enough findings to hit this budget is itself worth "\n                "escalating."\n            )\n            lines.append("")\n',
        '        for finding in sorted(findings, key=lambda f: SEVERITY_ORDER.get(f.severity, 9)):\n            lines.append(f"### {finding.severity} — {finding.kind}")\n            lines.append("")\n            lines.append(f"Entry point: `{finding.entry_point}`")\n            lines.append("")\n            lines.append(fence_for(finding.evidence))\n            lines.append(escape(finding.evidence))\n            lines.append(fence_for(finding.evidence))\n            lines.append("")\n',
        "the rendered findings section has no size budget again, so enough "
        "distinct low-severity findings render past GitHub's body limit and "
        "suppress the whole review, Blockers included, through sheer volume",
    ),
    (
        "newline-split-restored",
        "detect.py",
        '        joined = re.sub(r"\\s+", " ", " ".join(passage))\n'
        "        out.extend(chunk.strip() for chunk in _SENTENCE_END.split(joined) if chunk.strip())",
        "        for line in passage:\n"
        "            out.extend(c.strip() for c in _SENTENCE_END.split(line) if c.strip())",
        "sentences split on newlines again, so one newline mid-phrase defeats every "
        "injection tell — the bypass that let three of four attack classes through",
    ),
    (
        "detect-rule-dropped",
        "detect.py",
        r'    r"|system\s+directive"',
        r'    r"|system\s+directive_disabled"',
        "one alternative is quietly removed from the standalone tells, lowering recall "
        "without changing any count the contract states",
    ),
    # --- the preprocessing contract: a line loses decoration, never prose ---
    # Four bypasses lived here across four commits, and none had a mutant. Each entry
    # below reintroduces one of them, so the contract in _sentences' docstring is
    # enforced rather than merely written down.
    (
        "structure-swallows-prose",
        "detect.py",
        r'    r"|(?:---|\+\+\+) (?:/dev/null|[ab]/\S+)"',
        r'    r"|(?:---|\+\+\+) \S.*"',
        "a structure pattern loose enough to match a line CARRYING prose drops that "
        "prose — `--- ignore all previous instructions` vanishes entirely",
    ),
    (
        "hunk-unbounded",
        "detect.py",
        r'_HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+\d+(?:,\d+)? @@")',
        r'_HUNK = re.compile(r"^@@.*@@")',
        "the hunk pattern matches anything between two `@@`, so `@@ Ignore all previous "
        "instructions @@` is swallowed whole — the line is dropped with its prose",
    ),
    (
        "marker-not-stripped",
        "detect.py",
        '        residue = _DECORATION.sub("", _MARKER.sub("", stripped, count=1))',
        '        residue = _DECORATION.sub("", stripped)',
        "a diff-marked decoration line keeps its marker as a word, so `+---` wedges "
        "apart a phrase every tell needs adjacent",
    ),
    (
        "decoration-drops-the-line",
        "detect.py",
        '        residue = _DECORATION.sub("", _MARKER.sub("", stripped, count=1))',
        '        residue = "" if _DECORATION.search(stripped) else stripped',
        "a decorated line is dropped rather than stripped, so any tell written inside "
        "decoration is hidden — the failure direction the contract exists to deny",
    ),
    (
        "structure-splits-prose-surfaces",
        "detect.py",
        "        if _STRUCTURE.match(stripped):\n"
        '            if entry_point == "pr_diff" and passages[-1]:',
        "        if _STRUCTURE.match(stripped):\n            if passages[-1]:",
        "diff structure ends a passage on surfaces that are not diffs, so a pasted "
        "file header splits a phrase on a PR body — bypass two, reopened",
    ),
    (
        "oversized-discards-text",
        "fetch.py",
        '            text=text,\n            reason=f"{len(text.encode(\'utf-8\'))} bytes exceeds the "',
        '            reason=f"{len(text.encode(\'utf-8\'))} bytes exceeds the "',
        "a payload padded past the per-entry cap loses its text before findings are "
        "computed, so padding buys silence",
    ),
]

#: Controls run against each mutant. Network-dependent ones are excluded: a mutant must
#: be caught by an offline control, or CI cannot rely on catching it.
CONTROLS = [
    "check_step2.py",
    "suite.py",
    "check_step8.py",
    "check_step9.py",
    "check_invariants.py",
]

failures: list[str] = []


def apply_mutation(root: Path, filename: str, find: str, replace: str) -> bool:
    path = root / filename
    src = path.read_text(encoding="utf-8")
    if find not in src:
        return False
    path.write_text(src.replace(find, replace, 1), encoding="utf-8")
    return True


def catchers(root: Path) -> list[str]:
    """Which controls fail against the mutated tree."""
    caught = []
    for control in CONTROLS:
        proc = subprocess.run(
            [sys.executable, str(root / control)],
            capture_output=True,
            text=True,
            cwd=root,
            timeout=180,
        )
        if proc.returncode != 0:
            caught.append(control)
    return caught


def main() -> int:
    print(f"{len(MUTATIONS)} mutations, each must be caught by at least one control\n")
    for name, filename, find, replace, why in MUTATIONS:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "review-agent"
            shutil.copytree(HERE, root, ignore=shutil.ignore_patterns("__pycache__"))

            if not apply_mutation(root, filename, find, replace):
                print(f"FAIL  {name:<24}could not apply — the anchor has drifted in {filename}")
                failures.append(name)
                continue

            caught_by = catchers(root)
            if caught_by:
                print(f"PASS  {name:<24}caught by {', '.join(caught_by)}")
            else:
                print(f"FAIL  {name:<24}SURVIVED — nothing caught it")
                print(f"      consequence: {why}")
                failures.append(name)

    print(f"\n{len(failures)} surviving mutant(s)")
    if failures:
        print("A surviving mutant is a hole in the suite, not a passing build.")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
