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

# Never accepted at all. Per-phase timeout knobs would let one phase be
# stretched without the others, which is a different experiment wearing the same
# score; --override-gpus could only add capability, since nothing in
# Terminal-Bench 2.1 requests a GPU.
REJECTED_FLAGS = (
    "--agent-timeout-multiplier",
    "--verifier-timeout-multiplier",
    "--agent-setup-timeout-multiplier",
    "--environment-build-timeout-multiplier",
    "--override-gpus",
)

# Accepted, but only on request, and each makes the job unsubmittable. A
# default run must contain none of them.
OPT_IN_FLAGS = (
    "--timeout-multiplier",
    "--override-cpus",
    "--override-memory-mb",
    "--override-storage-mb",
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
    for flag in REJECTED_FLAGS + OPT_IN_FLAGS:
        assert flag not in command, f"{flag} must be off unless asked for"
    # The full production stack rides in as agent kwargs.
    kwargs = [command[i + 1] for i, p in enumerate(command) if p == "--agent-kwarg"]
    assert any(k.startswith("buzz_acp_binary=") for k in kwargs)
    assert any(k.startswith("buzz_agent_binary=") for k in kwargs)
    assert any(k.startswith("buzz_dev_mcp_binary=") for k in kwargs)


def test_agent_binaries_must_exist(tmp_path):
    with pytest.raises(SystemExit, match="buzz-dev-mcp"):
        run_leaderboard.find_agent_binaries(tmp_path)


def required_argv(tmp_path):
    """Every required flag, so a parser error can only be about what we added.

    The previous version of this helper omitted --manifest, --endpoint-config
    and --provisioner-config, so parse_args exited over the missing required
    arguments and the assertion below passed for literally any input -- valid
    flags included. It proved nothing for as long as it existed.
    """
    for name in ("m.yaml", "e.json", "p.json"):
        (tmp_path / name).write_text("{}")
    return [
        "--dataset", "d",
        "--attempts", "5",
        "--manifest", str(tmp_path / "m.yaml"),
        "--endpoint-config", str(tmp_path / "e.json"),
        "--provisioner-config", str(tmp_path / "p.json"),
        "--agent-bin-dir", str(tmp_path),
    ]


@pytest.mark.parametrize("flag", REJECTED_FLAGS)
def test_rejected_flags_are_not_accepted(tmp_path, flag):
    with pytest.raises(SystemExit):
        run_leaderboard.parse_args(required_argv(tmp_path) + [flag, "1"])


def test_the_guard_admits_a_flag_that_is_genuinely_valid(tmp_path):
    """Guards against the failure the old test had: rejecting everything."""
    args = run_leaderboard.parse_args(required_argv(tmp_path) + ["--n-concurrent", "7"])
    assert args.n_concurrent == 7


@pytest.mark.parametrize(
    ("flag", "attr"),
    [
        ("--override-cpus", "override_cpus"),
        ("--override-memory-mb", "override_memory_mb"),
        ("--override-storage-mb", "override_storage_mb"),
    ],
)
def test_resource_overrides_are_accepted_on_request(tmp_path, flag, attr):
    args = run_leaderboard.parse_args(required_argv(tmp_path) + [flag, "8192"])
    assert getattr(args, attr) == 8192


def test_requested_resource_overrides_reach_harbor(args, binaries, agent_binaries):
    """The study pins these in the manifest; this is where they become flags."""
    args.override_cpus = 4
    args.override_memory_mb = 8192
    command = run_leaderboard.build_command(args, binaries, agent_binaries)
    assert command[command.index("--override-cpus") + 1] == "4"
    assert command[command.index("--override-memory-mb") + 1] == "8192"
    # Unset stays unset rather than defaulting to the task's own value.
    assert "--override-storage-mb" not in command


def test_metadata_template_matches_harbor_schema(args, tmp_path):
    from harbor.leaderboard.metadata import load_metadata

    path = run_leaderboard.write_metadata_template(args, tmp_path)
    loaded = load_metadata(path)
    assert loaded["agent_org_display_name"] == "Block"
    assert [m["model_name"] for m in loaded["models"]] == ["frontier", "fast"]
    assert loaded["models"][0]["model_org_display_name"] == "Anthropic"
    assert loaded["models"][1]["model_provider"] == "FILL_ME"
