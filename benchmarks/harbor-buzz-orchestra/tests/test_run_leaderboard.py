"""The leaderboard runner must emit only leaderboard-legal harbor settings."""

import importlib.util
import json
import sys
from pathlib import Path

import pytest
import yaml

_SCRIPT = Path(__file__).parent.parent / "scripts" / "run_leaderboard.py"
_spec = importlib.util.spec_from_file_location("run_leaderboard", _SCRIPT)
run_leaderboard = importlib.util.module_from_spec(_spec)
sys.modules["run_leaderboard"] = run_leaderboard
_spec.loader.exec_module(run_leaderboard)

FORBIDDEN_FLAGS = (
    "--timeout-multiplier",
    "--agent-timeout-multiplier",
    "--verifier-timeout-multiplier",
    "--agent-setup-timeout-multiplier",
    "--environment-build-timeout-multiplier",
    "--override-cpus",
    "--override-memory",
    "--override-storage",
    "--override-gpus",
)


@pytest.fixture
def binaries(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    found = {}
    for name in run_leaderboard.BINARIES:
        path = bin_dir / name
        path.write_text("#!/bin/sh\n")
        found[name] = path
    return found


@pytest.fixture
def agent_binaries(tmp_path):
    bin_dir = tmp_path / "linux-bin"
    bin_dir.mkdir()
    for name in run_leaderboard.AGENT_BINARIES:
        (bin_dir / name).write_text("ELF")
    return run_leaderboard.find_agent_binaries(bin_dir)


@pytest.fixture
def args(tmp_path, binaries, agent_binaries):
    manifest = tmp_path / "team.yaml"
    manifest.write_text(
        yaml.safe_dump(
            {
                "condition": "unit-test",
                "roster": [
                    {"id": "orch", "endpoint": "frontier"},
                    {"id": "worker", "endpoint": "fast", "count": 2},
                ],
            }
        )
    )
    endpoints = tmp_path / "endpoints.json"
    endpoints.write_text(json.dumps({"frontier": {"provider": "anthropic"}}))
    provisioner = tmp_path / "provisioner.json"
    provisioner.write_text("{}")
    return run_leaderboard.parse_args(
        [
            "--dataset",
            "terminal-bench/terminal-bench-2-1",
            "--attempts",
            "5",
            "--manifest",
            str(manifest),
            "--endpoint-config",
            str(endpoints),
            "--provisioner-config",
            str(provisioner),
            "--agent-bin-dir",
            str(next(iter(agent_binaries.values())).parent),
            "--job-name",
            "unit-test-job",
        ]
    )


def test_command_uses_standard_settings_only(args, binaries, agent_binaries):
    command = run_leaderboard.build_command(args, binaries, agent_binaries)
    assert command[:2] == ["harbor", "run"]
    assert command.count("-k") == 1
    assert command[command.index("-k") + 1] == "5"
    for flag in FORBIDDEN_FLAGS:
        assert flag not in command
    # The full production stack rides in as agent kwargs.
    kwargs = [command[i + 1] for i, p in enumerate(command) if p == "--agent-kwarg"]
    assert any(k.startswith("buzz_acp_binary=") for k in kwargs)
    assert any(k.startswith("buzz_agent_binary=") for k in kwargs)
    assert any(k.startswith("buzz_dev_mcp_binary=") for k in kwargs)


def test_agent_binaries_must_exist(tmp_path):
    with pytest.raises(SystemExit, match="buzz-dev-mcp"):
        run_leaderboard.find_agent_binaries(tmp_path)


def test_forbidden_flags_are_not_accepted(tmp_path):
    for flag in FORBIDDEN_FLAGS:
        with pytest.raises(SystemExit):
            run_leaderboard.parse_args(
                ["--dataset", "d", "--attempts", "5",
                 "--agent-bin-dir", str(tmp_path), flag, "1"]
            )


def test_metadata_template_matches_harbor_schema(args, tmp_path):
    from harbor.leaderboard.metadata import load_metadata

    path = run_leaderboard.write_metadata_template(args, tmp_path)
    loaded = load_metadata(path)
    assert loaded["agent_org_display_name"] == "Block"
    assert [m["model_name"] for m in loaded["models"]] == ["frontier", "fast"]
    assert loaded["models"][0]["model_org_display_name"] == "Anthropic"
    assert loaded["models"][1]["model_provider"] == "FILL_ME"
