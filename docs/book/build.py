#!/usr/bin/env python3
"""Assemble the mdBook site from the Markdown already tracked in this repo.

Design notes
------------
Pages are *discovered*, not listed. Every tracked `.md` file is published
unless it matches EXCLUDE, so a new document appears in the book by simply
existing. The inverse — an explicit include list — fails silently: a file
nobody remembered to add is indistinguishable from a file that was never
written.

Discovered files are copied into `src/` at their original repo paths. That is
deliberate: relative links between documents (`[NIP-AE](NIP-AE.md)`,
`[architecture](../../ARCHITECTURE.md)`) keep working untouched, because the
distance between any two files is preserved. mdBook rewrites `.md` targets to
`.html` at build time, so no link rewriting happens here.

Anything that matches no SECTION rule is still published, under "Unsorted",
and named on stdout. New docs are therefore loud rather than lost.

Usage:  python3 docs/book/build.py [--out DIR] [--check]
"""

from __future__ import annotations

import argparse
import pathlib
import re
import shutil
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]

# Machine-facing Markdown: agent instructions, prompt fragments, persona
# definitions, skill manifests, templates and test fixtures. These are part of
# how the project is built, not documentation about it.
EXCLUDE = [
    r"^\.github/",
    r"^\.(agents|claude|codex|goose)/",
    r"^AGENTS\.md$",
    r"^CLAUDE\.md$",
    r"/tests/fixtures/",
    r"^benchmarks/.*/personas/",
    r"\.persona\.md$",
    r"/SKILL\.md$",
    r"^crates/buzz-acp/src/base_prompt\.md$",
    r"^desktop/src-tauri/src/managed_agents/.*\.md$",
    r"^desktop/src/features/agents/AGENTS\.md$",
    r"^desktop/src-tauri/resources/",
    r"^mobile/ios/",
    r"^docs/book/",
]

# Ordered; first match wins. (section title, path pattern)
SECTIONS: list[tuple[str, str]] = [
    ("Architecture", r"^(ARCHITECTURE|NOSTR)\.md$"),
    ("Architecture", r"^docs/(multi-tenant-|bridge-|git-on-object-storage)"),
    ("Architecture", r"^perf/"),
    ("Protocol (NIPs)", r"NIP-[A-Z]{2}\.md$"),
    ("Agents", r"^VISION_(AGENT|REMOTE_AGENTS)\.md$"),
    ("Agents", r"^crates/buzz-(acp|agent|cli)/README\.md$"),
    ("Agents", r"^docs/(MCP_DRIVEN_HOOKS|buzz-shared-compute-dev|welcome-kickoff)"),
    ("Self-Hosting", r"^deploy/"),
    ("Self-Hosting", r"^docs/(push-gateway|admin/|linux-rendering)"),
    ("Self-Hosting", r"^scripts/cutover/"),
    ("Vision", r"^VISION"),
    ("Formal Specs", r"^docs/formal/"),
    ("Apps", r"^(desktop|mobile)/"),
    ("Examples", r"^(examples|benchmarks)/"),
    ("Crates", r"^crates/"),
    ("Project", r"^(CONTRIBUTING|TESTING|RELEASING|SECURITY|CHANGELOG|CODE_OF_CONDUCT|GOVERNANCE)\.md$"),
    ("Project", r"^bin/"),
]


# Section order follows first appearance in SECTIONS, so adding a rule is the
# only step needed — there is no second list to keep in sync. "Unsorted" is
# always last.
SECTION_ORDER = list(dict.fromkeys(title for title, _ in SECTIONS)) + ["Unsorted"]

FENCE = re.compile(r"^\s*(```|~~~)")
SETEXT = re.compile(r"^(.+)\n[=-]{3,}\s*$", re.MULTILINE)


GITHUB_BLOB = "https://github.com/block/buzz/blob/main/"
MD_LINK = re.compile(r"(\]\()([^)\s]+?\.md)((?:#[^)\s]*)?)(\))")


