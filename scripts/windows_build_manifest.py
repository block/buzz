#!/usr/bin/env python3
"""Generate and validate one bound unsigned Windows NSIS manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
from typing import Any


MANIFEST_NAME = "mybuzz-windows-build-manifest.json"
SCHEMA_NAME = "mybuzz-windows-nsis-manifest-v1"
WORKFLOW_PATH = ".github/workflows/windows-fork-integration.yml"
REPOSITORY = "Jari81/buzz"
MAX_BYTES = 2147483647
RELEASE_ID_RE = re.compile(r"[a-z0-9][a-z0-9._+-]{0,63}\Z")
GIT_SHA_RE = re.compile(r"[0-9a-f]{40}\Z")
DIGEST_RE = re.compile(r"[0-9a-f]{64}\Z")
EVENT_ID_RE = DIGEST_RE
RUN_ID_RE = re.compile(r"[1-9][0-9]{0,19}\Z")
INSTALLER_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]{0,199}\.exe\Z")
WINDOWS_RESERVED_NAMES = {
    "CON",
    "PRN",
    "AUX",
    "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}
ROOT_FIELDS = {
    "schema",
    "version",
    "release_id",
    "freeze_sha",
    "candidate_sha",
    "build_request_event_id",
    "pipeline_event_id",
    "provider",
    "digests",
    "signing",
    "artifacts",
}
PROVIDER_FIELDS = {
    "name",
    "repository",
    "workflow",
    "run_id",
    "run_attempt",
    "head_sha",
}
DIGEST_FIELDS = {
    "workflow_sha256",
    "manifest_schema_sha256",
    "generator_sha256",
}
ARTIFACT_FIELDS = {"type", "mime", "name", "path", "bytes", "sha256", "signing"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_pattern(value: Any, pattern: re.Pattern[str], label: str) -> str:
    if not isinstance(value, str) or not pattern.fullmatch(value):
        raise ValueError(f"invalid {label}")
    return value


def require_positive(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= MAX_BYTES:
        raise ValueError(f"invalid {label}")
    return value


def require_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError(f"invalid {label} fields")
    return value


def require_unsigned(value: Any, label: str) -> None:
    if value != {"status": "unsigned"}:
        raise ValueError(f"invalid {label} unsigned status")


def validate_release_bindings(
    release_id: str,
    freeze_sha: str,
    build_request_event_id: str,
    pipeline_event_id: str,
) -> None:
    require_pattern(release_id, RELEASE_ID_RE, "release ID")
    require_pattern(freeze_sha, GIT_SHA_RE, "freeze SHA")
    require_pattern(build_request_event_id, EVENT_ID_RE, "build-request event ID")
    require_pattern(pipeline_event_id, EVENT_ID_RE, "pipeline event ID")


def validate_generate_bindings(args: argparse.Namespace) -> None:
    validate_release_bindings(
        args.release_id,
        args.freeze_sha,
        args.build_request_event_id,
        args.pipeline_event_id,
    )
    if args.repository != REPOSITORY:
        raise ValueError(f"repository must be {REPOSITORY}")
    if args.workflow != WORKFLOW_PATH:
        raise ValueError("invalid workflow path")
    require_pattern(args.run_id, RUN_ID_RE, "provider run ID")
    require_positive(args.run_attempt, "provider run attempt")
    require_pattern(args.head_sha, GIT_SHA_RE, "provider head SHA")
    if args.freeze_sha != args.head_sha:
        raise ValueError("freeze SHA does not equal provider head SHA")


def installer_entries(installer_dir: Path) -> list[Path]:
    entries = sorted(path for path in installer_dir.rglob("*") if path.suffix.lower() == ".exe")
    if any(path.is_symlink() or not path.is_file() for path in entries):
        raise ValueError("NSIS installer must be a regular non-symlink file")
    if len(entries) != 1:
        raise ValueError(f"expected exactly one NSIS installer, found {len(entries)}")
    installer = entries[0]
    if (
        not INSTALLER_NAME_RE.fullmatch(installer.name)
        or installer.stem.upper() in WINDOWS_RESERVED_NAMES
    ):
        raise ValueError("unsafe NSIS installer basename")
    if not 1 <= installer.stat().st_size <= MAX_BYTES:
        raise ValueError("invalid NSIS installer byte count")
    return entries


def load_json(path: Path) -> Any:
    def object_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def reject_constant(value: str) -> None:
        raise ValueError(f"invalid JSON constant: {value}")

    with path.open("r", encoding="utf-8") as source:
        return json.load(
            source,
            object_pairs_hook=object_pairs,
            parse_constant=reject_constant,
        )


def validate_manifest(
    manifest_path: Path,
    artifact_dir: Path,
    workflow_file: Path,
    schema_file: Path,
) -> None:
    if manifest_path.parent != artifact_dir or manifest_path.name != MANIFEST_NAME:
        raise ValueError("manifest must be the artifact directory's bound manifest")
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("manifest must be a regular non-symlink file")
    manifest = require_keys(load_json(manifest_path), ROOT_FIELDS, "manifest")
    if manifest["schema"] != SCHEMA_NAME or manifest["version"] != 1:
        raise ValueError("invalid manifest schema or version")
    validate_release_bindings(
        manifest["release_id"],
        manifest["freeze_sha"],
        manifest["build_request_event_id"],
        manifest["pipeline_event_id"],
    )
    require_pattern(manifest["candidate_sha"], GIT_SHA_RE, "candidate SHA")
    if manifest["candidate_sha"] != manifest["freeze_sha"]:
        raise ValueError("candidate SHA does not equal freeze SHA")

    provider = require_keys(manifest["provider"], PROVIDER_FIELDS, "provider")
    if provider["name"] != "github-actions":
        raise ValueError("invalid provider name")
    if provider["repository"] != REPOSITORY:
        raise ValueError("invalid provider repository")
    if provider["workflow"] != WORKFLOW_PATH:
        raise ValueError("invalid provider workflow")
    require_pattern(provider["run_id"], RUN_ID_RE, "provider run ID")
    require_positive(provider["run_attempt"], "provider run attempt")
    require_pattern(provider["head_sha"], GIT_SHA_RE, "provider head SHA")
    if provider["head_sha"] != manifest["freeze_sha"]:
        raise ValueError("provider head SHA does not equal freeze SHA")

    digests = require_keys(manifest["digests"], DIGEST_FIELDS, "digest")
    for name, digest in digests.items():
        require_pattern(digest, DIGEST_RE, name.replace("_", " "))
    if digests["workflow_sha256"] != sha256(workflow_file):
        raise ValueError("workflow SHA-256 mismatch")
    if digests["manifest_schema_sha256"] != sha256(schema_file):
        raise ValueError("manifest schema SHA-256 mismatch")
    if digests["generator_sha256"] != sha256(Path(__file__)):
        raise ValueError("manifest generator SHA-256 mismatch")

    require_unsigned(manifest["signing"], "manifest")
    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, list) or len(artifacts) != 1:
        raise ValueError("manifest must contain exactly one artifact")
    artifact = require_keys(artifacts[0], ARTIFACT_FIELDS, "artifact")
    if artifact["type"] != "nsis-installer":
        raise ValueError("invalid artifact type")
    if artifact["mime"] != "application/vnd.microsoft.portable-executable":
        raise ValueError("invalid artifact MIME type")
    name = require_pattern(artifact["name"], INSTALLER_NAME_RE, "artifact name")
    path = require_pattern(artifact["path"], INSTALLER_NAME_RE, "artifact path")
    if name != path or Path(path).name != path or Path(path).is_absolute():
        raise ValueError("artifact path must equal its safe basename")
    if Path(path).stem.upper() in WINDOWS_RESERVED_NAMES:
        raise ValueError("unsafe artifact path")
    require_positive(artifact["bytes"], "artifact byte count")
    require_pattern(artifact["sha256"], DIGEST_RE, "artifact SHA-256")
    require_unsigned(artifact["signing"], "artifact")

    installer = artifact_dir / path
    if installer.is_symlink() or not installer.is_file():
        raise ValueError("artifact must be a regular non-symlink file")
    expected_entries = {MANIFEST_NAME, path}
    actual_entries = {entry.name for entry in artifact_dir.iterdir()}
    if actual_entries != expected_entries:
        raise ValueError("artifact directory must contain only installer and manifest")
    if installer.stat().st_size != artifact["bytes"]:
        raise ValueError("artifact byte count mismatch")
    if sha256(installer) != artifact["sha256"]:
        raise ValueError("artifact SHA-256 mismatch")


def generate(args: argparse.Namespace) -> None:
    validate_generate_bindings(args)
    installers = installer_entries(Path(args.installer_dir))
    workflow_file = Path(args.workflow_file)
    schema_file = Path(args.manifest_schema_file)
    workflow_digest = sha256(workflow_file)
    schema_digest = sha256(schema_file)
    if workflow_digest != args.expected_workflow_sha256:
        raise ValueError("workflow SHA-256 mismatch")
    if schema_digest != args.expected_manifest_schema_sha256:
        raise ValueError("manifest schema SHA-256 mismatch")

    output_dir = Path(args.output_dir)
    if output_dir.exists():
        raise ValueError("output directory already exists")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary_dir = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}.", dir=output_dir.parent)
    )
    try:
        source_installer = installers[0]
        final_installer = temporary_dir / source_installer.name
        shutil.copyfile(source_installer, final_installer)
        manifest = {
            "schema": SCHEMA_NAME,
            "version": 1,
            "release_id": args.release_id,
            "freeze_sha": args.freeze_sha,
            "candidate_sha": args.head_sha,
            "build_request_event_id": args.build_request_event_id,
            "pipeline_event_id": args.pipeline_event_id,
            "provider": {
                "name": "github-actions",
                "repository": args.repository,
                "workflow": args.workflow,
                "run_id": args.run_id,
                "run_attempt": args.run_attempt,
                "head_sha": args.head_sha,
            },
            "digests": {
                "workflow_sha256": workflow_digest,
                "manifest_schema_sha256": schema_digest,
                "generator_sha256": sha256(Path(__file__)),
            },
            "signing": {"status": "unsigned"},
            "artifacts": [
                {
                    "type": "nsis-installer",
                    "mime": "application/vnd.microsoft.portable-executable",
                    "name": final_installer.name,
                    "path": final_installer.name,
                    "bytes": final_installer.stat().st_size,
                    "sha256": sha256(final_installer),
                    "signing": {"status": "unsigned"},
                }
            ],
        }
        manifest_path = temporary_dir / MANIFEST_NAME
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        validate_manifest(manifest_path, temporary_dir, workflow_file, schema_file)
        os.replace(temporary_dir, output_dir)
    except BaseException:
        shutil.rmtree(temporary_dir, ignore_errors=True)
        raise
    print(output_dir / MANIFEST_NAME)


def preflight(args: argparse.Namespace) -> None:
    if not args.release_id:
        raise ValueError("release ID is required")
    validate_release_bindings(
        args.release_id,
        args.freeze_sha,
        args.build_request_event_id,
        args.pipeline_event_id,
    )
    require_pattern(args.github_sha, GIT_SHA_RE, "GitHub SHA")
    if args.freeze_sha != args.github_sha:
        raise ValueError("freeze SHA does not equal checked-out GitHub SHA")
    if args.github_repository != REPOSITORY:
        raise ValueError(f"workflow is fork-only for {REPOSITORY}")
    workflow_digest = sha256(Path(args.workflow_file))
    schema_digest = sha256(Path(args.manifest_schema_file))
    if workflow_digest != args.expected_workflow_sha256:
        raise ValueError("workflow SHA-256 mismatch")
    if schema_digest != args.expected_manifest_schema_sha256:
        raise ValueError("manifest schema SHA-256 mismatch")
    output = Path(args.github_output)
    with output.open("a", encoding="utf-8") as destination:
        destination.write(f"workflow_sha256={workflow_digest}\n")
        destination.write(f"manifest_schema_sha256={schema_digest}\n")
        destination.write(f"generator_sha256={sha256(Path(__file__))}\n")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    command = commands.add_parser("generate")
    command.add_argument("--installer-dir", required=True)
    command.add_argument("--output-dir", required=True)
    command.add_argument("--release-id", required=True)
    command.add_argument("--freeze-sha", required=True)
    command.add_argument("--build-request-event-id", required=True)
    command.add_argument("--pipeline-event-id", required=True)
    command.add_argument("--repository", required=True)
    command.add_argument("--workflow", default=WORKFLOW_PATH)
    command.add_argument("--run-id", required=True)
    command.add_argument("--run-attempt", required=True, type=int)
    command.add_argument("--head-sha", required=True)
    command.add_argument("--workflow-file", required=True)
    command.add_argument("--manifest-schema-file", required=True)
    command.add_argument("--expected-workflow-sha256", required=True)
    command.add_argument("--expected-manifest-schema-sha256", required=True)
    command.set_defaults(handler=generate)

    command = commands.add_parser("preflight")
    command.add_argument("--release-id", required=True)
    command.add_argument("--freeze-sha", required=True)
    command.add_argument("--build-request-event-id", required=True)
    command.add_argument("--pipeline-event-id", required=True)
    command.add_argument("--github-sha", required=True)
    command.add_argument("--github-repository", required=True)
    command.add_argument("--workflow-file", required=True)
    command.add_argument("--manifest-schema-file", required=True)
    command.add_argument("--expected-workflow-sha256", required=True)
    command.add_argument("--expected-manifest-schema-sha256", required=True)
    command.add_argument("--github-output", required=True)
    command.set_defaults(handler=preflight)

    command = commands.add_parser("validate")
    command.add_argument("--manifest", required=True)
    command.add_argument("--artifact-dir", required=True)
    command.add_argument("--workflow-file", required=True)
    command.add_argument("--manifest-schema-file", required=True)
    command.set_defaults(
        handler=lambda args: validate_manifest(
            Path(args.manifest),
            Path(args.artifact_dir),
            Path(args.workflow_file),
            Path(args.manifest_schema_file),
        )
    )
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        args.handler(args)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
