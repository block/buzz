"""Container resources are a property of the condition, not of the invocation.

Harbor enforces a task's declared resources as hard Docker limits, and the Buzz
stack runs inside that container — so a three-agent roster shares whatever the
task asked for, typically 1 vCPU and 2 GB. Raising that is necessary, but it
must be recorded: a run at 8 GB and a run at 2 GB are different conditions, and
if the level lived only in the operator's shell history the two would pool into
a single pass rate with nothing in the results to tell them apart.

So the manifest holds the value (and therefore hashes it) and this module is the
seam that hands it to Harbor's flags.
"""

import importlib.util
from pathlib import Path

import pytest
import yaml

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location(
    "benchmark_resources_under_test", _SCRIPT
)
benchmark = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(benchmark)


def write(tmp_path, block):
    manifest = tmp_path / "manifest.yaml"
    document = {"condition": "unit-test", "trial_budget": {"timeout_seconds": 36000}}
    if block is not None:
        document["environment"] = block
    manifest.write_text(yaml.safe_dump(document))
    return manifest


@pytest.mark.parametrize("block", [None, {}])
def test_a_manifest_that_pins_nothing_forwards_nothing(tmp_path, block):
    """Absent and empty both mean "let each task keep its own request".

    This is what every manifest written before the block existed looks like, so
    getting it wrong would silently re-resource the whole back catalogue.
    """
    assert benchmark.environment_override_argv(write(tmp_path, block)) == []


def test_pinned_resources_reach_harbor_as_flags(tmp_path):
    argv = benchmark.environment_override_argv(
        write(tmp_path, {"override_cpus": 4, "override_memory_mb": 8192})
    )
    assert argv == ["--override-cpus", "4", "--override-memory-mb", "8192"]


def test_only_the_keys_the_manifest_sets_are_forwarded(tmp_path):
    """storage is uniform across Terminal-Bench, so the manifests omit it.

    Emitting a flag for an unset key would restate the task's own value as
    though it were an operator decision.
    """
    argv = benchmark.environment_override_argv(
        write(tmp_path, {"override_memory_mb": 8192})
    )
    assert argv == ["--override-memory-mb", "8192"]


def test_storage_is_forwarded_when_a_manifest_does_pin_it(tmp_path):
    argv = benchmark.environment_override_argv(
        write(tmp_path, {"override_storage_mb": 20480})
    )
    assert argv == ["--override-storage-mb", "20480"]


def test_an_unreadable_manifest_defers_to_the_real_validator(tmp_path):
    """A bad manifest must fail downstream with a real message, not here.

    Same contract as trial_timeout_seconds: this helper runs before validation,
    so raising here would replace a precise pydantic error with a stack trace
    from a resource-flag helper.
    """
    manifest = tmp_path / "manifest.yaml"
    manifest.write_text("- not\n- a\n- mapping\n")
    assert benchmark.environment_override_argv(manifest) == []
    assert benchmark.environment_override_argv(tmp_path / "absent.yaml") == []


def test_the_shipped_study_manifests_all_pin_the_same_resources():
    """Every cell must be measured on identical hardware or the tiers do not compare.

    Raising the limit only for the team conditions would trade the memory-pressure
    confound for a worse one — teams and solos would then differ in two things at
    once, and the study could not say which produced the score gap.
    """
    manifests = Path(__file__).resolve().parents[1] / "manifests"
    cells = [
        "tb-solo-luna",
        "tb-solo-sol",
        "tb-solo-opus",
        "tb-peer-2luna",
        "tb-triad-3luna",
        "tb-team-3luna",
        "tb-team-opus-luna",
    ]
    pinned = {
        name: benchmark.environment_override_argv(manifests / f"{name}.yaml")
        for name in cells
    }
    # 4 and 8192 are the dataset maxima. Harbor replaces rather than raises, so
    # anything smaller would shrink the tasks that ask for the most.
    expected = ["--override-cpus", "4", "--override-memory-mb", "8192"]
    assert pinned == dict.fromkeys(cells, expected)
