#!/usr/bin/env python3
"""Regenerates sanitizer_parity_expected_python_output.json by running the
REAL, pinned cccareers/buzz-auditor `collector.nip_am_pipeline_bridge.
sanitize_pipeline_line` against sanitizer_parity_examples.json -- so the
committed capture in this directory can be proven reproducible rather than
trusted as a one-off hand transcription.

Usage (from the block/buzz repo root, after running
scripts/mike49-pin-nip-am-verify.sh):

    python3 desktop/src-tauri/src/commands/mike49_audit/testdata/generate_sanitizer_parity_expected_python_output.py \\
        --auditor-src .mike49-nip-am-verify-pinned \\
        --check

`--check` (default) diffs the freshly generated output against the
committed file and exits non-zero on any difference, without writing
anything. Pass `--write` to overwrite the committed file instead -- only
do that deliberately, as part of re-capturing parity against a new pinned
revision, and re-run the Rust parity test afterward.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import asdict
from pathlib import Path

THIS_DIR = Path(__file__).resolve().parent
EXAMPLES_PATH = THIS_DIR / "sanitizer_parity_examples.json"
EXPECTED_PATH = THIS_DIR / "sanitizer_parity_expected_python_output.json"


def generate(auditor_src: Path) -> list[dict]:
    sys.path.insert(0, str(auditor_src / "src"))
    from collector.nip_am_pipeline_bridge import sanitize_pipeline_line  # noqa: E402

    examples = json.loads(EXAMPLES_PATH.read_text())
    results = []
    for example in examples:
        scenario = example["scenario"]
        sanitized = sanitize_pipeline_line(example)
        result = asdict(sanitized)
        results.append(
            {
                "scenario": scenario,
                "status": result["status"],
                "line": result["line"],
                "dropped_field_count": result["dropped_field_count"],
                "invalid_fields": list(result["invalid_fields"]),
                "blocked_field": result["blocked_field"],
            }
        )
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--auditor-src",
        type=Path,
        required=True,
        help="path to the pinned cccareers/buzz-auditor checkout "
        "(e.g. .mike49-nip-am-verify-pinned, produced by "
        "scripts/mike49-pin-nip-am-verify.sh)",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        default=True,
        help="diff against the committed capture; exit 1 on any difference (default)",
    )
    mode.add_argument(
        "--write",
        action="store_true",
        help="overwrite the committed capture instead of diffing against it",
    )
    args = parser.parse_args()

    generated = generate(args.auditor_src.resolve())
    generated_text = json.dumps(generated, indent=2, sort_keys=True) + "\n"

    if args.write:
        EXPECTED_PATH.write_text(generated_text)
        print(f"wrote {EXPECTED_PATH}")
        return 0

    committed = json.loads(EXPECTED_PATH.read_text())
    committed_text = json.dumps(committed, indent=2, sort_keys=True) + "\n"
    if generated_text != committed_text:
        print(
            f"mismatch: regenerating from the pinned Python reference does not "
            f"reproduce {EXPECTED_PATH}",
            file=sys.stderr,
        )
        return 1

    print(f"match: {EXPECTED_PATH} is reproducible from the pinned Python reference")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
