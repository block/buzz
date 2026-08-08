import copy
import pathlib

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


def test_unpinned_effort_is_absent_from_the_condition_hash(manifest_data):
    """Opening the effort axis must not re-identify conditions that ignore it.

    `canonical_bytes` serialises with ``exclude_none=False``, so a new optional
    field lands in the hash as an explicit null and shifts every existing
    manifest -- which would have stopped results measured before the axis
    existed from pooling with results measured after it, despite both sending
    exactly the same container environment.
    """
    manifest = ExperimentManifest.load(manifest_data)
    assert b"thinking_effort" not in manifest.canonical_bytes()


def test_pinned_effort_is_part_of_the_condition_hash(manifest_data):
    """The other half: a cell that *does* pin effort is a distinct condition.

    A2x/A3x differ from A2/A3 in this field alone, so if it were excluded
    unconditionally the two would share an identity and their trials would pool
    into one meaningless pass rate.
    """
    baseline = ExperimentManifest.load(manifest_data)
    pinned = copy.deepcopy(manifest_data)
    pinned["roster"][0].setdefault("generation", {})["thinking_effort"] = "xhigh"
    pinned_manifest = ExperimentManifest.load(pinned)
    assert b"thinking_effort" in pinned_manifest.canonical_bytes()
    assert pinned_manifest.sha256 != baseline.sha256


MANIFEST_DIR = pathlib.Path(__file__).resolve().parents[1] / "manifests"
SHIPPED = sorted(MANIFEST_DIR.glob("*.yaml"))


@pytest.mark.parametrize("path", SHIPPED, ids=lambda p: p.stem)
def test_shipped_manifest_loads(path):
    """Nothing else in the suite reads the files the sweeps actually run.

    Every other test builds a manifest from a fixture, so a typo in a shipped
    YAML -- a persona hash that moved, a price key that does not match an
    endpoint name -- surfaces only when a box has been provisioned and a sweep
    refuses to start.
    """
    assert len(ExperimentManifest.load(path).sha256) == 64


@pytest.mark.parametrize("path", SHIPPED, ids=lambda p: p.stem)
def test_shipped_manifest_pins_effort_uniformly(path):
    """Either the whole roster pins an effort or none of it does.

    A multi-agent cell that pins `thinking_effort` on the driver and forgets the
    navigator is not the condition its filename claims: it is an asymmetric pair,
    a different and far less interpretable experiment. It fails silently, because
    the unpinned seat inherits the harness default and runs perfectly well at
    `medium` while the condition name, the hash and the write-up all say `high`.
    """
    roster = ExperimentManifest.load(path).roster
    pinned = {
        entry.id: (entry.generation.thinking_effort if entry.generation else None)
        for entry in roster
    }
    levels = set(pinned.values())
    assert len(levels) == 1, (
        f"{path.stem} pins reasoning effort on some seats but not all: {pinned}"
    )


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


def test_solo_roster_is_valid(manifest_data):
    """The single-agent baseline every multi-agent condition is measured against."""
    manifest_data["roster"] = [manifest_data["roster"][0]]
    manifest = ExperimentManifest.load(manifest_data)
    assert manifest.is_solo is True


def test_multi_agent_roster_is_not_solo(manifest_data):
    assert ExperimentManifest.load(manifest_data).is_solo is False


def test_temperature_is_rejected(manifest_data):
    """buzz-agent has no temperature knob; a manifest must not imply otherwise."""
    manifest_data["roster"][0]["generation"]["temperature"] = 0.0
    with pytest.raises(ManifestError, match="Extra inputs"):
        ExperimentManifest.load(manifest_data)


def test_non_mapping_document_is_rejected(tmp_path):
    path = tmp_path / "manifest.yaml"
    path.write_text("- not\n- a\n- mapping\n")
    with pytest.raises(ManifestError, match="root must be a mapping"):
        ExperimentManifest.load(path)


def generation(manifest_data, **overrides):
    manifest_data["roster"][0]["generation"].update(overrides)
    return ExperimentManifest.load(manifest_data).roster[0].generation


def test_a_roster_entry_may_omit_generation_entirely(manifest_data):
    """Not pinning the window is a legal condition, and the shipped manifests do it.

    Every field in the block is optional, so the block itself has to be. This was
    briefly not true: the manifests dropped their `generation:` blocks while the
    schema still required one, which made every manifest in the repo unloadable.
    """
    del manifest_data["roster"][0]["generation"]
    gen = ExperimentManifest.load(manifest_data).roster[0].generation
    assert gen.max_output_tokens is None
    assert gen.context_window_tokens is None


def test_an_empty_generation_block_hashes_as_an_absent_one(manifest_data):
    """Two spellings of "pin nothing" must not be two different conditions."""
    absent = {**manifest_data}
    absent["roster"] = [{**manifest_data["roster"][0]}, manifest_data["roster"][1]]
    del absent["roster"][0]["generation"]
    empty = {**manifest_data}
    empty["roster"] = [
        {**manifest_data["roster"][0], "generation": {}},
        manifest_data["roster"][1],
    ]
    assert ExperimentManifest.load(absent).sha256 == ExperimentManifest.load(empty).sha256


