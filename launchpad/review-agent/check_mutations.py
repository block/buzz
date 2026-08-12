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
        "            lines.append(escape(finding.evidence))",
        "            lines.append(finding.evidence)",
        "the published review quotes attacker text raw, becoming a fresh vector",
    ),
    (
        "fixed-fence",
        "review.py",
        "            lines.append(fence_for(finding.evidence))",
        '            lines.append("```")',
        "backticks in evidence break the fence and corrupt the rest of the review",
    ),
    (
        "cap-warns-only",
        "contain.py",
        '        return "\\n".join(lines), findings, False',
        '        pass',
        "oversized input is warned about but still rendered in full",
    ),
    (
        "empty-reads-as-absent",
        "fetch.py",
        '        return Surface(entry_point, "empty")',
        '        return Surface(entry_point, "absent", reason="no content")',
        "a fetched-and-empty surface becomes indistinguishable from one never read",
    ),
]

#: Controls run against each mutant. Network-dependent ones are excluded: a mutant must
#: be caught by an offline control, or CI cannot rely on catching it.
CONTROLS = [
    "check_step2.py",
    "suite.py",
    "check_step8.py",
    "check_step9.py",
    "check_step3.py",
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
