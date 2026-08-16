#!/usr/bin/env python3
"""Check that ADR-0005's sanctioned-file list is honest, and that it is real.

ADR-0005 names the upstream files Launchpad is permitted to modify and calls the
list closed. AGENTS.md section 3 repeats that list, and tells reviewers not to
raise it in review. Two copies of one list drift; and a list nobody may question
in review needs something else to question it.

Two independent things are checked, because an earlier version checked only the
first and passed:

1. **The list agrees with itself.** The ADR's table, the ADR's own prose count,
   and AGENTS.md section 3 must name the same files.
2. **The exceptions are actually used.** A file may be sanctioned to carry a
   Launchpad value only if it carries one. The first version asserted
   `Path.exists()` on five files that exist upstream anyway, so it reported
   `failed: 0` on a tree where compose.yml still defaulted to
   `ghcr.io/block/buzz:main` and the Dockerfile still labelled `block/buzz`. A
   green check was compatible with the boundary never having landed — while
   AGENTS.md told reviewers not to look. That is worse than no check.

Usage:
    python3 adr_boundary_check.py [repo-root]

Exits 0 when every check passes, 1 otherwise. Failures print one per line.
"""

import re
import sys
from pathlib import Path

ADR_REL = "launchpad/decisions/ADR-0005-launchpad-deployment-boundary.md"
AGENTS_REL = "launchpad/AGENTS.md"

# The exception entry in AGENTS.md section 3 that this ADR governs. The file list
# is read from the ADR, never from here — but the check has to know WHERE in
# AGENTS.md to look, or a path mentioned anywhere in the document would satisfy
# it. That was a real defect: plain `path in whole_document`.
AGENTS_ENTRY_MARKER = "Deployment image provenance"

# WHAT EACH SANCTIONED FILE MUST CARRY.
#
# This is not a second copy of the list — the list comes from the ADR. These are
# assertions about CONTENT, keyed by path, and they exist because "the file is
# present" says nothing. Each entry is (forbidden_substring, why) or
# (None, required_substring, why).
#
# A path in the ADR with no entry here is reported rather than silently skipped:
# adding a sixth sanctioned file must not quietly widen the boundary.
FORBIDDEN = {
    "deploy/compose/compose.yml": (
        "ghcr.io/block/buzz",
        "still defaults to upstream's image, so a clean checkout deploys Block's build",
    ),
    "deploy/compose/.env.example": (
        "ghcr.io/block/buzz",
        "the file operators copy still names upstream's image",
    ),
    "Dockerfile": (
        "github.com/block/buzz",
        "OCI labels still attribute the image to upstream",
    ),
}
REQUIRED = {
    "deploy/compose/README.md": (
        "launchpad/deploy",
        "does not point operators at the Launchpad wrapper",
    ),
}

# .github/workflows/docker.yml needs more than the FORBIDDEN/REQUIRED shape above
# can express. That file also builds and publishes ghcr.io/block/buzz-push-gateway,
# an unrelated component this fork deliberately leaves under upstream's ownership
# (disabled here via `if: github.repository == 'block/buzz'`, never deleted) — and
# "ghcr.io/block/buzz" is a literal prefix of that name. A plain forbidden-substring
# check that ignores the distinction reports the relay as still publishing to
# upstream on every commit, forever, because it can never stop matching a string
# this file is supposed to keep. And a check that only asserts the forbidden
# substring's absence cannot tell "the relay moved to Launchpad's namespace" from
# "the relay moved to some other namespace that also isn't Block's" — so the
# sanctioned values are asserted positively here too, not just by absence.
DOCKER_YML_ASSERTIONS = [
    (
        "forbid_re",
        re.compile(r"ghcr\.io/block/buzz(?!-push-gateway)"),
        "publishes the relay to upstream's namespace",
    ),
    (
        "require",
        "ghcr.io/launchpad-26/buzz",
        "does not publish the relay to Launchpad's namespace",
    ),
    (
        "require",
        "branches: [launchpad]",
        "does not trigger relay publication on push to launchpad",
    ),
    (
        "require",
        "if: github.repository == 'block/buzz'",
        "does not keep the upstream-only push-gateway jobs disabled for this fork",
    ),
]

NUMBER_WORDS = {
    "one": 1, "two": 2, "three": 3, "four": 4, "five": 5,
    "six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10,
}


def strip_fences(text):
    """Remove ``` fenced blocks.

    Both documents contain fenced examples. Pairing inline backticks across a
    fence mis-pairs an opening fence with a later backtick and swallows whole
    sections, which reports a correct document as broken.
    """
    return re.sub(r"```.*?```", "", text, flags=re.DOTALL)


def parse_adr_files(adr_text):
    """Return the file paths in the ADR's sanctioned-file table, in order."""
    paths = []
    for line in strip_fences(adr_text).splitlines():
        row = re.match(r"^\|\s*`([^`]+)`\s*\|", line)
        if row:
            paths.append(row.group(1))
    return paths


