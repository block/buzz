"""Exercise the real publishing CLI with Git snapshots and a fake registry CLI."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("manual_image", ROOT / "scripts/manual_relay_image.py")
IMAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(IMAGE)

FAKE_DOCKER = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
a=sys.argv[1:]
with open(os.environ['TRACE'], 'a') as f: f.write(json.dumps(a)+'\n')
mode=os.environ.get('FAKE_MODE','')
digest='sha256:'+'a'*64
if a[:2]==['buildx','build']:
    # Prove the build used the selected clean snapshot, not caller modifications.
    assert pathlib.Path('Dockerfile').read_text()=='FROM scratch AS runtime\n'
    assert not pathlib.Path('dirty-untracked').exists()
    if mode=='build-failure': sys.exit(7)
    pathlib.Path(a[a.index('--metadata-file')+1]).write_text(json.dumps({'containerimage.digest':digest}))
elif a[:3]==['buildx','imagetools','inspect']:
    if '--raw' in a:
        arches=['amd64'] if mode=='missing-arm64' else ['amd64','arm64']
        print(json.dumps({'manifests':[{'platform':{'os':'linux','architecture':arch},'digest':'sha256:'+str(i+1)*64} for i,arch in enumerate(arches)]}))
    elif '.Manifest' in a[-1]:
        print(json.dumps({'digest':digest}))
    else:
        arch='amd64' if a[3].endswith('1'*64) else 'arm64'
        sha='0'*40 if mode=='wrong-sha' and arch=='arm64' else os.environ['SOURCE_SHA']
        print(json.dumps({'os':'linux','architecture':arch,'config':{'Labels':{
          'org.opencontainers.image.revision':sha,
          'org.opencontainers.image.source':'https://github.com/example-org/relay'}}}))
else:
    print('fake Docker ready')
'''


class PublishingCLI(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        self.git("config", "core.fsmonitor", "false")
        self.git("config", "user.name", "Example Operator")
        self.git("config", "user.email", "operator@example.org")
        self.git("remote", "add", "origin", "https://github.com/example-org/relay.git")
        (self.repo / "Dockerfile").write_text("FROM scratch AS runtime\n")
        self.git("add", "Dockerfile")
        self.git("commit", "-q", "-s", "-m", "Fixture")
        self.sha = self.git("rev-parse", "HEAD").strip()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        docker = self.bin / "docker"
        docker.write_text(FAKE_DOCKER)
        docker.chmod(0o755)
        self.receipt = self.root / "receipt.json"
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        TRACE=str(self.root / "trace"), SOURCE_SHA=self.sha)

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True, timeout=30)

    def invoke(self, *args, mode=""):
        return subprocess.run(
            [str(ROOT / "scripts/build-relay-image.sh"), *args], cwd=self.repo,
            env=dict(self.env, FAKE_MODE=mode), text=True, capture_output=True, timeout=30,
        )

    def publish(self, mode=""):
        return self.invoke("--local-ref", "HEAD", "--push", "--image", "ghcr.io/example-org/relay",
                           "--receipt", str(self.receipt), mode=mode)

    def assert_cleaned_up(self):
        self.assertEqual(self.git("worktree", "list", "--porcelain").count("worktree "), 1)

    def test_success_builds_committed_snapshot_and_verifies_both_architectures(self):
        (self.repo / "Dockerfile").write_text("uncommitted change\n")
        (self.repo / "dirty-untracked").touch()
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = json.loads(self.receipt.read_text())
        self.assertEqual(receipt["source_sha"], self.sha)
        self.assertEqual(set(receipt["platforms"]), set(IMAGE.PLATFORMS))
        self.assertIn(":manual-", receipt["tag"])
        commands = [json.loads(line) for line in (self.root / "trace").read_text().splitlines()]
        build = next(c for c in commands if c[:2] == ["buildx", "build"])
        self.assertIn("BUZZ_SOURCE_SHA=" + self.sha, build)
        self.assertEqual(build[build.index("--platform") + 1], "linux/amd64,linux/arm64")
        self.assertEqual(build[build.index("--target") + 1], "runtime")
        self.assertEqual((self.repo / "Dockerfile").read_text(), "uncommitted change\n")
        self.assert_cleaned_up()

    def test_failures_do_not_create_success_receipt(self):
        for mode in ("build-failure", "missing-arm64", "wrong-sha"):
            with self.subTest(mode=mode):
                result = self.publish(mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.receipt.exists())
                self.assert_cleaned_up()

    def test_existing_receipt_is_not_mistaken_for_new_success(self):
        self.receipt.write_text("previous receipt")
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual(self.receipt.read_text(), "previous receipt")
        self.assertFalse((self.root / "trace").exists())

    def test_dry_run_resolves_local_commit_without_docker_or_registry(self):
        result = self.invoke("--local-ref", "HEAD", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["source_sha"], self.sha)
        self.assertFalse((self.root / "trace").exists())
        self.assert_cleaned_up()

    def test_invalid_ref_fails_before_build(self):
        result = self.invoke("--local-ref", "nonexistent", "--push")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "trace").exists())

    def test_remote_resolution_fetches_once_then_pins_commit(self):
        calls = []
        def fake_run(args, *unused, **kwargs):
            calls.append(args)
            return self.sha
        with patch.object(IMAGE, "run", side_effect=fake_run):
            self.assertEqual(IMAGE.source_sha(self.repo, "feature/example"), self.sha)
        private_ref = calls[0][-1].split(":", 1)[1]
        self.assertTrue(private_ref.startswith("refs/manual-image/"))
        self.assertEqual(calls, [["git", "fetch", "--no-tags", "--no-write-fetch-head", "--",
                                 "origin", "feature/example:" + private_ref],
                                ["git", "rev-parse", "--verify", private_ref + "^{commit}"],
                                ["git", "update-ref", "-d", private_ref]])

    def test_refspec_cannot_write_a_local_branch(self):
        with patch.object(IMAGE, "run") as command:
            with self.assertRaises(ValueError):
                IMAGE.source_sha(self.repo, "main:refs/heads/main")
            command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
