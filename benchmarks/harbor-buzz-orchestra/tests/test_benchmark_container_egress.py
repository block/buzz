"""The preflight that checks egress from inside a container, not just the host.

The TLS preflight next door proves the *host* can reach PyPI unintercepted.
That is a different path from the one every verifier depends on: the docker
proxy, the image's trust store, apt. On the A1 sweep the host was fine and
containers were not, and three trials recorded reward 0 because their verifier
could not `apt-get install curl` to fetch the uv installer.
"""

import importlib.util
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location("benchmark_egress_under_test", _SCRIPT)
benchmark = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(benchmark)


def probe(monkeypatch, value):
    monkeypatch.setattr(benchmark, "container_scoring_probe", lambda *a, **k: value)


def test_a_container_that_can_install_curl_lets_the_run_proceed(monkeypatch, capsys):
    probe(monkeypatch, ("seeded", "installed"))
    benchmark.check_container_can_be_scored()  # does not raise
    out = capsys.readouterr().out
    # The operator should see that the seed was needed, not just that it passed.
    assert "seeded" in out and "installed" in out


def test_an_image_that_already_ships_curl_is_fine_too(monkeypatch):
    probe(monkeypatch, ("present", "present"))
    benchmark.check_container_can_be_scored()


def test_a_container_that_cannot_install_curl_stops_the_run(monkeypatch):
    """81 of 89 verifiers need curl; without it the sweep scores zeros, not answers."""
    probe(monkeypatch, ("failed", "unavailable"))
    with pytest.raises(benchmark.ContainerEgressError) as caught:
        benchmark.check_container_can_be_scored()
    message = str(caught.value)
    # What broke, how bad it is, and what to try next.
    assert "curl" in message
    assert "81 of 89" in message
    assert "docker run" in message


def test_an_unrunnable_probe_does_not_block_the_run(monkeypatch, capsys):
    """No docker, no image, a pull that timed out: unknown is not broken.

    Same rule as the TLS preflight — a check that cannot see must not stand in
    the way of a run that would have worked.
    """
    probe(monkeypatch, None)
    benchmark.check_container_can_be_scored()
    assert "skipping check" in capsys.readouterr().out


def test_the_probe_runs_the_trials_own_setup_commands(monkeypatch):
    """Not an approximation of them.

    A preflight that proved something subtly different from what the trials do
    would be worse than none: it would read as a clean bill of health while the
    real path stayed broken. So it executes the same two shells the container
    runtime executes, and mounts the CA bundle where the runtime puts it.
    """
    seen = {}

    class Completed:
        stdout = "seeded\ninstalled\n"
        stderr = ""

    def fake_run(argv, **kwargs):
        seen["argv"] = argv
        return Completed()

    monkeypatch.setattr(benchmark.subprocess, "run", fake_run)
    assert benchmark.container_scoring_probe() == ("seeded", "installed")

    argv = seen["argv"]
    assert argv[0] == "docker"
    assert benchmark.CANARY_IMAGE in argv
    script = argv[-1]
    assert benchmark.seed_trust_store_command() in script
    assert benchmark.install_verifier_deps_command() in script
    # Mounted at the path the runtime uploads to, or the seed command's `cp`
    # source would not exist and the probe would test nothing.
    assert any(
        arg.endswith(f":{benchmark.REMOTE_CA_BUNDLE}:ro")
        for arg in argv
    )


def test_a_probe_that_prints_nothing_usable_is_treated_as_unknown(monkeypatch):
    """A truncated or empty result must not be read as a pass."""

    class Completed:
        stdout = "seeded\n"
        stderr = ""

    monkeypatch.setattr(benchmark.subprocess, "run", lambda *a, **k: Completed())
    assert benchmark.container_scoring_probe() is None
