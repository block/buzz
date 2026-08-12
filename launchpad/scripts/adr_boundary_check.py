#!/usr/bin/env python3
"""Check that ADR-0005's sanctioned-file list and AGENTS.md section 3 agree.

ADR-0005 names the upstream files Launchpad is permitted to modify and calls the
list closed. AGENTS.md section 3 repeats that list in its own words. Two copies of
one list drift, and the drift is invisible until an upstream merge conflicts on a
file nobody remembered was sanctioned.

The ADR is the source of truth here. This does not hardcode the list; it reads the
ADR's table, checks the ADR's own prose count agrees with it, then requires
AGENTS.md and the filesystem to match. Adding a sixth file therefore means editing
the record, which is what the ADR says it means.

Usage:
    python3 adr_boundary_check.py [repo-root]

Exits 0 when every check passes, 1 otherwise. Failures print one per line.
"""

import re
import sys
from pathlib import Path

ADR_REL = "launchpad/decisions/ADR-0005-launchpad-deployment-boundary.md"
AGENTS_REL = "launchpad/AGENTS.md"

# The ADR states its own count in prose. Spelled out, so map the words we accept.
NUMBER_WORDS = {
    "one": 1, "two": 2, "three": 3, "four": 4, "five": 5,
    "six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10,
}


def strip_fences(text):
    """Remove ``` fenced blocks.

    Both documents contain fenced examples. Pairing inline backticks with a regex
    across a fence mis-pairs an opening fence with a later backtick and swallows
    whole sections, which reports a correct document as broken.
    """
    return re.sub(r"```.*?```", "", text, flags=re.DOTALL)


def parse_adr_files(adr_text):
    """Return the file paths in the ADR's sanctioned-file table.

    Table rows look like: | `path/to/file` | what it carries | why not an override |
    Only the first cell is a path, so rows are matched rather than every backtick
    in the document.
    """
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
    if not m:
        return None
    return NUMBER_WORDS.get(m.group(1).lower())


def check_documents(adr_text, agents_text, exists):
    """Compare the two documents and the tree. Returns a list of failure strings.

    `exists` is a callable taking a repo-relative path and returning a bool, so the
    comparison logic is testable without a real checkout.
    """
    failures = []
    adr_body = strip_fences(adr_text)
    agents_body = strip_fences(agents_text)

    table_paths = parse_adr_files(adr_text)
    table_set = set(table_paths)

    if not table_paths:
        failures.append("ADR names no sanctioned files - the table is missing or unparseable")
        return failures

    if len(table_paths) != len(table_set):
        dupes = sorted({p for p in table_paths if table_paths.count(p) > 1})
        failures.append(f"ADR table lists a file twice: {dupes}")

    # The ADR's prose count and its own table must agree. If they disagree, the
    # record contradicts itself and neither number can be trusted downstream.
    prose = parse_prose_count(adr_text)
    if prose is None:
        failures.append("ADR states no sanctioned-file count in prose")
    elif prose != len(table_set):
        failures.append(
            f"ADR prose says {prose} sanctioned files, its table lists {len(table_set)}"
        )

    for rel in sorted(table_set):
        if not exists(rel):
            failures.append(f"sanctioned file named by the ADR does not exist: {rel}")

    # AGENTS.md must name the same set. Missing entries are the drift that matters;
    # an extra is equally wrong, because the list is closed.
    agents_named = {p for p in table_set if p in agents_body}
    missing = sorted(table_set - agents_named)
    if missing:
        failures.append(f"AGENTS.md does not name: {missing}")

    if "Three deliberate exceptions" not in agents_body:
        failures.append("AGENTS.md section 3 does not read 'Three deliberate exceptions'")

    # An ADR nothing routes to is invisible, which is the failure it documents.
    link = re.search(r"\((decisions/ADR-0005[^)]*\.md)\)", agents_body)
    if not link:
        failures.append("AGENTS.md does not link to the ADR")

    if not re.search(r"\(\.\./AGENTS\.md\)", adr_body):
        failures.append("ADR does not link back to AGENTS.md")

    if "#TBD" in adr_text:
        failures.append("ADR front matter still carries a #TBD placeholder")

    return failures


def main(argv):
    root = Path(argv[1] if len(argv) > 1 else ".").resolve()
    adr, agents = root / ADR_REL, root / AGENTS_REL

    missing = [str(p) for p in (adr, agents) if not p.is_file()]
    if missing:
        for m in missing:
            print(f"FAIL missing document: {m}")
        return 1

    failures = check_documents(
        adr.read_text(), agents.read_text(), lambda rel: (root / rel).exists()
    )

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
