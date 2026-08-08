import importlib.util
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location("benchmark_under_test", _SCRIPT)
benchmark = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(benchmark)


@pytest.fixture
def registries(monkeypatch):
    """Pin both registries so the suite never depends on the network."""
    monkeypatch.setattr(
        benchmark, "registry_datasets", lambda *a, **k: {"terminal-bench@2.0"}
    )
    monkeypatch.setattr(
        benchmark,
        "package_dataset_tasks",
        lambda dataset, **k: 89 if dataset == "terminal-bench/terminal-bench-2-1" else None,
    )


def test_default_dataset_resolves(registries):
    benchmark.validate_dataset(benchmark.DEFAULT_DATASET)


def test_default_dataset_is_the_package_form():
    """TB 2.1 exists only as a hub package; registry.json stops at 2.0."""
    assert benchmark.DEFAULT_DATASET == "terminal-bench/terminal-bench-2-1"


def test_package_and_registry_forms_are_both_accepted(registries):
    benchmark.validate_dataset("terminal-bench/terminal-bench-2-1")
    benchmark.validate_dataset("terminal-bench@2.0")


def test_unresolvable_package_is_rejected(registries):
    with pytest.raises(benchmark.DatasetError, match="did not resolve"):
        benchmark.validate_dataset("terminal-bench/nope-9-9")


def test_missing_version_names_both_forms(registries):
    """The error has to say which of the two shapes the operator meant."""
    with pytest.raises(benchmark.DatasetError, match="org/name") as caught:
        benchmark.validate_dataset("terminal-bench")
    assert "name@version" in str(caught.value)


def test_unpublished_registry_version_lists_what_is_available(registries):
    with pytest.raises(benchmark.DatasetError, match="terminal-bench@2.0"):
        benchmark.validate_dataset("terminal-bench@2.1")


def test_unknown_registry_dataset_says_so(registries):
    with pytest.raises(benchmark.DatasetError, match="no dataset named"):
        benchmark.validate_dataset("nope@1.0")


def test_unreachable_registry_warns_rather_than_blocking(monkeypatch, capsys):
    """Offline is a network problem for preflight to report, not a typo."""
    monkeypatch.setattr(benchmark, "registry_datasets", lambda *a, **k: None)
    benchmark.validate_dataset("anything@1.0")
    assert "unverified" in capsys.readouterr().err
