#!/usr/bin/env python3
"""Regression tests for build.py's non-mutating safety checks.

Run with: python3 test_build.py

These exercise `ensure_safe_package_dir` and `validate_home_url` directly —
no cargo/rustc invocation, no network, no filesystem mutation outside a
per-test temporary directory.
"""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location("build", Path(__file__).resolve().parent / "build.py")
build = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(build)


class EnsureSafePackageDirTests(unittest.TestCase):
    def test_missing_directory_is_allowed(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "does-not-exist-yet"
            build.ensure_safe_package_dir(output)  # must not raise

    def test_empty_existing_directory_is_allowed(self):
        with tempfile.TemporaryDirectory() as tmp:
            build.ensure_safe_package_dir(Path(tmp))  # must not raise

    def test_readme_only_directory_without_manifest_is_rejected_and_preserved(self):
        # A directory whose only entry happens to be one of our filenames
        # is not proof it's a package we generated — refuse it, and leave
        # the file untouched, rather than silently adopting and
        # overwriting it.
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "pkg"
            output.mkdir()
            readme = output / "README.md"
            readme.write_text("someone else's README\n")

            with self.assertRaises(RuntimeError):
                build.write_package(output, "aarch64-apple-darwin", Path(tmp) / "unused-binary")

            self.assertEqual(readme.read_text(), "someone else's README\n")
            self.assertFalse((output / "manifest.json").exists())
            self.assertFalse((output / "bin").exists())

    def test_existing_own_manifest_is_allowed(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / "manifest.json").write_text(json.dumps({"id": build.PACKAGE_ID}))
            build.ensure_safe_package_dir(output)  # must not raise

    def test_existing_foreign_manifest_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / "manifest.json").write_text(json.dumps({"id": "someone.else.plugin"}))
            with self.assertRaises(RuntimeError):
                build.ensure_safe_package_dir(output)

    def test_unexpected_top_level_entry_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / "notes.txt").write_text("unrelated\n")
            with self.assertRaises(RuntimeError):
                build.ensure_safe_package_dir(output)

    def test_symlinked_output_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            real = Path(tmp) / "real"
            real.mkdir()
            (real / "secret.txt").write_text("must survive\n")
            link = Path(tmp) / "link"
            link.symlink_to(real)
            with self.assertRaises(RuntimeError):
                build.ensure_safe_package_dir(link)
            self.assertEqual((real / "secret.txt").read_text(), "must survive\n")

    def test_symlinked_manifest_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "pkg"
            output.mkdir()
            target = Path(tmp) / "unrelated.txt"
            target.write_text("must survive\n")
            (output / "manifest.json").symlink_to(target)
            with self.assertRaises(RuntimeError):
                build.ensure_safe_package_dir(output)
            self.assertEqual(target.read_text(), "must survive\n")

    def test_unexpected_entry_inside_bin_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / "manifest.json").write_text(json.dumps({"id": build.PACKAGE_ID}))
            bin_dir = output / "bin"
            bin_dir.mkdir()
            (bin_dir / "valuable-script").write_text("precious data\n")
            with self.assertRaises(RuntimeError):
                build.ensure_safe_package_dir(output)
            self.assertEqual((bin_dir / "valuable-script").read_text(), "precious data\n")


class ValidateHomeUrlTests(unittest.TestCase):
    def test_valid_https_url_is_accepted(self):
        self.assertEqual(
            build.validate_home_url("https://example.com:8443/start"),
            "https://example.com:8443/start",
        )

    def test_empty_url_is_rejected(self):
        with self.assertRaises(ValueError):
            build.validate_home_url("")

    def test_non_http_scheme_is_rejected(self):
        with self.assertRaises(ValueError):
            build.validate_home_url("ftp://example.com/")

    def test_missing_hostname_is_rejected(self):
        for url in ("http://:80/", "https://user@/", "https://"):
            with self.assertRaises(ValueError):
                build.validate_home_url(url)

    def test_overlong_url_is_rejected(self):
        with self.assertRaises(ValueError):
            build.validate_home_url("https://example.com/" + "a" * build.MAX_URL_LEN)


if __name__ == "__main__":
    sys.exit(unittest.main())