def test_compaction_is_unset_by_default(manifest_data):
    """Silence means "inherit the agent's defaults", not "pin them here"."""
    gen = generation(manifest_data)
    assert gen.compact_at_tokens is None
    assert gen.compact_at_percent is None


def test_compaction_knobs_are_accepted_together(manifest_data):
    """buzz-agent fires at whichever binds first, so both may be set."""
    gen = generation(
        manifest_data,
        context_window_tokens=1_000_000,
        max_output_tokens=4096,
        compact_at_percent=30,
        compact_at_tokens=272_000,
    )
    assert gen.compact_at_percent == 30
    assert gen.compact_at_tokens == 272_000


def test_a_target_the_output_reservation_would_override_is_rejected(manifest_data):
    """buzz-agent reserves room for the response, so this could never fire."""
    with pytest.raises(ManifestError, match="can never fire"):
        generation(
            manifest_data,
            context_window_tokens=200_000,
            max_output_tokens=4096,
            compact_at_tokens=199_000,
        )


@pytest.mark.parametrize("max_output_tokens", [200_000, 10_000_000])
def test_an_output_budget_that_fills_the_window_is_rejected(
    manifest_data, max_output_tokens
):
    """The failure mode is degraded behaviour, not an error, so catch it here.

    buzz-agent subtracts max_output_tokens from the window to pick its
    compaction threshold. At or above the window that threshold is zero, so the
    agent compacts on its first turn, exhausts max_handoffs, and silently
    truncates — a run that completes and reports plausible numbers while having
    thrown away its context.
    """
    with pytest.raises(ManifestError, match="must be less than"):
        generation(
            manifest_data,
            context_window_tokens=200_000,
            max_output_tokens=max_output_tokens,
        )


def test_an_out_of_range_percentage_is_rejected(manifest_data):
    with pytest.raises(ManifestError):
        generation(manifest_data, compact_at_percent=0)


def test_platform_prompt_is_part_of_the_condition_hash(manifest_data):
    """Turning [Base] off changes the system prompt, so it changes the condition.

    The sensitivity run and the run it is compared against differ in nothing
    else, so if this field were not hashed the two would be indistinguishable
    in the results — the one comparison the check exists to make.
    """
    baseline = ExperimentManifest.load(copy.deepcopy(manifest_data))
    assert baseline.roster[0].include_platform_prompt is True
    manifest_data["roster"][0]["include_platform_prompt"] = False
    assert ExperimentManifest.load(manifest_data).sha256 != baseline.sha256


def test_compaction_target_is_part_of_the_condition_hash(manifest_data):
    import copy

    baseline = ExperimentManifest.load(copy.deepcopy(manifest_data)).sha256
    manifest_data["roster"][0]["generation"]["compact_at_tokens"] = 100_000
    assert ExperimentManifest.load(manifest_data).sha256 != baseline


def test_environment_overrides_default_to_leaving_the_task_alone(manifest_data):
    """Silence must mean "honour each task's own request", not "pick a number".

    Most of Terminal-Bench asks for 1 vCPU and 2 GB, and a manifest that says
    nothing about resources has not opted into changing that.
    """
    env = ExperimentManifest.load(manifest_data).environment
    assert env.override_cpus is None
    assert env.override_memory_mb is None
    assert env.override_storage_mb is None


def test_an_empty_environment_block_hashes_as_an_absent_one(manifest_data):
    """Two spellings of "override nothing" must not be two different conditions."""
    absent = ExperimentManifest.load(copy.deepcopy(manifest_data))
    empty = ExperimentManifest.load({**manifest_data, "environment": {}})
    assert absent.sha256 == empty.sha256


def test_container_resources_are_part_of_the_condition_hash(manifest_data):
    """A team given 8 GB and a team given 2 GB are not the same condition.

    Without this the two would be indistinguishable in the results and could
    pool into a single pass rate, which is the specific accident the resource
    override was introduced to avoid.
    """
    baseline = ExperimentManifest.load(copy.deepcopy(manifest_data)).sha256
    manifest_data["environment"] = {"override_cpus": 4, "override_memory_mb": 8192}
    assert ExperimentManifest.load(manifest_data).sha256 != baseline


@pytest.mark.parametrize(
    "block",
    [
        {"override_cpus": 0},
        {"override_memory_mb": 0},
        {"override_storage_mb": -1},
        {"override_gpus": 1},
    ],
)
def test_nonsensical_resource_overrides_are_rejected(manifest_data, block):
    """Zero CPUs is not a smaller container, and a typo must not be silently kept."""
    manifest_data["environment"] = block
    with pytest.raises(ManifestError):
        ExperimentManifest.load(manifest_data)
