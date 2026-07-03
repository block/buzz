import hashlib
import json
from pathlib import Path

import pytest

from harbor_buzz_orchestra.prepare_verifier import prepare_task


def _write_fixture(root: Path, lock_dir: Path) -> None:
    (root / "environment").mkdir(parents=True)
    (root / "tests").mkdir()
    (root / "task.toml").write_text(
        "[verifier]\ntimeout_sec = 120.0\n\n[agent]\n\n[verifier.env]\n"
    )
    (root / "environment/Dockerfile").write_text("FROM ubuntu:24.04\nWORKDIR /app")
    (root / "tests/test.sh").write_bytes(b"#!/bin/bash\necho unchanged\n")
    (lock_dir / "wheels").mkdir(parents=True)
    wheel = lock_dir / "wheels/example-1-py3-none-any.whl"
    wheel.write_bytes(b"wheel")
    digest = hashlib.sha256(b"wheel").hexdigest()
    (lock_dir / "requirements.lock").write_text(f"example==1 --hash=sha256:{digest}\n")
    (lock_dir / "lock.json").write_text(
        json.dumps(
            {
                "task": "test/example",
                "base_image": "ubuntu:24.04@sha256:abc",
                "python_packages": [{"file": wheel.name, "sha256": digest}],
            }
        )
    )


def test_prepare_is_deterministic_and_preserves_tests(tmp_path: Path) -> None:
    source, lock = tmp_path / "source", tmp_path / "lock"
    _write_fixture(source, lock)
    before = (source / "tests/test.sh").read_bytes()
    first = prepare_task(source, tmp_path / "one", lock)
    second = prepare_task(source, tmp_path / "two", lock)
    assert first == second
    assert (tmp_path / "one/tests/test.sh").read_bytes() == before
    assert 'network_mode = "no-network"' in (tmp_path / "one/task.toml").read_text()
    assert first["prepared_image"] is True
    assert first["verifier_network_enforcement"] == "enforced"
    assert first["offline_attestation_required"] is False


def test_prepare_advisory_mode_is_explicit_and_omits_provider_policy(
    tmp_path: Path,
) -> None:
    source, lock = tmp_path / "source", tmp_path / "lock"
    _write_fixture(source, lock)
    metadata = prepare_task(
        source, tmp_path / "out", lock, network_enforcement="advisory"
    )
    task = (tmp_path / "out/task.toml").read_text()
    assert "network_mode" not in task
    assert 'UV_OFFLINE = "1"' in task
    assert metadata["verifier_network_mode"] is None
    assert metadata["verifier_network_enforcement"] == "advisory"
    assert metadata["offline_attestation_required"] is True


def test_prepare_rejects_unknown_network_enforcement(tmp_path: Path) -> None:
    source, lock = tmp_path / "source", tmp_path / "lock"
    _write_fixture(source, lock)
    with pytest.raises(ValueError, match="must be 'enforced' or 'advisory'"):
        prepare_task(source, tmp_path / "out", lock, network_enforcement="silent")


def test_prepare_rejects_tampered_wheel(tmp_path: Path) -> None:
    source, lock = tmp_path / "source", tmp_path / "lock"
    _write_fixture(source, lock)
    (lock / "wheels/example-1-py3-none-any.whl").write_bytes(b"tampered")
    with pytest.raises(ValueError, match="wheel hash mismatch"):
        prepare_task(source, tmp_path / "out", lock)
