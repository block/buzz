"""just benchmark must default to leaderboard-eligible settings."""

import asyncio
import importlib.util
import json
import sys
from pathlib import Path

import pytest
from harbor.models.job.config import DatasetConfig

_SCRIPT = Path(__file__).parents[2] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location("benchmark", _SCRIPT)
benchmark = importlib.util.module_from_spec(_spec)
sys.modules["benchmark"] = benchmark
_spec.loader.exec_module(benchmark)


@pytest.fixture
def state_dir(tmp_path, monkeypatch):
    monkeypatch.setattr(benchmark, "STATE_DIR", tmp_path / ".benchmark")
    return tmp_path / ".benchmark"


def test_defaults_are_leaderboard_eligible():
    args = benchmark.parse_args([])
    assert args.attempts is None
    assert args.dataset is None and args.path is None  # dataset default applied later
    (run,) = benchmark.plan_benchmark_runs(args)
    argv = benchmark.leaderboard_argv(run, Path("prov.json"), Path("linux-bin"))
    assert argv[argv.index("--dataset") + 1] == "terminal-bench/terminal-bench-2-1"
    assert argv[argv.index("--attempts") + 1] == "5"
    assert argv[argv.index("--manifest") + 1].endswith("tb-cobol-sonnet-haiku.yaml")
    assert argv[argv.index("--agent-bin-dir") + 1] == "linux-bin"
    # In-container agents reach the host relay through the forwarder gateway.
    assert argv[argv.index("--relay-gateway") + 1] == (
        f"host.docker.internal:{benchmark.RELAY_HTTP_PORT}"
    )


def test_selectors_pass_through():
    args = benchmark.parse_args(
        [
            "--path",
            "/tmp/task",
            "-i",
            "cobol*",
            "-x",
            "flaky*",
            "-k",
            "1",
            "--job-name",
            "smoke",
            "--dry-run",
        ]
    )
    (run,) = benchmark.plan_benchmark_runs(args)
    argv = benchmark.leaderboard_argv(run, Path("p.json"), Path("b"))
    assert argv[argv.index("--path") + 1] == "/tmp/task"
    assert argv[argv.index("--include-task") + 1] == "cobol*"
    assert argv[argv.index("--exclude-task") + 1] == "flaky*"
    assert argv[argv.index("--attempts") + 1] == "1"
    assert "--dry-run" in argv
    assert "--dataset" not in argv


def test_buzz_task_metadata_defines_the_expected_layers():
    tasks = benchmark.buzz_tasks_for_path(benchmark.BUZZ_DATASET_ROOT)
    assert tasks is not None
    by_layer = {
        layer: {task.path.name for task in tasks if task.layer == layer}
        for layer in benchmark.EVALUATION_LAYERS
    }
    assert by_layer == {
        "regression": {
            "reply-to-thread",
            "user-mention",
            "read-named-path-outside-workspace",
            "multiline-message",
            "memory-retrieval",
            "narrative-agent-names",
        },
        "workflow": {
            "ambiguous-user-mention",
            "cross-thread-requests",
            "create-channel-invite-users",
            "interleaved-agent-reports",
        },
    }


@pytest.mark.parametrize("layer", [None, "other"])
def test_buzz_task_metadata_rejects_missing_or_unknown_layers(
    tmp_path, monkeypatch, layer
):
    dataset = tmp_path / "buzz-dataset"
    task = dataset / "example"
    task.mkdir(parents=True)
    metadata = "" if layer is None else f'evaluation_layer = "{layer}"\n'
    (task / "task.toml").write_text(
        'schema_version = "1.3"\n\n'
        '[task]\nname = "buzz-native/example"\n\n'
        f'[metadata]\n{metadata}difficulty = "easy"\n'
    )
    monkeypatch.setattr(benchmark, "BUZZ_DATASET_ROOT", dataset)

    with pytest.raises(SystemExit, match="evaluation_layer|metadata"):
        benchmark.buzz_tasks_for_path(dataset)


@pytest.mark.parametrize(("layer", "attempts"), [("regression", 1), ("workflow", 3)])
def test_layer_selects_metadata_and_uses_its_default_attempts(layer, attempts):
    args = benchmark.parse_args(
        ["--path", str(benchmark.BUZZ_DATASET_ROOT), "--layer", layer]
    )
    (run,) = benchmark.plan_benchmark_runs(args)

    assert run.attempts == attempts
    selected = benchmark.buzz_tasks_for_path(benchmark.BUZZ_DATASET_ROOT)
    assert selected is not None
    assert set(run.include_task) == {
        task.path.name for task in selected if task.layer == layer
    }


