#!/usr/bin/env python3
"""Unit tests for place_proof.v1 + dual_body refuse (no live home required)."""
from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import place_proof as pp  # noqa: E402


class PlaceProofTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.reg = self.root / "registry.json"
        self.units = self.root / "units"
        self.units.mkdir()
        self.reg.write_text(
            json.dumps(
                {
                    "schema": "host-agent.registry.v0",
                    "host_id": "test-host",
                    "host_role": "home",
                    "seats": [
                        {
                            "seat_id": "home-grok",
                            "pubkey": "a" * 64,
                            "display_name": "Buzz-home-grok",
                            "runtimes": ["watch"],
                            "model": "gemma3:4b",
                            "surface_root": "/home/asus/secret/path/project",
                            "expected_online": True,
                        }
                    ],
                }
            )
        )
        self.env = {
            "BUZZ_HOST_ROOT": str(self.root),
            "BUZZ_HOST_REGISTRY": str(self.reg),
            "BUZZ_HOST_ID": "test-host",
            "BUZZ_HOST_ROLE": "home",
        }

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_public_proof_redacts_surface_root_and_pid(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            seat = {
                "seat_id": "home-grok",
                "pubkey": "a" * 64,
                "surface_root": "/home/asus/secret/path",
                "runtimes": ["watch"],
            }
            unit = {"unit_name": "home-grok-watch", "unit_pid": 4242, "alive": True}
            pub = pp.public_place_proof_for_seat(seat, unit=unit)
            self.assertEqual(pub["schema"], "place_proof.v1")
            self.assertEqual(pub["birth_cert_id"], "a" * 64)
            self.assertNotIn("surface_root", pub)
            self.assertNotIn("unit_pid", pub)
            self.assertTrue(str(pub["surface_id"]).startswith("bind:"))
            self.assertEqual(pub["health"], "ok")
            raw = json.dumps(pub)
            self.assertNotIn("/home/asus/secret", raw)
            self.assertNotIn("4242", raw)

    def test_host_local_keeps_root(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            seat = {
                "seat_id": "home-grok",
                "pubkey": "a" * 64,
                "surface_root": "/home/asus/secret/path",
            }
            local = pp.host_local_place_proof_for_seat(seat)
            self.assertEqual(local["surface_root"], "/home/asus/secret/path")

    def test_dual_body_when_unit_alive(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            unit_dir = self.units / "home-grok-watch"
            unit_dir.mkdir()
            # fake alive pid: use this process
            (unit_dir / "watch.pid").write_text(str(os.getpid()))
            seat = {
                "seat_id": "home-grok",
                "pubkey": "b" * 64,
                "runtimes": ["watch"],
            }
            blocked, proof = pp.check_dual_body(seat, "home-grok")
            self.assertTrue(blocked)
            assert proof is not None
            self.assertEqual(proof["error"] if "error" in proof else None, None)
            self.assertEqual(proof["birth_cert_id"], "b" * 64)
            self.assertEqual(proof["schema"], "place_proof.v1")

    def test_no_dual_when_stopped(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            seat = {"seat_id": "home-grok", "pubkey": "c" * 64}
            blocked, proof = pp.check_dual_body(seat, "home-grok")
            self.assertFalse(blocked)
            self.assertIsNone(proof)

    def test_lease_epoch_increments(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            seat = {"seat_id": "home-grok", "pubkey": "d" * 64}
            l1 = pp.grant_lease(seat, "home-grok", body_id="home-grok-u1")
            l2 = pp.grant_lease(seat, "home-grok", body_id="home-grok-u2")
            self.assertEqual(l1["lease_epoch"], 1)
            self.assertEqual(l2["lease_epoch"], 2)
            self.assertEqual(l2["body_id"], "home-grok-u2")

    def test_bundle_public_only(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            bundle = pp.build_location_bundle()
            pub = pp.public_only(bundle)
            self.assertEqual(pub["schema"], "place_proof.v1")
            self.assertIn("bodies", pub)
            self.assertNotIn("host_local", pub)
            raw = json.dumps(pub)
            self.assertNotIn("secret/path", raw)

    def test_birth_cert_from_registry_pubkey(self) -> None:
        with mock.patch.dict(os.environ, self.env, clear=False):
            bundle = pp.build_location_bundle()
            seats = bundle.get("seats") or []
            self.assertTrue(seats)
            self.assertEqual(seats[0].get("birth_cert_id"), "a" * 64)
            self.assertEqual(seats[0].get("pubkey"), "a" * 64)


class DualBodyHttpTests(unittest.TestCase):
    """Spin host-agentd with mocked CLI and live unit → arm 409."""

    def test_arm_409_dual_body(self) -> None:
        import subprocess
        import time
        import urllib.error
        import urllib.request

        tmp = tempfile.TemporaryDirectory()
        root = Path(tmp.name)
        reg = root / "registry.json"
        units = root / "units"
        units.mkdir()
        unit_dir = units / "smoke-create-test-watch"
        unit_dir.mkdir()
        (unit_dir / "watch.pid").write_text(str(os.getpid()))
        reg.write_text(
            json.dumps(
                {
                    "host_id": "t",
                    "host_role": "home",
                    "seats": [
                        {
                            "seat_id": "smoke-create-test",
                            "pubkey": "e" * 64,
                            "runtimes": ["watch"],
                        }
                    ],
                }
            )
        )
        # fake CLI that would arm if called
        cli = root / "fake-cli"
        cli.write_text("#!/bin/bash\necho ok\n")
        cli.chmod(0o755)
        port = 18877
        token = "dual-test-token"
        env = {
            **os.environ,
            "HOST_AGENTD_TOKEN": token,
            "HOST_AGENTD_HOST": "127.0.0.1",
            "HOST_AGENTD_PORT": str(port),
            "BUZZ_HOST_ROOT": str(root),
            "BUZZ_HOST_REGISTRY": str(reg),
            "BUZZ_HOST_ROLE": "home",
            "BUZZ_HOST_ID": "t",
            "BUZZ_HOST_AGENTS": str(cli),
        }
        proc = subprocess.Popen(
            [sys.executable, str(ROOT / "host-agentd.py")],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            deadline = time.time() + 5
            while time.time() < deadline:
                try:
                    req = urllib.request.Request(
                        f"http://127.0.0.1:{port}/v1/health",
                        headers={"Authorization": f"Bearer {token}"},
                    )
                    with urllib.request.urlopen(req, timeout=1) as resp:
                        if resp.status == 200:
                            break
                except Exception:
                    time.sleep(0.1)
            else:
                self.fail("daemon did not start")

            data = json.dumps({"preset": "co-lab-watch"}).encode()
            req = urllib.request.Request(
                f"http://127.0.0.1:{port}/v1/agents/smoke-create-test/arm",
                data=data,
                headers={
                    "Authorization": f"Bearer {token}",
                    "Content-Type": "application/json",
                },
                method="POST",
            )
            try:
                with urllib.request.urlopen(req, timeout=5) as resp:
                    self.fail(f"expected 409, got {resp.status}")
            except urllib.error.HTTPError as exc:
                self.assertEqual(exc.code, 409)
                body = json.loads(exc.read().decode())
                self.assertEqual(body.get("error"), "dual_body")
                self.assertIn("place_proof", body)
                self.assertEqual(
                    body["place_proof"].get("birth_cert_id"), "e" * 64
                )
                # no secret path in response
                self.assertNotIn("surface_root", body["place_proof"])
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()
            tmp.cleanup()


if __name__ == "__main__":
    unittest.main()
