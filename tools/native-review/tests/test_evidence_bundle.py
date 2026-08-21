import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock

MODULE_PATH = pathlib.Path(__file__).parents[1] / "evidence_bundle.py"
SPEC = importlib.util.spec_from_file_location("evidence_bundle", MODULE_PATH)
evidence = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(evidence)


class EvidenceBundleTests(unittest.TestCase):
    def test_relay_safe_video_uses_canonical_privacy_profile(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "source.mov"
            destination = root / "finding.mp4"
            source.write_bytes(b"input")

            def completed(command, **_kwargs):
                pathlib.Path(command[-1]).write_bytes(b"canonical")
                return mock.Mock(returncode=0, stderr="")

            with mock.patch.object(evidence.shutil, "which", return_value="/usr/bin/ffmpeg"), \
                 mock.patch.object(evidence.subprocess, "run", side_effect=completed) as invoked:
                evidence.relay_safe_video(source, destination, start=1.5, duration=4.0)
            command = invoked.call_args.args[0]
            self.assertIn("-map_metadata", command)
            self.assertEqual(command[command.index("-ss") + 1], "1.5")
            self.assertEqual(command[command.index("-t") + 1], "4.0")
            self.assertIn("-map_chapters", command)
            self.assertIn("-an", command)
            self.assertIn("+faststart", command)
            self.assertIn("+bitexact", command)
            self.assertTrue(destination.is_file())

    def test_bundle_emits_provenance_hashes_and_redacted_focused_log(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "video.mp4").write_bytes(b"source")
            (root / "flutter.log").write_text(
                "before\nBUZZ_PRIVATE_KEY=deadbeef\nBad state: Cannot clone a disposed image\nafter\nunrelated\n"
            )
            receipt = {
                "status": "failed",
                "failure": "Authorization: Bearer failure-secret",
                "provenance": {"head_sha": "abc123", "dirty": False},
                "artifacts": {"video": "video.mp4", "log": "flutter.log"},
                "cleanup": {"status": "passed", "errors": [{"token": "nested-secret"}]},
                "isolation": {"secret_path": "/private/path"},
            }
            receipt_path = root / "receipt.json"
            receipt_path.write_text(json.dumps(receipt))
            output = root / "bundle"

            def finalize(_source, destination, **_kwargs):
                destination.write_bytes(b"canonical-video")
                return destination

            with mock.patch.object(evidence, "relay_safe_video", side_effect=finalize):
                manifest = evidence.finding_bundle(
                    receipt_path, output, match="disposed image", context=1
                )
            excerpt = (output / "log-excerpt.txt").read_text()
            self.assertIn("Cannot clone a disposed image", excerpt)
            self.assertIn("BUZZ_PRIVATE_KEY=[REDACTED]", excerpt)
            self.assertNotIn("deadbeef", excerpt)
            self.assertNotIn("unrelated", excerpt)
            self.assertNotIn("isolation", json.loads((output / "receipt.json").read_text()))
            bundle_receipt = json.loads((output / "receipt.json").read_text())
            self.assertNotIn("failure-secret", json.dumps(bundle_receipt))
            self.assertNotIn("nested-secret", json.dumps(bundle_receipt))
            self.assertEqual(bundle_receipt["failure"], "Authorization: [REDACTED]")
            self.assertEqual(bundle_receipt["cleanup"]["errors"][0]["token"], "[REDACTED]")
            self.assertEqual(manifest["head_sha"], "abc123")
            self.assertEqual(manifest["status"], "failed")
            self.assertEqual(manifest["cleanup"], "passed")
            self.assertEqual(set(manifest["files"]), {"finding.mp4", "receipt.json", "log-excerpt.txt"})

    def test_redacts_header_and_json_secret_forms(self):
        sources = {
            'Authorization: Bearer abc123\n{"token": "json-secret", "safe": "visible"}':
                'Authorization: [REDACTED]\n{"token": "[REDACTED]", "safe": "visible"}',
            'Authorization: token ghp_supersecret':
                'Authorization: [REDACTED]',
            'Authorization: Digest username="user", response="secret"':
                'Authorization: [REDACTED]',
            'Proxy-Authorization: Custom opaque credential with spaces':
                'Proxy-Authorization: [REDACTED]',
            '  Authorization: token ghp_indented':
                '  Authorization: [REDACTED]',
            '{"message":"Authorization: token ghp_embedded", "safe":"visible"}':
                '{"message":"Authorization: [REDACTED]',
            '{"message":"Authorization: Digest username=alice, response=secret"}':
                '{"message":"Authorization: [REDACTED]',
            '{"message":"Authorization: Negotiate opaque-secret"}':
                '{"message":"Authorization: [REDACTED]',
            'prefix Authorization: Custom opaque-secret suffix':
                'prefix Authorization: [REDACTED]',
            '{"authorization": "Bearer supersecret"}':
                '{"authorization": "[REDACTED]"}',
            "{'authorization': 'Bearer supersecret'}":
                "{'authorization': '[REDACTED]'}",
            "AUTHORIZATION=Bearer supersecret":
                "AUTHORIZATION=[REDACTED]",
            "AUTHORIZATION=Negotiate opaque-secret":
                "AUTHORIZATION=[REDACTED]",
            "PROXY_AUTHORIZATION=Custom opaque-secret":
                "PROXY_AUTHORIZATION=[REDACTED]",
            "Authorization: Negotiate first-part\n second-secret":
                "Authorization: [REDACTED]",
            "Proxy-Authorization: Custom first-part\r\n\tsecond-secret":
                "Proxy-Authorization: [REDACTED]",
            "Cookie: session=secret; refresh=supersecret":
                "Cookie: [REDACTED]",
            "COOKIE=session-secret refresh-secret":
                "COOKIE=[REDACTED]",
            "PASSWORD=correct horse battery staple":
                "PASSWORD=[REDACTED]",
            "token=alpha beta gamma":
                "token=[REDACTED]",
            "X-Api-Key: alpha beta":
                "X-Api-Key: [REDACTED]",
            "api_key=alpha; beta gamma":
                "api_key=[REDACTED]",
            "password=first-part\n second-part\nnext=visible":
                "password=[REDACTED]\nnext=visible",
            'prefix token=alpha beta; gamma':
                'prefix token=[REDACTED]',
            '{"token": "prefix\\\"tail-secret", "safe": "visible"}':
                '{"token": "[REDACTED]", "safe": "visible"}',
            '{"api_key": "prefix\\\\\\\"tail-secret", "safe": "visible"}':
                '{"api_key": "[REDACTED]", "safe": "visible"}',
            "{'password': 'prefix\\'tail-secret', 'safe': 'visible'}":
                "{'password': '[REDACTED]', 'safe': 'visible'}",
            'token="prefix\\\"tail-secret"\nnext=visible':
                'token="[REDACTED]"\nnext=visible',
        }
        for source, expected in sources.items():
            with self.subTest(source=source):
                self.assertEqual(evidence.redact_log(source), expected)

    def test_non_finite_clip_bounds_fail_before_ffmpeg(self):
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "source.mp4"
            source.write_bytes(b"video")
            for start, duration in ((float("nan"), None), (float("inf"), None), (None, float("nan"))):
                with self.subTest(start=start, duration=duration), \
                     mock.patch.object(evidence.shutil, "which") as which:
                    with self.assertRaisesRegex(evidence.EvidenceError, "finite"):
                        evidence.relay_safe_video(source, source.with_name("out.mp4"), start=start, duration=duration)
                    which.assert_not_called()

    def test_bundle_refuses_missing_match_and_removes_partial_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "video.mp4").write_bytes(b"source")
            (root / "flutter.log").write_text("ordinary output\n")
            receipt_path = root / "receipt.json"
            receipt_path.write_text(json.dumps({
                "provenance": {}, "artifacts": {"video": "video.mp4", "log": "flutter.log"}
            }))
            output = root / "bundle"
            with mock.patch.object(evidence, "relay_safe_video", side_effect=lambda _s, d, **_k: d.write_bytes(b"v") or d):
                with self.assertRaisesRegex(evidence.EvidenceError, "found no lines"):
                    evidence.finding_bundle(receipt_path, output, match="missing")
            self.assertFalse(output.exists())

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "bundle"
            output.mkdir()
            with self.assertRaisesRegex(evidence.EvidenceError, "already exists"):
                evidence.finding_bundle(pathlib.Path(directory) / "receipt.json", output)
