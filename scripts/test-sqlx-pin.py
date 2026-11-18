#!/usr/bin/env python3
"""Exercise the dependency guard through its real command and repository inputs."""

import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
INPUTS = {name: (ROOT / name).read_text() for name in ("Cargo.toml", "Cargo.lock", "renovate.json")}
REVISION = "c3735abf034689bb1fab9d7bf9f040e492de28cf"


def replace_once(text, old, new):
    if old not in text:
        raise AssertionError(f"missing mutation target: {old}")
    return text.replace(old, new, 1)


def edit_rules(text, edit):
    config = json.loads(text)
    edit(config["packageRules"])
    return json.dumps(config)


class SqlxPinTests(unittest.TestCase):
    def run_guard(self, filename=None, change=None):
        with tempfile.TemporaryDirectory(prefix="buzz-sqlx-pin-") as directory:
            root = Path(directory)
            for name, text in INPUTS.items():
                (root / name).write_text(change(text) if name == filename else text)
            return subprocess.run(
                [sys.executable, str(ROOT / "scripts/check-sqlx-pin.py"), str(root)],
                capture_output=True,
                text=True,
                check=False,
            )

    def test_current_repository_passes(self):
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_dependency_updates_cannot_silently_remove_the_fix(self):
        cases = [
            (
                "version range", "Cargo.toml",
                lambda s: replace_once(s, 'version = "=0.9.0"', 'version = "0.9"'),
                "exactly",
            ),
            (
                "changed patch", "Cargo.toml",
                lambda s: replace_once(s, REVISION, "0" * 40), "Git patch",
            ),
            (
                "missing patch", "Cargo.toml",
                lambda s: re.sub(r'^sqlx-sqlite = .*\n', '', s, flags=re.M), "Git patch",
            ),
            (
                "changed lock revision", "Cargo.lock",
                lambda s: replace_once(s, REVISION, "0" * 40), "Cargo.lock",
            ),
            (
                "registry replacement", "Cargo.lock",
                lambda s: re.sub(
                    r'^source = "git\+https://github.com/launchbadge/sqlx[^\n]+',
                    'source = "registry+https://github.com/rust-lang/crates.io-index"',
                    s, flags=re.M,
                ),
                "Cargo.lock",
            ),
            (
                "duplicate version", "Cargo.lock",
                lambda s: s + '\n[[package]]\nname = "sqlx"\nversion = "0.9.1"\n',
                "Cargo.lock",
            ),
            (
                "missing lock package", "Cargo.lock",
                lambda s: re.sub(
                    r'\[\[package\]\]\nname = "sqlx-sqlite"\n.*?(?=\[\[package\]\]|\Z)',
                    '', s, flags=re.S,
                ),
                "Cargo.lock",
            ),
            (
                "removed Renovate rule", "renovate.json",
                lambda s: edit_rules(s, lambda rules: rules.pop()), "Renovate",
            ),
            (
                "enabled Renovate", "renovate.json",
                lambda s: edit_rules(s, lambda rules: rules[-1].update(enabled=True)),
                "Renovate",
            ),
            (
                "omitted crate", "renovate.json",
                lambda s: edit_rules(s, lambda rules: rules[-1]["matchDepNames"].pop()),
                "Renovate",
            ),
            (
                "later override", "renovate.json",
                lambda s: edit_rules(s, lambda rules: rules.append({
                    "matchManagers": ["cargo"], "enabled": True,
                })),
                "Renovate",
            ),
            (
                "narrowed rule", "renovate.json",
                lambda s: edit_rules(s, lambda rules: rules[-1].update(matchUpdateTypes=["major"])),
                "Renovate",
            ),
        ]
        for name, filename, change, diagnostic in cases:
            with self.subTest(name=name):
                self.assertNotEqual(change(INPUTS[filename]), INPUTS[filename])
                result = self.run_guard(filename, change)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn(diagnostic, result.stderr)


if __name__ == "__main__":
    unittest.main()
