import copy

import pytest
import yaml

from harbor_buzz_orchestra import ExperimentManifest, ManifestError


def test_hash_is_independent_of_mapping_and_yaml_key_order(tmp_path, manifest_data):
    first = ExperimentManifest.load(manifest_data)
    path = tmp_path / "manifest.yaml"
    path.write_text(
        yaml.safe_dump(
            dict(reversed(list(copy.deepcopy(manifest_data).items()))), sort_keys=False
        )
    )
    second = ExperimentManifest.load(path)
    assert first.canonical_bytes() == second.canonical_bytes()
    assert first.sha256 == second.sha256
    assert len(first.sha256) == 64


def test_hash_changes_when_staffing_changes(manifest_data):
    first = ExperimentManifest.load(manifest_data)
    changed = copy.deepcopy(manifest_data)
    changed["roster"][1]["count"] = 3
    assert ExperimentManifest.load(changed).sha256 != first.sha256


@pytest.mark.parametrize(
    ("mutation", "match"),
    [
        (lambda data: data.update({"unknown": True}), "Extra inputs"),
        (lambda data: data["roster"].pop(0), "exactly one orchestrator"),
        (lambda data: data["prices"].pop("databricks/qwen"), "prices missing"),
        (lambda data: data["roster"][1].update({"concurrency": 5}), "concurrency"),
    ],
)
def test_invalid_manifest_is_rejected(manifest_data, mutation, match):
    mutation(manifest_data)
    with pytest.raises(ManifestError, match=match):
        ExperimentManifest.load(manifest_data)


def test_non_mapping_document_is_rejected(tmp_path):
    path = tmp_path / "manifest.yaml"
    path.write_text("- not\n- a\n- mapping\n")
    with pytest.raises(ManifestError, match="root must be a mapping"):
        ExperimentManifest.load(path)


def test_thinking_effort_is_pinnable_and_validated(manifest_data):
    pinned = copy.deepcopy(manifest_data)
    pinned["roster"][0]["generation"]["thinking_effort"] = "medium"

    manifest = ExperimentManifest.load(pinned)

    assert manifest.roster[0].generation.thinking_effort == "medium"
    assert manifest.roster[1].generation.thinking_effort is None

    bogus = copy.deepcopy(manifest_data)
    bogus["roster"][0]["generation"]["thinking_effort"] = "medium-high"
    with pytest.raises(ManifestError):
        ExperimentManifest.load(bogus)


def test_unpinned_thinking_effort_does_not_change_the_condition_hash(manifest_data):
    # A manifest written before the effort axis existed sends a byte-identical
    # container environment, so opening the axis must not re-identify it.
    baseline = ExperimentManifest.load(manifest_data)
    explicit_null = copy.deepcopy(manifest_data)
    explicit_null["roster"][0]["generation"]["thinking_effort"] = None

    assert ExperimentManifest.load(explicit_null).sha256 == baseline.sha256

    pinned = copy.deepcopy(manifest_data)
    pinned["roster"][0]["generation"]["thinking_effort"] = "high"
    assert ExperimentManifest.load(pinned).sha256 != baseline.sha256