@pytest.mark.parametrize("layer", benchmark.EVALUATION_LAYERS)
def test_layer_selectors_resolve_through_harbor_local_dataset_filter(layer):
    args = benchmark.parse_args(
        ["--path", str(benchmark.BUZZ_DATASET_ROOT), "--layer", layer]
    )
    (run,) = benchmark.plan_benchmark_runs(args)

    configs = asyncio.run(
        DatasetConfig(
            path=benchmark.BUZZ_DATASET_ROOT,
            task_names=run.include_task,
        ).get_task_configs(disable_verification=True)
    )

    assert {config.path.name for config in configs} == set(run.include_task)


def test_omitted_layer_splits_buzz_dataset_into_two_jobs():
    args = benchmark.parse_args(["--path", str(benchmark.BUZZ_DATASET_ROOT)])
    runs = benchmark.plan_benchmark_runs(args, stamp="20260825T120000Z")

    assert [run.attempts for run in runs] == [1, 3]
    assert [run.job_name.rsplit("-", 1)[-1] for run in runs] == [
        "regression",
        "workflow",
    ]
    assert set(runs[0].include_task).isdisjoint(runs[1].include_task)


def test_single_buzz_task_infers_its_layer_default():
    task_path = benchmark.BUZZ_DATASET_ROOT / "cross-thread-requests"
    args = benchmark.parse_args(["--path", str(task_path)])
    (run,) = benchmark.plan_benchmark_runs(args)

    assert run.attempts == 3
    assert run.include_task == ["cross-thread-requests"]


def test_explicit_attempts_override_keeps_one_mixed_buzz_job():
    args = benchmark.parse_args(
        ["--path", str(benchmark.BUZZ_DATASET_ROOT), "--attempts", "7"]
    )
    (run,) = benchmark.plan_benchmark_runs(args)

    assert run.attempts == 7
    assert len(run.include_task) == 10

    layered = benchmark.parse_args(
        [
            "--path",
            str(benchmark.BUZZ_DATASET_ROOT),
            "--layer",
            "workflow",
            "-k",
            "2",
        ]
    )
    (layered_run,) = benchmark.plan_benchmark_runs(layered)
    assert layered_run.attempts == 2
    assert len(layered_run.include_task) == 4


def test_invalid_layer_is_rejected():
    with pytest.raises(SystemExit):
        benchmark.parse_args(["--layer", "conformance"])

    args = benchmark.parse_args(
        ["--dataset", "terminal-bench/x", "--layer", "workflow"]
    )
    with pytest.raises(SystemExit, match="only valid"):
        benchmark.plan_benchmark_runs(args)


def test_layer_dry_run_constructs_exact_task_selectors_and_attempts():
    args = benchmark.parse_args(
        [
            "--path",
            str(benchmark.BUZZ_DATASET_ROOT),
            "--layer",
            "workflow",
            "--dry-run",
            "--job-name",
            "workflow-smoke",
        ]
    )
    (run,) = benchmark.plan_benchmark_runs(args)
    argv = benchmark.leaderboard_argv(run, Path("prov.json"), Path("linux-bin"))
    lower_args = benchmark.run_leaderboard.parse_args(argv)
    binaries = {"buzz": Path("host-bin/buzz")}
    agent_binaries = {
        name: Path("linux-bin") / name
        for name in benchmark.run_leaderboard.AGENT_BINARIES
        + (benchmark.run_leaderboard.FORWARDER_BINARY,)
    }
    command = benchmark.run_leaderboard.build_command(
        lower_args, binaries, agent_binaries
    )

    assert command[command.index("-k") + 1] == "3"
    selected = [
        command[index + 1]
        for index, part in enumerate(command)
        if part == "--include-task-name"
    ]
    assert set(selected) == set(run.include_task)
    assert set(selected) == {
        "ambiguous-user-mention",
        "cross-thread-requests",
        "create-channel-invite-users",
        "interleaved-agent-reports",
    }


def test_state_is_generated_once_and_reused(state_dir):
    first = benchmark.load_state()
    second = benchmark.load_state()
    assert first["user_secret_key"] == second["user_secret_key"]
    assert first["owner_secret_key"] != first["user_secret_key"]
    assert len(first["user_pubkey"]) == 64
    stored = json.loads((state_dir / "state.json").read_text())
    assert "user_pubkey" not in stored  # derived, never persisted