def resolve_links(out: pathlib.Path, published: set[str]) -> list[tuple[str, str]]:
    """Repoint links that the copy step would otherwise leave dangling.

    Paths are preserved on copy, so most relative links already resolve. Three
    cases still need handling:

      * mdBook renders `README.md` as `index.html`, so links *to* a README
        must be rewritten or they 404.
      * Links to files excluded from the book (agent instructions, prompts)
        are sent to GitHub, where they do exist.
      * Anything else is already broken in the repo. Those are reported rather
        than rewritten — the fix belongs in the source document.
    """
    broken: list[tuple[str, str]] = []

    for page in sorted(out.rglob("*.md")):
        if page.name == "SUMMARY.md":
            continue
        rel = page.relative_to(out).as_posix()
        here = pathlib.PurePosixPath(rel).parent
        lines, in_fence, changed = [], False, False

        for line in page.read_text(encoding="utf-8").split("\n"):
            if FENCE.match(line):
                in_fence = not in_fence
                lines.append(line)
                continue
            if in_fence:
                lines.append(line)
                continue

            def fix(match: re.Match[str]) -> str:
                nonlocal changed
                prefix, target, anchor, close = match.groups()
                if re.match(r"^(https?:|mailto:|/)", target):
                    return match.group(0)
                resolved = str((here / target).as_posix())
                while "/../" in resolved or resolved.startswith("../"):
                    resolved = re.sub(r"[^/]+/\.\./", "", resolved, count=1)
                    if resolved.startswith("../"):
                        break
                if resolved in published:
                    if pathlib.PurePosixPath(resolved).name == "README.md":
                        changed = True
                        return f"{prefix}{target[:-len('README.md')]}index.md{anchor}{close}"
                    return match.group(0)
                if (REPO / resolved).is_file():
                    changed = True
                    return f"{prefix}{GITHUB_BLOB}{resolved}{anchor}{close}"
                broken.append((rel, target))
                return match.group(0)

            lines.append(MD_LINK.sub(fix, line))

        if changed:
            page.write_text("\n".join(lines), encoding="utf-8")

    return broken


def tracked_markdown() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "*.md"], cwd=REPO, capture_output=True, text=True, check=True
    )
    return sorted(out.stdout.split())


def title_of(path: pathlib.Path, rel: str) -> str:
    """First ATX or setext heading, else a name derived from the path."""
    text = path.read_text(encoding="utf-8", errors="replace")
    atx = re.search(r"^#\s+(.+)$", text, re.MULTILINE)
    setext = SETEXT.search(text)
    if atx and (not setext or atx.start() < setext.start()):
        return atx.group(1).strip()
    if setext:
        return setext.group(1).strip()
    stem = pathlib.Path(rel).stem
    if stem.upper() in {"README", "NOTE"}:
        parent = pathlib.Path(rel).parent.name
        return parent or stem
    return stem.replace("_", " ").replace("-", " ").title()


def section_of(rel: str) -> str:
    for title, pattern in SECTIONS:
        if re.search(pattern, rel):
            return title
    return "Unsorted"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default=str(pathlib.Path(__file__).parent / "src"))
    parser.add_argument(
        "--check", action="store_true",
        help="fail if any document lands in Unsorted",
    )
    args = parser.parse_args()

    out = pathlib.Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    excluded = re.compile("|".join(EXCLUDE))
    grouped: dict[str, list[tuple[str, str]]] = {}
    published: set[str] = set()
    intro: str | None = None

    for rel in tracked_markdown():
        if excluded.search(rel):
            continue
        source = REPO / rel
        destination = out / rel
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        published.add(rel)
        if rel == "README.md":
            intro = rel
            continue
        grouped.setdefault(section_of(rel), []).append((title_of(source, rel), rel))

    broken = resolve_links(out, published)

    lines = ["# Summary", ""]
    if intro:
        lines += [f"[Introduction]({intro})", ""]
    for section in SECTION_ORDER:
        entries = grouped.get(section)
        if not entries:
            continue
        lines.append(f"# {section}")
        lines.append("")
        for title, rel in sorted(entries):
            lines.append(f"- [{title}]({rel})")
        lines.append("")
    (out / "SUMMARY.md").write_text("\n".join(lines), encoding="utf-8")

    orphaned = set(grouped) - set(SECTION_ORDER)
    if orphaned:
        raise AssertionError(f"sections with no slot in SECTION_ORDER: {sorted(orphaned)}")

    written = sum(len(grouped[s]) for s in SECTION_ORDER if s in grouped)
    total = written + (1 if intro else 0)
    print(f"published {total} pages")
    for section in SECTION_ORDER:
        if grouped.get(section):
            print(f"  {section}: {len(grouped[section])}")

    if broken:
        print(f"\n{len(broken)} link(s) already broken in the source docs:", file=sys.stderr)
        for page_rel, target in sorted(set(broken)):
            print(f"  {page_rel} -> {target}", file=sys.stderr)

    unsorted = grouped.get("Unsorted", [])
    if unsorted:
        print("\nUnsorted — add a SECTIONS rule or an EXCLUDE entry:", file=sys.stderr)
        for _, rel in sorted(unsorted, key=lambda e: e[1]):
            print(f"  {rel}", file=sys.stderr)
        if args.check:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
