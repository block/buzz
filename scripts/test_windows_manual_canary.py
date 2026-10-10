"""Manual Windows test lane: execute the actual dispatch guard, offline."""
import os
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / '.github/workflows/windows-canary.yml'


class ManualCanaryTests(unittest.TestCase):
    def guard(self, repository, ref, requested, actual='a' * 40):
        source = WORKFLOW.read_text()
        match = re.search(r'      - name: Validate manual source\n.*?        run: \|\n(.*?)(?=\n      -)', source, re.S)
        self.assertIsNotNone(match, 'commit-bound manual source guard is missing')
        assert match is not None
        script = '\n'.join(line[10:] for line in match.group(1).splitlines())
        env = {**os.environ, 'GITHUB_REPOSITORY': repository, 'SOURCE_REF': ref,
               'SOURCE_SHA': requested, 'GITHUB_SHA': actual}
        return subprocess.run([os.environ.get('BUZZ_TEST_BASH', 'bash'), '-c', script], env=env, capture_output=True, text=True)

    def test_guard_honors_explicit_bash(self):
        from unittest.mock import patch
        import shutil
        # Simulate Windows PATH selecting the wrong bash.exe.
        real_run = subprocess.run
        executable = os.environ.get('BUZZ_TEST_BASH') or shutil.which('bash')
        assert executable
        def checked_run(command, **kwargs):
            self.assertEqual(command[0], executable)
            self.assertEqual(command[1], '-c', 'avoid Windows text-stdin CRLF translation')
            return real_run(command, **kwargs)
        with patch.dict(os.environ, {'BUZZ_TEST_BASH': executable}), patch.object(subprocess, 'run', side_effect=checked_run):
            result = self.guard('Jari81/buzz', 'refs/heads/windows', 'a' * 40)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_fork_requires_exact_sha_and_windows_branch(self):
        self.assertEqual(self.guard('Jari81/buzz', 'refs/heads/windows', 'a' * 40).returncode, 0)
        for ref, sha in [('refs/heads/main', 'a' * 40), ('refs/tags/windows', 'a' * 40),
                         ('refs/heads/windows', ''), ('refs/heads/windows', 'b' * 40),
                         ('refs/heads/windows', '$(touch /tmp/should-not-execute)')]:
            with self.subTest(ref=ref, sha=sha):
                self.assertNotEqual(self.guard('Jari81/buzz', ref, sha).returncode, 0)

    def test_lineage_loop_accepts_windows_crlf(self):
        import tempfile
        source = WORKFLOW.read_text()
        match = re.search(r'          while IFS= read -r previous_sha; do\n(.*?)          done < previous-windows-shas.txt', source, re.S)
        self.assertIsNotNone(match)
        assert match is not None
        script = 'set -eu\nwhile IFS= read -r previous_sha; do\n' + match.group(1) + 'done < previous-windows-shas.txt\n'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            verifier = root / 'scripts/verify-windows-client-lineage.sh'
            verifier.write_bytes(b'#!/usr/bin/env bash\n[[ "$1" =~ ^[0-9a-f]{40}$ ]]\n')
            verifier.chmod(0o755)
            (root / 'previous-windows-shas.txt').write_bytes(('a' * 40 + '\r\n').encode())
            result = subprocess.run([os.environ.get('BUZZ_TEST_BASH', 'bash'), '-c', script], cwd=root,
                                    env={**os.environ, 'GITHUB_SHA': 'b' * 40, 'GITHUB_REF_NAME': 'windows'},
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_preserves_upstream_main_only(self):
        self.assertEqual(self.guard('block/buzz', 'refs/heads/main', '').returncode, 0)
        self.assertNotEqual(self.guard('block/buzz', 'refs/heads/windows', '').returncode, 0)
        self.assertNotEqual(self.guard('stranger/buzz', 'refs/heads/windows', 'a' * 40).returncode, 0)

    def test_cumulative_checks_precede_compilation_and_no_bw_mutation(self):
        source = WORKFLOW.read_text()
        self.assertIn('actions: read', source)
        self.assertLess(source.index('Validate cumulative fork source'), source.index('Add Rust target'))
        for marker in ['windows-fork-integration.yml/runs', 'windows-canary.yml/runs',
                       'scripts/verify-windows-client-lineage.sh', 'scripts/check-windows-client-contract.sh',
                       '83afea0a1549410a8f7effa12ceed0f7b13234e6', 'manual-test-build.json',
                       'manual-test-only', 'Exactly one NSIS installer required']:
            self.assertIn(marker, source)
        self.assertNotIn('build_request_event_id', source)
        self.assertNotIn('contents: write', source)
        self.assertNotIn('gh release create', source)
        self.assertIn('"createUpdaterArtifacts": false', source)
        # Integration now also admits explicitly manual builds. BW checks remain
        # conditional on real request metadata, never fabricated by this canary.
        bw = (ROOT / '.github/workflows/windows-fork-integration.yml').read_text()
        self.assertIn("- name: Validate dispatch bindings\n        if: inputs.build_request_event_id != ''", bw)
        self.assertIn('scripts/windows_build_manifest.py preflight', bw)


if __name__ == '__main__':
    unittest.main()
