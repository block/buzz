#!/usr/bin/env python3
"""Focused tests for the unsigned Windows NSIS build manifest."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOL = REPO_ROOT / "scripts" / "windows_build_manifest.py"
SCHEMA = REPO_ROOT / "docs" / "nips" / "BW_WINDOWS_NSIS_MANIFEST_V1.schema.json"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "windows-fork-integration.yml"
FREEZE_SHA = "a" * 40
BUILD_REQUEST_ID = "b" * 64
PIPELINE_ID = "c" * 64


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def preflight_command(github_output: Path) -> list[str]:
    return [
        sys.executable,
        str(TOOL),
        "preflight",
        "--release-id",
        "mybuzz-1.2.3+42",
        "--freeze-sha",
        FREEZE_SHA,
        "--build-request-event-id",
        BUILD_REQUEST_ID,
        "--pipeline-event-id",
        PIPELINE_ID,
        "--github-sha",
        FREEZE_SHA,
        "--github-repository",
        "Jari81/buzz",
        "--workflow-file",
        str(WORKFLOW),
        "--manifest-schema-file",
        str(SCHEMA),
        "--expected-workflow-sha256",
        sha256(WORKFLOW),
        "--expected-manifest-schema-sha256",
        sha256(SCHEMA),
        "--github-output",
        str(github_output),
    ]


def generate_command(installer_dir: Path, artifact_dir: Path) -> list[str]:
    return [
        sys.executable,
        str(TOOL),
        "generate",
        "--installer-dir",
        str(installer_dir),
        "--output-dir",
        str(artifact_dir),
        "--release-id",
        "mybuzz-1.2.3+42",
        "--freeze-sha",
        FREEZE_SHA,
        "--build-request-event-id",
        BUILD_REQUEST_ID,
        "--pipeline-event-id",
        PIPELINE_ID,
        "--repository",
        "Jari81/buzz",
        "--workflow",
        ".github/workflows/windows-fork-integration.yml",
        "--run-id",
        "34277458261",
        "--run-attempt",
        "2",
        "--head-sha",
        FREEZE_SHA,
        "--workflow-file",
        str(WORKFLOW),
        "--manifest-schema-file",
        str(SCHEMA),
        "--expected-workflow-sha256",
        sha256(WORKFLOW),
        "--expected-manifest-schema-sha256",
        sha256(SCHEMA),
    ]


class WindowsBuildManifestTests(unittest.TestCase):
    def test_generate_copies_one_installer_and_emits_bound_unsigned_manifest(self) -> None:
        installer_bytes = b"synthetic-nsis\x00final-bytes\xff"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            installer_dir = root / "nsis"
            artifact_dir = root / "artifact"
            installer_dir.mkdir()
            installer = installer_dir / "Buzz_1.2.3-fork.42_x64-setup.exe"
            installer.write_bytes(installer_bytes)

            result = subprocess.run(
                [
                    sys.executable,
                    str(TOOL),
                    "generate",
                    "--installer-dir",
                    str(installer_dir),
                    "--output-dir",
                    str(artifact_dir),
                    "--release-id",
                    "mybuzz-1.2.3+42",
                    "--freeze-sha",
                    FREEZE_SHA,
                    "--build-request-event-id",
                    BUILD_REQUEST_ID,
                    "--pipeline-event-id",
                    PIPELINE_ID,
                    "--repository",
                    "Jari81/buzz",
                    "--workflow",
                    ".github/workflows/windows-fork-integration.yml",
                    "--run-id",
                    "34277458261",
                    "--run-attempt",
                    "2",
                    "--head-sha",
                    FREEZE_SHA,
                    "--workflow-file",
                    str(WORKFLOW),
                    "--manifest-schema-file",
                    str(SCHEMA),
                    "--expected-workflow-sha256",
                    sha256(WORKFLOW),
                    "--expected-manifest-schema-sha256",
                    sha256(SCHEMA) if SCHEMA.exists() else "0" * 64,
                ],
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads(
                (artifact_dir / "mybuzz-windows-build-manifest.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(manifest["schema"], "mybuzz-windows-nsis-manifest-v1")
            self.assertEqual(manifest["version"], 1)
            self.assertEqual(manifest["release_id"], "mybuzz-1.2.3+42")
            self.assertEqual(manifest["freeze_sha"], FREEZE_SHA)
            self.assertEqual(manifest["candidate_sha"], FREEZE_SHA)
            self.assertEqual(manifest["build_request_event_id"], BUILD_REQUEST_ID)
            self.assertEqual(manifest["pipeline_event_id"], PIPELINE_ID)
            self.assertEqual(
                manifest["provider"],
                {
                    "name": "github-actions",
                    "repository": "Jari81/buzz",
                    "workflow": ".github/workflows/windows-fork-integration.yml",
                    "run_id": "34277458261",
                    "run_attempt": 2,
                    "head_sha": FREEZE_SHA,
                },
            )
            self.assertEqual(manifest["signing"], {"status": "unsigned"})
            self.assertEqual(len(manifest["artifacts"]), 1)
            artifact = manifest["artifacts"][0]
            self.assertEqual(artifact["type"], "nsis-installer")
            self.assertEqual(
                artifact["mime"], "application/vnd.microsoft.portable-executable"
            )
            self.assertEqual(artifact["name"], installer.name)
            self.assertEqual(artifact["path"], installer.name)
            self.assertEqual(artifact["bytes"], len(installer_bytes))
            self.assertEqual(artifact["sha256"], hashlib.sha256(installer_bytes).hexdigest())
            self.assertEqual(artifact["signing"], {"status": "unsigned"})
            self.assertEqual((artifact_dir / installer.name).read_bytes(), installer_bytes)
            self.assertEqual(manifest["digests"]["workflow_sha256"], sha256(WORKFLOW))
            self.assertEqual(
                manifest["digests"]["manifest_schema_sha256"], sha256(SCHEMA)
            )
            self.assertEqual(manifest["digests"]["generator_sha256"], sha256(TOOL))


    def test_preflight_accepts_bound_dispatch_and_exports_digests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            github_output = Path(tmp) / "github-output"
            result = subprocess.run(
                [
                    sys.executable,
                    str(TOOL),
                    "preflight",
                    "--release-id",
                    "mybuzz-1.2.3+42",
                    "--freeze-sha",
                    FREEZE_SHA,
                    "--build-request-event-id",
                    BUILD_REQUEST_ID,
                    "--pipeline-event-id",
                    PIPELINE_ID,
                    "--github-sha",
                    FREEZE_SHA,
                    "--github-repository",
                    "Jari81/buzz",
                    "--workflow-file",
                    str(WORKFLOW),
                    "--manifest-schema-file",
                    str(SCHEMA),
                    "--expected-workflow-sha256",
                    sha256(WORKFLOW),
                    "--expected-manifest-schema-sha256",
                    sha256(SCHEMA),
                    "--github-output",
                    str(github_output),
                ],
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                github_output.read_text(encoding="utf-8").splitlines(),
                [
                    f"workflow_sha256={sha256(WORKFLOW)}",
                    f"manifest_schema_sha256={sha256(SCHEMA)}",
                    f"generator_sha256={sha256(TOOL)}",
                ],
            )


    def test_preflight_rejects_empty_required_dispatch_input(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            command = preflight_command(Path(tmp) / "github-output")
            command[command.index("--release-id") + 1] = ""
            result = subprocess.run(
                command, text=True, capture_output=True, check=False
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("release ID is required", result.stderr)


    def test_preflight_rejects_malformed_release_event_ids_and_git_shas(self) -> None:
        cases = [
            ("--release-id", "MyBuzz bad", "invalid release ID"),
            ("--freeze-sha", "A" * 40, "invalid freeze SHA"),
            (
                "--build-request-event-id",
                "g" * 64,
                "invalid build-request event ID",
            ),
            ("--pipeline-event-id", "c" * 63, "invalid pipeline event ID"),
            ("--github-sha", "0" * 39, "invalid GitHub SHA"),
        ]
        for option, value, error in cases:
            with self.subTest(option=option), tempfile.TemporaryDirectory() as tmp:
                command = preflight_command(Path(tmp) / "github-output")
                command[command.index(option) + 1] = value
                result = subprocess.run(
                    command, text=True, capture_output=True, check=False
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(error, result.stderr)


    def test_preflight_rejects_freeze_sha_that_differs_from_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            command = preflight_command(Path(tmp) / "github-output")
            command[command.index("--github-sha") + 1] = "d" * 40
            result = subprocess.run(
                command, text=True, capture_output=True, check=False
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("freeze SHA does not equal checked-out GitHub SHA", result.stderr)


    def test_preflight_rejects_non_fork_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            command = preflight_command(Path(tmp) / "github-output")
            command[command.index("--github-repository") + 1] = "block/buzz"
            result = subprocess.run(
                command, text=True, capture_output=True, check=False
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("workflow is fork-only for Jari81/buzz", result.stderr)


    def test_preflight_rejects_workflow_or_schema_digest_mismatch(self) -> None:
        cases = [
            (
                "--expected-workflow-sha256",
                "0" * 64,
                "workflow SHA-256 mismatch",
            ),
            (
                "--expected-manifest-schema-sha256",
                "0" * 64,
                "manifest schema SHA-256 mismatch",
            ),
        ]
        for option, value, error in cases:
            with self.subTest(option=option), tempfile.TemporaryDirectory() as tmp:
                command = preflight_command(Path(tmp) / "github-output")
                command[command.index(option) + 1] = value
                result = subprocess.run(
                    command, text=True, capture_output=True, check=False
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(error, result.stderr)


    def test_generate_rejects_zero_or_multiple_nsis_installers(self) -> None:
        for count in (0, 2):
            with self.subTest(count=count), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                installer_dir = root / "nsis"
                installer_dir.mkdir()
                for index in range(count):
                    (installer_dir / f"Buzz_{index}_x64-setup.exe").write_bytes(
                        f"installer-{index}".encode()
                    )
                result = subprocess.run(
                    generate_command(installer_dir, root / "artifact"),
                    text=True,
                    capture_output=True,
                    check=False,
                )

                self.assertNotEqual(result.returncode, 0)
                self.assertIn(
                    f"expected exactly one NSIS installer, found {count}", result.stderr
                )


    def test_generate_rejects_untrusted_cli_bindings_without_partial_output(self) -> None:
        cases = [
            ("--release-id", "Bad release", "invalid release ID"),
            ("--freeze-sha", "A" * 40, "invalid freeze SHA"),
            (
                "--build-request-event-id",
                "g" * 64,
                "invalid build-request event ID",
            ),
            ("--pipeline-event-id", "c" * 63, "invalid pipeline event ID"),
            ("--repository", "block/buzz", "repository must be Jari81/buzz"),
            ("--workflow", "other.yml", "invalid workflow path"),
            ("--run-id", "0", "invalid provider run ID"),
            ("--run-attempt", "0", "invalid provider run attempt"),
            ("--head-sha", "A" * 40, "invalid provider head SHA"),
            ("--head-sha", "d" * 40, "freeze SHA does not equal provider head SHA"),
            (
                "--expected-workflow-sha256",
                "0" * 64,
                "workflow SHA-256 mismatch",
            ),
            (
                "--expected-manifest-schema-sha256",
                "0" * 64,
                "manifest schema SHA-256 mismatch",
            ),
        ]
        for option, value, error in cases:
            with self.subTest(option=option), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                installer_dir = root / "nsis"
                output_dir = root / "artifact"
                installer_dir.mkdir()
                (installer_dir / "Buzz_1.2.3_x64-setup.exe").write_bytes(b"bytes")
                command = generate_command(installer_dir, output_dir)
                command[command.index(option) + 1] = value
                result = subprocess.run(
                    command, text=True, capture_output=True, check=False
                )

                self.assertNotEqual(result.returncode, 0)
                self.assertIn(error, result.stderr)
                self.assertFalse(output_dir.exists())


    def test_generate_rejects_symlink_or_unsafe_installer_before_output(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            installer_dir = root / "nsis"
            output_dir = root / "artifact"
            installer_dir.mkdir()
            target = root / "outside.exe"
            target.write_bytes(b"outside")
            (installer_dir / "Buzz_link.exe").symlink_to(target)
            result = subprocess.run(
                generate_command(installer_dir, output_dir),
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("regular non-symlink", result.stderr)
            self.assertFalse(output_dir.exists())

        for unsafe_name in ("Buzz unsafe.exe", "CON.exe"):
            with self.subTest(name=unsafe_name), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                installer_dir = root / "nsis"
                output_dir = root / "artifact"
                installer_dir.mkdir()
                (installer_dir / unsafe_name).write_bytes(b"bytes")
                result = subprocess.run(
                    generate_command(installer_dir, output_dir),
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unsafe NSIS installer basename", result.stderr)
                self.assertFalse(output_dir.exists())


    def test_manifest_schema_is_closed_and_defines_every_bound_field(self) -> None:
        schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
        root_fields = {
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
        self.assertFalse(schema["additionalProperties"])
        self.assertEqual(set(schema["required"]), root_fields)
        self.assertEqual(set(schema["properties"]), root_fields)
        self.assertEqual(
            schema["properties"]["schema"]["const"],
            "mybuzz-windows-nsis-manifest-v1",
        )
        self.assertEqual(schema["properties"]["version"]["const"], 1)
        self.assertEqual(
            schema["properties"]["release_id"]["pattern"],
            "^[a-z0-9][a-z0-9._+-]{0,63}$",
        )
        for field in (
            "build_request_event_id",
            "pipeline_event_id",
        ):
            self.assertEqual(
                schema["properties"][field]["pattern"], "^[0-9a-f]{64}$"
            )
        for field in ("freeze_sha", "candidate_sha"):
            self.assertEqual(
                schema["properties"][field]["pattern"], "^[0-9a-f]{40}$"
            )

        provider = schema["properties"]["provider"]
        self.assertFalse(provider["additionalProperties"])
        self.assertEqual(
            set(provider["required"]),
            {"name", "repository", "workflow", "run_id", "run_attempt", "head_sha"},
        )
        self.assertEqual(provider["properties"]["name"]["const"], "github-actions")
        self.assertEqual(provider["properties"]["repository"]["const"], "Jari81/buzz")
        self.assertEqual(
            provider["properties"]["workflow"]["const"],
            ".github/workflows/windows-fork-integration.yml",
        )
        self.assertEqual(provider["properties"]["run_attempt"]["minimum"], 1)

        digests = schema["properties"]["digests"]
        self.assertFalse(digests["additionalProperties"])
        self.assertEqual(
            set(digests["required"]),
            {"workflow_sha256", "manifest_schema_sha256", "generator_sha256"},
        )
        signing = schema["properties"]["signing"]
        self.assertFalse(signing["additionalProperties"])
        self.assertEqual(signing["properties"]["status"]["const"], "unsigned")

        artifacts = schema["properties"]["artifacts"]
        self.assertEqual(artifacts["minItems"], 1)
        self.assertEqual(artifacts["maxItems"], 1)
        artifact = artifacts["items"]
        self.assertFalse(artifact["additionalProperties"])
        self.assertEqual(
            set(artifact["required"]),
            {"type", "mime", "name", "path", "bytes", "sha256", "signing"},
        )
        self.assertEqual(artifact["properties"]["type"]["const"], "nsis-installer")
        self.assertEqual(artifact["properties"]["bytes"]["minimum"], 1)
        self.assertEqual(
            artifact["properties"]["sha256"]["pattern"], "^[0-9a-f]{64}$"
        )
        self.assertEqual(schema["properties"]["schema"]["type"], "string")
        for field in ("name", "repository", "workflow"):
            self.assertEqual(provider["properties"][field]["type"], "string")
        self.assertEqual(signing["properties"]["status"]["type"], "string")
        for field in ("type", "mime"):
            self.assertEqual(artifact["properties"][field]["type"], "string")
        self.assertEqual(
            artifact["properties"]["signing"]["properties"]["status"]["type"],
            "string",
        )


    def test_validate_checks_final_installer_hash_and_byte_count(self) -> None:
        installer_bytes = b"synthetic-final-installer-bytes"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            installer_dir = root / "nsis"
            artifact_dir = root / "artifact"
            installer_dir.mkdir()
            source_installer = installer_dir / "Buzz_1.2.3_x64-setup.exe"
            source_installer.write_bytes(installer_bytes)
            generated = subprocess.run(
                generate_command(installer_dir, artifact_dir),
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(generated.returncode, 0, generated.stderr)
            manifest = artifact_dir / "mybuzz-windows-build-manifest.json"
            final_installer = artifact_dir / source_installer.name
            validate_command = [
                sys.executable,
                str(TOOL),
                "validate",
                "--manifest",
                str(manifest),
                "--artifact-dir",
                str(artifact_dir),
                "--workflow-file",
                str(WORKFLOW),
                "--manifest-schema-file",
                str(SCHEMA),
            ]

            valid = subprocess.run(
                validate_command, text=True, capture_output=True, check=False
            )
            self.assertEqual(valid.returncode, 0, valid.stderr)

            final_installer.write_bytes(b"X" + installer_bytes[1:])
            wrong_hash = subprocess.run(
                validate_command, text=True, capture_output=True, check=False
            )
            self.assertNotEqual(wrong_hash.returncode, 0)
            self.assertIn("artifact SHA-256 mismatch", wrong_hash.stderr)

            final_installer.write_bytes(installer_bytes + b"extra")
            wrong_size = subprocess.run(
                validate_command, text=True, capture_output=True, check=False
            )
            self.assertNotEqual(wrong_size.returncode, 0)
            self.assertIn("artifact byte count mismatch", wrong_size.stderr)


    def test_validate_rejects_closed_shape_binding_digest_and_path_mismatches(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            installer_dir = root / "nsis"
            artifact_dir = root / "artifact"
            installer_dir.mkdir()
            installer = installer_dir / "Buzz_1.2.3_x64-setup.exe"
            installer.write_bytes(b"final-installer")
            generated = subprocess.run(
                generate_command(installer_dir, artifact_dir),
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(generated.returncode, 0, generated.stderr)
            manifest_path = artifact_dir / "mybuzz-windows-build-manifest.json"
            original = json.loads(manifest_path.read_text(encoding="utf-8"))

            def validate(path: Path = manifest_path) -> subprocess.CompletedProcess[str]:
                return subprocess.run(
                    [
                        sys.executable,
                        str(TOOL),
                        "validate",
                        "--manifest",
                        str(path),
                        "--artifact-dir",
                        str(artifact_dir),
                        "--workflow-file",
                        str(WORKFLOW),
                        "--manifest-schema-file",
                        str(SCHEMA),
                    ],
                    text=True,
                    capture_output=True,
                    check=False,
                )

            cases = [
                (
                    "extra field",
                    lambda value: value.update({"extra": True}),
                    "invalid manifest fields",
                ),
                (
                    "missing field",
                    lambda value: value.pop("release_id"),
                    "invalid manifest fields",
                ),
                (
                    "candidate mismatch",
                    lambda value: value.update({"candidate_sha": "d" * 40}),
                    "candidate SHA does not equal freeze SHA",
                ),
                (
                    "provider head mismatch",
                    lambda value: value["provider"].update({"head_sha": "d" * 40}),
                    "provider head SHA does not equal freeze SHA",
                ),
                (
                    "artifact path mismatch",
                    lambda value: value["artifacts"][0].update({"path": "Other.exe"}),
                    "artifact path must equal its safe basename",
                ),
                (
                    "workflow digest mismatch",
                    lambda value: value["digests"].update(
                        {"workflow_sha256": "0" * 64}
                    ),
                    "workflow SHA-256 mismatch",
                ),
                (
                    "schema digest mismatch",
                    lambda value: value["digests"].update(
                        {"manifest_schema_sha256": "0" * 64}
                    ),
                    "manifest schema SHA-256 mismatch",
                ),
                (
                    "generator digest mismatch",
                    lambda value: value["digests"].update(
                        {"generator_sha256": "0" * 64}
                    ),
                    "manifest generator SHA-256 mismatch",
                ),
                (
                    "multiple artifacts",
                    lambda value: value["artifacts"].append(
                        dict(value["artifacts"][0])
                    ),
                    "manifest must contain exactly one artifact",
                ),
            ]
            for label, mutate, error in cases:
                with self.subTest(label=label):
                    candidate = json.loads(json.dumps(original))
                    mutate(candidate)
                    manifest_path.write_text(
                        json.dumps(candidate) + "\n", encoding="utf-8"
                    )
                    result = validate()
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(error, result.stderr)

            manifest_path.write_text(json.dumps(original) + "\n", encoding="utf-8")
            external_manifest = root / "external-manifest.json"
            external_manifest.write_text(json.dumps(original) + "\n", encoding="utf-8")
            external = validate(external_manifest)
            self.assertNotEqual(external.returncode, 0)
            self.assertIn(
                "manifest must be the artifact directory's bound manifest",
                external.stderr,
            )

            final_installer = artifact_dir / installer.name
            outside = root / "outside.exe"
            final_installer.replace(outside)
            final_installer.symlink_to(outside)
            symlink = validate()
            self.assertNotEqual(symlink.returncode, 0)
            self.assertIn("artifact must be a regular non-symlink file", symlink.stderr)
            final_installer.unlink()
            outside.replace(final_installer)

            (artifact_dir / "unexpected.txt").write_text("extra", encoding="utf-8")
            extra = validate()
            self.assertNotEqual(extra.returncode, 0)
            self.assertIn(
                "artifact directory must contain only installer and manifest",
                extra.stderr,
            )


if __name__ == "__main__":
    unittest.main()
