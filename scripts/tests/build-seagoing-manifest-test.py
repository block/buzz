#!/usr/bin/env python3
"""Unit tests for the sea-going component manifest builder."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "build-seagoing-manifest.py"


def load_module():
    spec = importlib.util.spec_from_file_location("seagoing_manifest", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load sea-going manifest module")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SeaGoingManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def component(self, role: str, name: str, relative: str, content: bytes):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return self.module.ComponentInput(role=role, name=name, path=path)

    def required_components(self):
        return [
            self.component("app", "command-adviser", "Command Adviser.app/app", b"app"),
            self.component("model", "gemma", "models/gemma.gguf", b"model"),
            self.component("embedding_model", "bge-m3", "models/bge-m3.bin", b"embed"),
            self.component("rag_snapshot", "rag", "rag/manifest.json", b"rag"),
            self.component("memory_backup", "memory", "memory/vault.tar.gz.enc", b"memory"),
            self.component("relay", "relay", "bin/buzz-relay", b"relay"),
            self.component("recovery", "runbook", "docs/recovery.md", b"recover"),
        ]

    def test_bundle_id_is_deterministic_across_input_order(self):
        components = self.required_components()
        first = self.module.build_manifest(components, [])
        second = self.module.build_manifest(list(reversed(components)), [])
        self.assertEqual(first["bundle_id"], second["bundle_id"])
        self.assertEqual(first["components"], second["components"])

    def test_directory_hash_is_stable_and_tracks_relative_paths(self):
        directory = self.root / "tree"
        (directory / "b").mkdir(parents=True)
        (directory / "b" / "two").write_bytes(b"same")
        (directory / "one").write_bytes(b"same")
        first = self.module.inventory_path(directory)
        (directory / "one").rename(directory / "renamed")
        second = self.module.inventory_path(directory)
        self.assertNotEqual(first.sha256, second.sha256)

    def test_symlinks_are_rejected(self):
        target = self.root / "target"
        target.write_text("payload", encoding="utf-8")
        link = self.root / "link"
        link.symlink_to(target)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.module.inventory_path(link)

    def test_missing_required_role_is_rejected(self):
        components = self.required_components()
        components = [item for item in components if item.role != "relay"]
        with self.assertRaisesRegex(ValueError, "missing required component roles: relay"):
            self.module.build_manifest(components, [])

    def test_duplicate_component_name_is_rejected(self):
        components = self.required_components()
        components.append(
            self.component("recovery", "relay", "docs/other.md", b"duplicate")
        )
        with self.assertRaisesRegex(ValueError, "duplicate component name"):
            self.module.build_manifest(components, [])

    def test_protected_config_records_permissions_without_content_hash(self):
        config = self.root / "trusted.json"
        config.write_text('{"secret":"not-for-manifest"}', encoding="utf-8")
        config.chmod(0o600)
        protected = self.module.ProtectedConfigInput(name="trusted-sources", path=config)
        manifest = self.module.build_manifest(self.required_components(), [protected])
        recorded = manifest["protected_configs"][0]
        self.assertEqual(recorded["mode"], "0600")
        self.assertNotIn("sha256", recorded)
        self.assertNotIn("secret", json.dumps(recorded))

    def test_protected_config_must_be_mode_0600(self):
        config = self.root / "trusted.json"
        config.write_text("{}", encoding="utf-8")
        config.chmod(0o644)
        protected = self.module.ProtectedConfigInput(name="trusted-sources", path=config)
        with self.assertRaisesRegex(ValueError, "mode 0600"):
            self.module.build_manifest(self.required_components(), [protected])

    def test_materialisation_rejects_insufficient_free_space(self):
        manifest = self.module.build_manifest(self.required_components(), [])

        def almost_full(_path):
            return shutil._ntuple_diskusage(total=1000, used=790, free=210)

        with self.assertRaisesRegex(ValueError, "insufficient free space"):
            self.module.ensure_materialization_capacity(
                self.root, manifest["payload_bytes"], disk_usage=almost_full
            )

    def test_materialisation_copies_payloads_without_protected_config(self):
        components = self.required_components()
        config = self.root / "trusted.json"
        config.write_text("sensitive", encoding="utf-8")
        config.chmod(0o600)
        destination = self.root / "portable"
        manifest = self.module.build_manifest(
            components,
            [self.module.ProtectedConfigInput(name="trusted-sources", path=config)],
        )

        def enough_space(_path):
            return shutil._ntuple_diskusage(total=10_000, used=0, free=10_000)

        materialized = self.module.materialize(
            manifest, components, destination, disk_usage=enough_space
        )
        self.assertTrue((destination / "payload" / "gemma" / "gemma.gguf").is_file())
        self.assertFalse((destination / "payload" / "trusted-sources").exists())
        self.assertTrue(all(item["materialized"] for item in materialized["components"]))

    def test_verification_rejects_changed_payload(self):
        components = self.required_components()
        manifest = self.module.build_manifest(components, [])
        self.module.verify_manifest(manifest)
        components[0].path.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "component identity changed"):
            self.module.verify_manifest(manifest)


if __name__ == "__main__":
    unittest.main()