def test_provisioner_config_pins_user_and_keeps_channels(
    state_dir, tmp_path, monkeypatch
):
    monkeypatch.setenv("FAKE_KEY_ENV", "sk-test")
    endpoints = tmp_path / "endpoints.json"
    endpoints.write_text(
        json.dumps(
            {"model-a": {"provider": "anthropic", "api_key_env": "FAKE_KEY_ENV"}}
        )
    )
    state = benchmark.load_state()
    path = benchmark.write_provisioner_config(state, endpoints)
    config = json.loads(path.read_text())
    assert config["user_secret_key"] == state["user_secret_key"]
    assert config["archive_on_teardown"] is False
    assert config["llm_api_keys"] == {"model-a": "sk-test"}
    assert str(benchmark.RELAY_HTTP_PORT) in config["relay_http_url"]
    # Both views dial the relay's canonical host-bound address; inside the
    # task container the loopback forwarder bridges it to the host gateway.
    assert config["relay_ws_url"] == f"ws://localhost:{benchmark.RELAY_HTTP_PORT}"
    assert config["relay_http_url"].startswith("http://localhost:")


def test_provisioner_config_missing_api_key_is_explicit(
    state_dir, tmp_path, monkeypatch
):
    monkeypatch.delenv("MISSING_KEY_ENV", raising=False)
    endpoints = tmp_path / "endpoints.json"
    endpoints.write_text(
        json.dumps({"model-a": {"provider": "x", "api_key_env": "MISSING_KEY_ENV"}})
    )
    with pytest.raises(SystemExit, match="MISSING_KEY_ENV"):
        benchmark.write_provisioner_config(benchmark.load_state(), endpoints)


def test_env_file_wires_owner_and_ports(state_dir):
    state = benchmark.load_state()
    env_path = benchmark.write_env_file(state)
    env = dict(line.split("=", 1) for line in env_path.read_text().splitlines() if line)
    assert env["RELAY_OWNER_PUBKEY"] == state["owner_pubkey"]
    assert env["BUZZ_HTTP_PORT"] == str(benchmark.RELAY_HTTP_PORT)
    assert env["BUZZ_PG_HOST_PORT"] == str(benchmark.PG_HOST_PORT)
    assert env["BUZZ_REQUIRE_RELAY_MEMBERSHIP"] == "true"


def test_compose_command_isolates_the_project(state_dir):
    command = benchmark.compose_command("up", "-d")
    assert command[:2] == ["docker", "compose"]
    assert command[command.index("--project-name") + 1] == "buzz-benchmark"
    files = [command[i + 1] for i, part in enumerate(command) if part == "-f"]
    assert any(f.endswith("deploy/compose/compose.yml") for f in files)
    assert any(f.endswith("compose.benchmark.yml") for f in files)


def test_bring_up_self_heals_a_stale_credential_volume(monkeypatch):
    calls = []

    def fake_run(command, check=True):
        calls.append(command)
        if len(calls) == 1:  # first up fails against the stale volume
            raise benchmark.subprocess.CalledProcessError(1, command)

    monkeypatch.setattr(benchmark.subprocess, "run", fake_run)
    monkeypatch.setattr(benchmark, "stale_credential_volume", lambda state: True)
    benchmark.bring_up_stack({})
    assert [c[-3:] for c in calls] == [
        ["up", "-d", "--wait"],
        [str(benchmark.COMPOSE_FILES[-1]), "down", "-v"],
        ["up", "-d", "--wait"],
    ]


def test_bring_up_reraises_unrelated_failures(monkeypatch):
    def fake_run(command, check=True):
        raise benchmark.subprocess.CalledProcessError(1, command)

    monkeypatch.setattr(benchmark.subprocess, "run", fake_run)
    monkeypatch.setattr(benchmark, "stale_credential_volume", lambda state: False)
    with pytest.raises(benchmark.subprocess.CalledProcessError):
        benchmark.bring_up_stack({})


def test_fresh_resets_volumes_and_gui_state(tmp_path, monkeypatch):
    commands = []
    monkeypatch.setattr(
        benchmark.subprocess, "run", lambda cmd, check=True: commands.append(cmd)
    )
    monkeypatch.setattr(benchmark.sys, "platform", "darwin")
    monkeypatch.setattr(benchmark.Path, "home", classmethod(lambda cls: tmp_path))
    gui_state = tmp_path / "Library" / "WebKit" / benchmark.GUI_BUNDLE_IDENTIFIER
    gui_state.mkdir(parents=True)
    (gui_state / "localstorage.sqlite3").touch()

    benchmark.reset_environment()

    assert ["down", "-v"] == commands[0][-2:]
    assert not gui_state.exists()
    assert benchmark.parse_args(["--fresh"]).fresh
    assert not benchmark.parse_args([]).fresh
