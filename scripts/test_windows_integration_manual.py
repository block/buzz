"""Exercise the shared integration workflow's real manual/BW dispatch guard."""
import os
from pathlib import Path
import re
import subprocess
import unittest

WORKFLOW = Path(__file__).resolve().parents[1] / '.github/workflows/windows-fork-integration.yml'

class IntegrationManualTests(unittest.TestCase):
    def guard(self, **values):
        text = WORKFLOW.read_text()
        match = re.search(r'      - name: Validate manual or BW source\n.*?        run: \|\n(.*?)(?=\n      -)', text, re.S)
        self.assertIsNotNone(match)
        assert match
        script = '\n'.join(line[10:] for line in match.group(1).splitlines())
        env = {**os.environ, 'GITHUB_REPOSITORY': 'Jari81/buzz',
               'GITHUB_REF_NAME': 'windows-integration', 'GITHUB_SHA': 'a' * 40,
               'FREEZE_SHA': 'a' * 40, 'BUILD_REQUEST_EVENT_ID': '',
               'PIPELINE_EVENT_ID': '', 'RELEASE_ID': '',
               'EXPECTED_WORKFLOW_SHA256': '', 'EXPECTED_MANIFEST_SCHEMA_SHA256': '', **values}
        return subprocess.run(['bash', '-c', script], env=env, text=True, capture_output=True)

    def test_manual_requires_no_issue_or_release_metadata(self):
        self.assertEqual(self.guard().returncode, 0)
        self.assertEqual(self.guard(GITHUB_REF_NAME='windows').returncode, 0)

    def test_manual_still_binds_source_and_fork(self):
        for values in ({'FREEZE_SHA': ''}, {'FREEZE_SHA': 'b' * 40},
                       {'GITHUB_REF_NAME': 'feature'}, {'GITHUB_REPOSITORY': 'other/buzz'}):
            with self.subTest(values=values):
                self.assertNotEqual(self.guard(**values).returncode, 0)

    def test_partial_bw_cannot_fall_back_to_manual(self):
        for field in ('PIPELINE_EVENT_ID', 'RELEASE_ID', 'EXPECTED_WORKFLOW_SHA256',
                      'EXPECTED_MANIFEST_SCHEMA_SHA256'):
            with self.subTest(field=field):
                self.assertNotEqual(self.guard(**{field: 'present'}).returncode, 0)

    def test_bw_stays_on_canonical_windows_branch(self):
        self.assertNotEqual(self.guard(BUILD_REQUEST_EVENT_ID='b' * 64).returncode, 0)
        # Metadata/signature digest checks remain in the existing BW preflight.
        self.assertEqual(self.guard(BUILD_REQUEST_EVENT_ID='b' * 64,
                                    GITHUB_REF_NAME='windows').returncode, 0)

    def test_bw_preflight_and_manifest_are_conditional_not_disabled(self):
        text = WORKFLOW.read_text()
        self.assertIn("- name: Validate dispatch bindings\n        if: inputs.build_request_event_id != ''", text)
        self.assertIn("- name: Stage installer and generate bound manifest\n        if: inputs.build_request_event_id != ''", text)
        self.assertIn("- name: Stage manual test installer\n        if: inputs.build_request_event_id == ''", text)
        self.assertIn('manual-test-only', text)
        self.assertNotIn('continue-on-error: true', text)

if __name__ == '__main__':
    unittest.main()
