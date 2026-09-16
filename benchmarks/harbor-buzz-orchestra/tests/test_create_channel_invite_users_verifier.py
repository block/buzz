import copy
import importlib.util
from pathlib import Path

from harbor_buzz_orchestra.task_fixtures import TARGET_BOTS, TARGET_USERS, fixture_for

# The Buzz-native tasks are a sibling dataset of this harness package.
DATASET_ROOT = Path(__file__).resolve().parents[2] / "buzz-dataset"
VERIFIER = DATASET_ROOT / "create-channel-invite-users" / "tests" / "verify.py"
SPEC = importlib.util.spec_from_file_location("create_channel_verifier", VERIFIER)
verifier = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(verifier)

ORCHESTRATOR = "a" * 64


def _evidence() -> dict:
    fixture = fixture_for("create-channel-invite-users")
    directory = [
        {"name": entry.name, "role": entry.role, "pubkey": f"{index:064x}"}
        for index, entry in enumerate(fixture.directory, start=1)
    ]
    by_name = {entry["name"]: entry for entry in directory}
    members = [{"pubkey": ORCHESTRATOR, "role": "owner"}]
    members += [
        {"pubkey": by_name[name]["pubkey"], "role": "member"} for name in TARGET_USERS
    ]
    members += [
        {"pubkey": by_name[name]["pubkey"], "role": "bot"} for name in TARGET_BOTS
    ]
    return {
        "schema_version": 1,
        "task_name": "create-channel-invite-users",
        "identities": {"solo-1": {"role": "orchestrator", "pubkey": ORCHESTRATOR}},
        "directory": directory,
        "observed_channels": [
            {
                "channel_id": "channel-1",
                "name": "fix-pr-1234",
                "channel_type": "stream",
                "visibility": "private",
                "archived": False,
                "ttl_seconds": 3600,
                "members": members,
            }
        ],
    }


def test_exact_temporary_channel_and_roster_passes():
    metrics, details = verifier.score_evidence(_evidence())

    assert all(value == 1.0 for value in metrics.values())
    assert details["channel_id"] == "channel-1"


def test_extra_member_fails_exact_membership():
    evidence = _evidence()
    evidence["observed_channels"][0]["members"].append(
        {"pubkey": "f" * 64, "role": "member"}
    )

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["channel_created"] == 1.0
    assert metrics["exact_membership"] == 0.0
    assert metrics["reward"] == 0.0


def test_wrong_bot_role_fails_roles():
    evidence = _evidence()
    bot_pubkey = next(
        row["pubkey"] for row in evidence["directory"] if row["name"] == TARGET_BOTS[0]
    )
    member = next(
        row
        for row in evidence["observed_channels"][0]["members"]
        if row["pubkey"] == bot_pubkey
    )
    member["role"] = "member"

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["exact_membership"] == 1.0
    assert metrics["expected_roles"] == 0.0
    assert metrics["reward"] == 0.0


def test_permanent_channel_fails_temporary_requirement():
    evidence = _evidence()
    evidence["observed_channels"][0]["ttl_seconds"] = None

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["temporary_channel"] == 0.0
    assert metrics["reward"] == 0.0


def test_duplicate_exact_name_fails_channel_creation():
    evidence = _evidence()
    evidence["observed_channels"].append(
        copy.deepcopy(evidence["observed_channels"][0])
    )

    metrics, details = verifier.score_evidence(evidence)

    assert details["matching_channel_count"] == 2
    assert metrics["channel_created"] == 0.0
    assert metrics["reward"] == 0.0


def test_missing_evidence_fails_closed():
    metrics, details = verifier.score_evidence(None)

    assert all(value == 0.0 for value in metrics.values())
    assert "error" in details