def parse_prose_count(adr_text):
    """Return the count the ADR states in prose, or None if it states none."""
    m = re.search(
        r"\*\*(\w+)\s+files?\s+are\s+sanctioned", strip_fences(adr_text), re.I
    )
    return NUMBER_WORDS.get(m.group(1).lower()) if m else None


def agents_exception_entry(agents_text):
    """Return only the section-3 bullet that governs this ADR, or None.

    Scoped deliberately. Asking whether a path appears ANYWHERE in AGENTS.md is
    satisfied by any unrelated mention — `Dockerfile` appears in ordinary prose —
    so the check passed on documents that never listed the file as sanctioned.
    """
    body = strip_fences(agents_text)
    start = body.find(AGENTS_ENTRY_MARKER)
    if start == -1:
        return None
    # The entry runs to the next top-level bullet or blank-line-separated block.
    rest = body[start:]
    end = re.search(r"\n\s*\n", rest)
    return rest[: end.start()] if end else rest


def check_documents(adr_text, agents_text, read_file):
    """Compare the documents and the tree. Returns a list of failure strings.

    `read_file` takes a repo-relative path and returns its text, or None when the
    file does not exist, so the comparison is testable without a checkout.
    """
    failures = []
    adr_body = strip_fences(adr_text)

    table_paths = parse_adr_files(adr_text)
    table_set = set(table_paths)
    if not table_paths:
        return ["ADR names no sanctioned files - the table is missing or unparseable"]

    if len(table_paths) != len(table_set):
        dupes = sorted({p for p in table_paths if table_paths.count(p) > 1})
        failures.append(f"ADR table lists a file twice: {dupes}")

    prose = parse_prose_count(adr_text)
    if prose is None:
        failures.append("ADR states no sanctioned-file count in prose")
    elif prose != len(table_set):
        failures.append(
            f"ADR prose says {prose} sanctioned files, its table lists {len(table_set)}"
        )

    # --- 1. the list agrees with itself -----------------------------------
    entry = agents_exception_entry(agents_text)
    if entry is None:
        failures.append(
            f"AGENTS.md section 3 has no '{AGENTS_ENTRY_MARKER}' exception entry"
        )
    else:
        missing = sorted(p for p in table_set if p not in entry)
        if missing:
            failures.append(
                f"AGENTS.md's exception entry does not name: {missing}"
            )
        if not re.search(r"\(decisions/ADR-0005[^)]*\.md\)", entry):
            failures.append("AGENTS.md's exception entry does not link to the ADR")

    # The count of section-3 exceptions is deliberately NOT asserted. An earlier
    # version required the literal "Three deliberate exceptions", which meant a
    # legitimate fourth exception - unrelated to deployment - would turn an
    # intended-required check red for the whole repository.

    if not re.search(r"\(\.\./AGENTS\.md\)", adr_body):
        failures.append("ADR does not link back to AGENTS.md")

    if "#TBD" in adr_text:
        failures.append("ADR front matter still carries a #TBD placeholder")

    # --- 2. the exceptions are actually used ------------------------------
    for rel in sorted(table_set):
        content = read_file(rel)
        if content is None:
            failures.append(f"sanctioned file named by the ADR does not exist: {rel}")
            continue

        if rel == ".github/workflows/docker.yml":
            for kind, pattern, why in DOCKER_YML_ASSERTIONS:
                if kind == "forbid_re":
                    if pattern.search(content):
                        failures.append(f"{rel} {why} (matches {pattern.pattern!r})")
                elif kind == "require":
                    if pattern not in content:
                        failures.append(f"{rel} {why} (no {pattern!r})")
        elif rel in FORBIDDEN:
            needle, why = FORBIDDEN[rel]
            if needle in content:
                failures.append(f"{rel} {why} (still contains '{needle}')")
        elif rel in REQUIRED:
            needle, why = REQUIRED[rel]
            if needle not in content:
                failures.append(f"{rel} {why} (no '{needle}')")
        else:
            # Fail rather than skip. A sixth file added to the ADR with no content
            # assertion here would otherwise be sanctioned on the strength of
            # existing, which is the defect this section was written to close.
            failures.append(
                f"{rel} is sanctioned by the ADR but this check asserts nothing about "
                f"its content - add it to FORBIDDEN or REQUIRED before widening the list"
            )

    return failures


def main(argv):
    root = Path(argv[1] if len(argv) > 1 else ".").resolve()
    adr, agents = root / ADR_REL, root / AGENTS_REL

    missing = [str(p) for p in (adr, agents) if not p.is_file()]
    if missing:
        for m in missing:
            print(f"FAIL missing document: {m}")
        return 1

    def read_file(rel):
        p = root / rel
        try:
            return p.read_text()
        except OSError:
            return None

    failures = check_documents(adr.read_text(), agents.read_text(), read_file)

    link = re.search(
        r"\((decisions/ADR-0005[^)]*\.md)\)", strip_fences(agents.read_text())
    )
    if link and not (agents.parent / link.group(1)).is_file():
        failures.append(f"AGENTS.md link does not resolve: {link.group(1)}")

    for f in failures:
        print(f"FAIL {f}")
    print(f"failed: {len(failures)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
