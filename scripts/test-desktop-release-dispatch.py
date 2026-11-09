#!/usr/bin/env python3
"""Exercise the workflow's real authorization shell with a fake GitHub API."""

import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest


WORKFLOW = (Path(__file__).resolve().parents[1] / ".github/workflows/desktop-release-candidate.yml").read_text()
AUTHORIZE = WORKFLOW.split("      - name: Authorize manual candidate validation\n", 1)[1].split("      - uses:", 1)[0]
SCRIPT = textwrap.dedent(AUTHORIZE.split("        run: |\n", 1)[1])


class ManualCandidateValidation(unittest.TestCase):
    def test_workflow_wiring(self):
        # An authorization script only protects the workflow if it runs before
        # checkout, with both real actor identities and the GitHub read token.
        self.assertLess(WORKFLOW.index(AUTHORIZE), WORKFLOW.index("actions/checkout@"))
        self.assertIn("        if: github.event_name == 'workflow_dispatch'\n", AUTHORIZE)
        self.assertIn("          DISPATCH_REF: ${{ github.ref }}\n", AUTHORIZE)
        self.assertIn("          TRIGGERING_ACTOR: ${{ github.triggering_actor }}\n", AUTHORIZE)
        self.assertIn("          GH_TOKEN: ${{ github.token }}\n", AUTHORIZE)
        self.assertNotIn("${{", SCRIPT)
        self.assertNotIn("continue-on-error:", WORKFLOW)
        self.assertIn("    branches: [main]\n", WORKFLOW)
        self.assertIn("  workflow_dispatch:\n    inputs:\n      version:\n", WORKFLOW)
        self.assertIn("        required: true\n        type: string\n", WORKFLOW)
        self.assertIn("format('refs/heads/version-bump/{0}', inputs.version) || github.event.pull_request.head.sha", WORKFLOW)
        self.assertIn("if: github.event_name == 'workflow_dispatch' || startsWith(github.event.pull_request.head.ref, 'version-bump/')", WORKFLOW)
        self.assertIn("VERSION: ${{ github.event_name == 'workflow_dispatch' && inputs.version || github.event.pull_request.head.ref }}", WORKFLOW)
        self.assertIn('scripts/desktop_release.py validate --candidate HEAD --version "$VERSION" --repo "$GITHUB_REPOSITORY"', WORKFLOW)
        self.assertIn("          persist-credentials: false\n", WORKFLOW)
        self.assertNotRegex(WORKFLOW, r"(?m)^\s+(contents|pull-requests): write$")

    def test_admin_authorization(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text('''#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == api && "$3" == --jq && "$4" == .permission ]] || exit 90
printf '%s\\n' "$2" >> "$API_CALLS"
case "$2" in
  repos/example-org/example-repo/collaborators/original/permission) permission=$ORIGINAL_PERMISSION ;;
  repos/example-org/example-repo/collaborators/rerunner/permission) permission=$RERUNNER_PERMISSION ;;
  *) exit 91 ;;
esac
[[ "$permission" != api-error ]] || exit 1
printf '%s\\n' "$permission"
''')
            gh.chmod(0o755)
            cases = [("admin", "admin", "refs/heads/main", True)]
            for permission in ("write", "maintain", "read", "triage", "none", "null", "", "api-error"):
                cases.extend([
                    (permission, "admin", "refs/heads/main", False),
                    ("admin", permission, "refs/heads/main", False),
                ])
            cases.extend(("admin", "admin", ref, False) for ref in (
                "refs/heads/version-bump/1.2.3", "refs/tags/main", "refs/heads/feature",
            ))
            for original, rerunner, ref, allowed in cases:
                with self.subTest(original=original, rerunner=rerunner, ref=ref):
                    calls = root / "calls"
                    calls.write_text("")
                    result = subprocess.run(
                        ["bash", "-e", "-o", "pipefail", "-c", SCRIPT],
                        env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                             "GH_TOKEN": "test-token", "GITHUB_REPOSITORY": "example-org/example-repo",
                             "GITHUB_ACTOR": "original", "TRIGGERING_ACTOR": "rerunner",
                             "DISPATCH_REF": ref, "ORIGINAL_PERMISSION": original,
                             "RERUNNER_PERMISSION": rerunner, "API_CALLS": str(calls)},
                        text=True, capture_output=True, timeout=10,
                    )
                    self.assertEqual(result.returncode == 0, allowed, result.stdout + result.stderr)
                    if allowed:
                        self.assertEqual(len(calls.read_text().splitlines()), 2)
                    if ref != "refs/heads/main":
                        self.assertEqual(calls.read_text(), "")


if __name__ == "__main__":
    unittest.main()
