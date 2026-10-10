#!/usr/bin/env python3
"""Static contract tests for the fork-only Windows build workflow."""

from pathlib import Path
import re
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "windows-fork-integration.yml"


class WindowsWorkflowContractTests(unittest.TestCase):
    def test_workflow_binds_dispatch_lineage_and_one_unsigned_manifest_artifact(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            "run-name: Windows Fork Integration ${{ inputs.build_request_event_id && 'BW' || 'manual' }} @ ${{ inputs.freeze_sha }}",
            source,
        )
        for name in (
            "release_id",
            "freeze_sha",
            "build_request_event_id",
            "pipeline_event_id",
            "expected_workflow_sha256",
            "expected_manifest_schema_sha256",
        ):
            self.assertRegex(
                source,
                re.compile(
                    rf"(?m)^      {name}:\n"
                    r"        description: .+\n"
                    rf"        required: {'true' if name == 'freeze_sha' else 'false'}\n"
                    r"        type: string$"
                ),
            )

        self.assertIn("CANONICAL_WINDOWS_BRANCH: windows", source)
        self.assertIn(
            "WINDOWS_BOOTSTRAP_SHA: 83afea0a1549410a8f7effa12ceed0f7b13234e6",
            source,
        )
        self.assertIn("branch=${CANONICAL_WINDOWS_BRANCH}", source)
        self.assertIn('previous_sha="$WINDOWS_BOOTSTRAP_SHA"', source)
        self.assertIn("scripts/verify-windows-client-lineage.sh", source)
        self.assertIn("scripts/check-windows-client-contract.sh", source)

        preflight = source.index("- name: Validate dispatch bindings")
        previous = source.index("- name: Resolve previous successful Windows build")
        install = source.index("- name: Install dependencies")
        build = source.index("- name: Build unsigned Windows NSIS installer")
        self.assertLess(preflight, previous)
        self.assertLess(preflight, install)
        self.assertLess(preflight, build)
        self.assertIn("scripts/windows_build_manifest.py preflight", source)
        self.assertIn('--github-sha "$GITHUB_SHA"', source)
        self.assertIn('--freeze-sha "$FREEZE_SHA"', source)
        self.assertIn('--expected-workflow-sha256 "$EXPECTED_WORKFLOW_SHA256"', source)
        self.assertIn(
            '--expected-manifest-schema-sha256 "$EXPECTED_MANIFEST_SCHEMA_SHA256"',
            source,
        )
        self.assertIn('--github-repository "$GITHUB_REPOSITORY"', source)

        self.assertNotIn("find \"$BUNDLE_DIR/nsis\" -name '*.exe' -type f | head -1", source)
        self.assertIn("scripts/windows_build_manifest.py generate", source)
        self.assertIn("scripts/windows_build_manifest.py validate", source)
        self.assertIn('--installer-dir "$BUNDLE_DIR/nsis"', source)
        self.assertIn('--output-dir "$ARTIFACT_DIR"', source)
        self.assertIn(
            "name: ${{ inputs.build_request_event_id && format('buzz-windows-{0}', github.sha) || format('buzz-windows-integration-{0}', github.sha) }}", source
        )
        self.assertIn("path: windows-build-artifact", source)
        self.assertIn("Upload unsigned installer and bound manifest", source)

        for pinned_action in (
            "actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10",
            "actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020",
            "pnpm/action-setup@b906affcce14559ad1aafd4ab0e942779e9f58b1",
            "Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32",
            "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        ):
            self.assertIn(pinned_action, source)


if __name__ == "__main__":
    unittest.main()
