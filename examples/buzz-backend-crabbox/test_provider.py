#!/usr/bin/env python3
"""Offline unit tests for buzz-backend-crabbox (no live Crabbox required)."""

from __future__ import annotations

import importlib.machinery
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parent
PROVIDER = ROOT / "buzz-backend-crabbox"


def load_provider():
    loader = importlib.machinery.SourceFileLoader(
        "buzz_backend_crabbox", str(PROVIDER)
    )
    spec = importlib.util.spec_from_loader(loader.name, loader)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


class ProviderCliTests(unittest.TestCase):
    def run_provider(self, payload: dict) -> tuple[int, dict]:
        proc = subprocess.run(
            [sys.executable, str(PROVIDER)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertTrue(
            proc.stdout.strip(), msg=f"empty stdout; stderr={proc.stderr!r}"
        )
        data = json.loads(proc.stdout.strip().splitlines()[-1])
        return proc.returncode, data

    def test_info(self) -> None:
        code, data = self.run_provider({"op": "info", "request_id": "t1"})
        self.assertEqual(code, 0)
        self.assertTrue(data["ok"])
        self.assertEqual(data["name"], "Crabbox")
        self.assertEqual(data["version"], "0.4.0")
        self.assertIn("config_schema", data)
        self.assertIn("provider", data["config_schema"]["properties"])

    def test_info_advertises_enum_config(self) -> None:
        _, data = self.run_provider({"op": "info", "request_id": "t-enum"})
        props = data["config_schema"]["properties"]
        self.assertIn("local-container", props["provider"]["enum"])
        self.assertIn("4h", props["idle_timeout"]["enum"])

    def test_unknown_op(self) -> None:
        code, data = self.run_provider({"op": "explode", "request_id": "t2"})
        self.assertNotEqual(code, 0)
        self.assertFalse(data["ok"])
        self.assertIn("unsupported op", data["error"])

    def test_deploy_rejects_loopback_relay(self) -> None:
        code, data = self.run_provider(
            {
                "op": "deploy",
                "request_id": "t3",
                "agent": {
                    "name": "Remote",
                    "private_key_nsec": "nsec1testonlynotreal",
                    "relay_url": "ws://localhost:3000",
                    "agent_command": "buzz-agent",
                },
                "provider_config": {},
            }
        )
        self.assertNotEqual(code, 0)
        self.assertFalse(data["ok"])
        self.assertIn("loopback", data["error"].lower())
        self.assertNotIn("nsec1testonlynotreal", data["error"])

    def test_deploy_requires_private_key(self) -> None:
        code, data = self.run_provider(
            {
                "op": "deploy",
                "request_id": "t4",
                "agent": {
                    "name": "Remote",
                    "private_key_nsec": "",
                    "relay_url": "wss://relay.example.com",
                },
                "provider_config": {},
            }
        )
        self.assertNotEqual(code, 0)
        self.assertIn("private_key_nsec", data["error"])


class ProviderUnitTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.mod = load_provider()

    def test_slug_for_agent(self) -> None:
        slug = self.mod.slug_for_agent({"name": "My Cool Agent!!!"})
        self.assertTrue(slug.startswith("buzz-my-cool-agent-"))
        self.assertLessEqual(len(slug), 40)

    def test_parse_lease_from_human_warmup(self) -> None:
        stdout = (
            "leased cbx_0123456789ab slug=swift-crab provider=hetzner "
            "server=x type=y ip=1.2.3.4 idle_timeout=30m expires=...\n"
            "ready ssh=root@1.2.3.4 :2222 network=public workroot=/work/crabbox\n"
        )
        self.assertEqual(
            self.mod.parse_lease_identity(stdout, "", fallback_slug="swift-crab"),
            "cbx_0123456789ab",
        )

    def test_parse_lease_from_timing_json(self) -> None:
        stderr = json.dumps(
            {"lease": {"id": "cbx_deadbeefcafe", "slug": "warm-slug"}, "exitCode": 0}
        )
        self.assertEqual(
            self.mod.parse_lease_identity("", stderr, fallback_slug="x"),
            "cbx_deadbeefcafe",
        )

    def test_build_agent_env_merges_and_filters(self) -> None:
        env = self.mod.build_agent_env(
            {
                "private_key_nsec": "nsec1abc",
                "relay_url": "wss://relay.example.com",
                "agent_command": "/opt/bin/goose",
                "agent_args": ["--foo", "bar"],
                "system_prompt": "be helpful",
                "parallelism": 2,
                "auth_tag": "tagvalue",
                "respond_to": "owner-only",
                "respond_to_allowlist": ["aa" * 32],
                "env_vars": {
                    "ANTHROPIC_API_KEY": "sk-test",
                    "BUZZ_PRIVATE_KEY": "should-not-override",
                    "BUZZ_ACP_SETUP_PAYLOAD": "blocked",
                    "bad-key": "nope",
                    "EMPTY_SKIP": "",
                },
            },
            remote_bin="/work/buzz-agent/bin",
        )
        self.assertEqual(env["BUZZ_PRIVATE_KEY"], "nsec1abc")
        self.assertEqual(env["BUZZ_ACP_AGENT_COMMAND"], "goose")
        self.assertEqual(env["BUZZ_ACP_AGENT_ARGS"], "--foo,bar")
        self.assertEqual(env["BUZZ_ACP_AGENTS"], "2")
        self.assertEqual(env["BUZZ_AUTH_TAG"], "tagvalue")
        self.assertEqual(env["BUZZ_ACP_RESPOND_TO"], "owner-only")
        self.assertIn("BUZZ_ACP_RESPOND_TO_ALLOWLIST", env)
        self.assertEqual(env["ANTHROPIC_API_KEY"], "sk-test")
        self.assertTrue(env["PATH"].startswith("/work/buzz-agent/bin:"))
        self.assertNotIn("BUZZ_ACP_SETUP_PAYLOAD", env)
        self.assertNotIn("bad-key", env)

    def test_redact_secrets(self) -> None:
        raw = "fail nsec1abc123def456 and sk-ant-supersecretkey999"
        redacted = self.mod.redact_secrets(raw)
        self.assertNotIn("nsec1abc123def456", redacted)
        self.assertNotIn("sk-ant-supersecretkey999", redacted)
        self.assertIn("[REDACTED]", redacted)

    def test_normalize_workdir_rejects_traversal(self) -> None:
        with self.assertRaises(self.mod.ProviderError):
            self.mod.normalize_workdir("/work/../etc")
        with self.assertRaises(self.mod.ProviderError):
            self.mod.normalize_workdir("relative")
        self.assertEqual(self.mod.normalize_workdir("/work/buzz-agent/"), "/work/buzz-agent")

    def test_write_env_profile_roundtrip(self) -> None:
        path = self.mod.write_env_profile(
            {
                "private_key_nsec": "nsec1xyz",
                "relay_url": "wss://relay.example.com",
                "agent_command": "buzz-agent",
            },
            remote_bin="/work/buzz-agent/bin",
        )
        try:
            text = path.read_text(encoding="utf-8")
            self.assertIn("BUZZ_PRIVATE_KEY=nsec1xyz", text)
            self.assertIn("BUZZ_RELAY_URL=wss://relay.example.com", text)
            mode = path.stat().st_mode & 0o777
            self.assertEqual(mode, 0o600)
            keys = self.mod.read_profile_keys(path)
            self.assertIn("BUZZ_PRIVATE_KEY", keys)
        finally:
            path.unlink(missing_ok=True)

    def test_stop_and_destroy_mocked(self) -> None:
        mod = self.mod
        with (
            mock.patch.object(mod.shutil, "which", return_value="/usr/bin/crabbox"),
            mock.patch.object(mod, "claim_repo") as claim,
            mock.patch.object(mod, "run_or_raise") as run_or_raise,
            mock.patch.object(mod, "run") as run_cmd,
        ):
            # claim_repo is a contextmanager — provide a dummy Path.
            from contextlib import contextmanager
            from pathlib import Path as P

            @contextmanager
            def fake_claim():
                yield P("/tmp/fake-repo")

            claim.side_effect = fake_claim
            run_cmd.return_value = mock.Mock(
                returncode=0, stdout="stopped\n", stderr=""
            )

            code = mod.stop_remote(
                {"agent_id": "cbx_deadbeefcafe", "provider_config": {}},
                destroy_lease=False,
            )
            self.assertEqual(code, 0)
            run_or_raise.assert_called()
            run_cmd.assert_not_called()

            run_or_raise.reset_mock()
            code = mod.stop_remote(
                {"agent_id": "cbx_deadbeefcafe", "provider_config": {}},
                destroy_lease=True,
            )
            self.assertEqual(code, 0)
            run_cmd.assert_called()
            stop_argv = run_cmd.call_args[0][0]
            self.assertEqual(stop_argv[:2], ["/usr/bin/crabbox", "stop"])

    def test_stop_rejects_bad_agent_id(self) -> None:
        with self.assertRaises(self.mod.ProviderError):
            self.mod.stop_remote(
                {"agent_id": "bad;rm -rf /", "provider_config": {}},
                destroy_lease=False,
            )

    def test_deploy_happy_path_mocked(self) -> None:
        mod = self.mod
        agent = {
            "name": "Remote",
            "private_key_nsec": "nsec1abc",
            "relay_url": "wss://relay.example.com",
            "agent_command": "buzz-agent",
            "agent_args": [],
        }
        request = {
            "op": "deploy",
            "agent": agent,
            "provider_config": {"idle_timeout": "1h", "provider": "local-container"},
        }

        fake_acp = Path(tempfile.mkdtemp()) / "buzz-acp"
        fake_acp.write_text("#!/bin/sh\n", encoding="utf-8")
        fake_acp.chmod(0o755)

        from contextlib import contextmanager
        from pathlib import Path as P

        @contextmanager
        def fake_claim():
            yield P("/tmp/fake-repo")

        with (
            mock.patch.object(mod.shutil, "which") as which,
            mock.patch.object(
                mod, "resolve_toolchain", return_value={"buzz-acp": str(fake_acp)}
            ),
            mock.patch.object(mod, "warmup_lease", return_value="cbx_mocklease12"),
            mock.patch.object(mod, "stage_remote_tree") as stage,
            mock.patch.object(mod, "install_env_helper") as helper,
            mock.patch.object(mod, "start_agent") as start,
            mock.patch.object(mod, "claim_repo", side_effect=fake_claim),
        ):

            def which_side(name: str) -> str | None:
                if name == "crabbox":
                    return "/usr/local/bin/crabbox"
                return None

            which.side_effect = which_side
            code = mod.deploy(request)
            self.assertEqual(code, 0)
            stage.assert_called_once()
            helper.assert_called_once()
            start.assert_called_once()


if __name__ == "__main__":
    os.chmod(PROVIDER, 0o755)
    unittest.main()
