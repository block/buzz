#!/usr/bin/env python3
"""Build an installable package for the Buzz browser plugin example.

Compiles the standalone stdio executable with a compile-time home URL baked
in via `BUZZ_BROWSER_HOME_URL`, then emits a package directory containing
`manifest.json` (with the real SHA-256 and byte count of the built
executable) and `bin/<target-triple>`.

Usage:
    python3 build.py --output <package-directory> [--home-url <http-or-https-url>]

The checked-in `manifest.json` and `bin/` layout in this source directory are
illustrative only — they are not installable. Only the directory this script
writes to `--output` is.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlparse

DEFAULT_HOME_URL = "https://example.com/"
MAX_URL_LEN = 2048
CRATE_DIR = Path(__file__).resolve().parent
PACKAGE_ID = "dev.example.webbrowser"
CONTRIBUTION_ID = "web"
# The only entries this script ever writes at the top level of a package.
ALLOWED_TOP_LEVEL_ENTRIES = {"manifest.json", "bin", "README.md"}
# The only filenames this script ever writes inside bin/ — the two macOS
# target triples named in the contract's package layout. Anything else
# inside an existing bin/ (e.g. an unrelated script someone left there)
# means the directory isn't safely ours to write into.
ALLOWED_TARGET_TRIPLES = {"aarch64-apple-darwin", "x86_64-apple-darwin"}


def validate_home_url(url: str) -> str:
    if not url:
        raise ValueError("--home-url must not be empty")
    if len(url) > MAX_URL_LEN:
        raise ValueError(f"--home-url exceeds {MAX_URL_LEN} characters")
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https"):
        raise ValueError("--home-url must start with http:// or https://")
    try:
        hostname = parsed.hostname
        _ = parsed.port  # raises ValueError on a malformed port
    except ValueError as error:
        raise ValueError(f"--home-url has an invalid host or port: {error}") from error
    if not hostname:
        raise ValueError("--home-url must include a host")
    return url


def host_target_triple() -> str:
    result = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True)
    for line in result.stdout.splitlines():
        if line.startswith("host: "):
            return line[len("host: ") :].strip()
    raise RuntimeError("could not determine host target triple from `rustc -vV`")


def cargo_build(home_url: str, triple: str) -> Path:
    # `option_env!` in main.rs reads BUZZ_BROWSER_HOME_URL at compile time.
    # rustc records env vars read via `option_env!`/`env!` as a compile
    # dependency, so a plain rebuild already picks up a changed value here —
    # no forced-recompile workaround needed.
    env = dict(os.environ)
    env["BUZZ_BROWSER_HOME_URL"] = home_url

    # Pin `--target` to the triple we detected and will label the manifest
    # with. Without this, Cargo can silently build for a different triple
    # (CARGO_BUILD_TARGET, `build.target` in .cargo/config.toml) than the
    # host `rustc -vV` reports, so the packaged artifact and the manifest's
    # `runtime.targets` key would disagree — always keep them the same
    # explicit triple, never Cargo's ambient default.
    #
    # `--message-format=json` reports the actual compiled artifact path, so
    # this works regardless of `CARGO_TARGET_DIR` or any target-dir override
    # in `.cargo/config.toml` — never assume `target/release/` or
    # `target/<triple>/release/`.
    process = subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--manifest-path",
            str(CRATE_DIR / "Cargo.toml"),
            "--target",
            triple,
            "--message-format=json",
        ],
        env=env,
        stdout=subprocess.PIPE,
        text=True,
    )
    if process.returncode != 0:
        raise RuntimeError(f"cargo build failed with exit code {process.returncode}")

    executable: Path | None = None
    for line in process.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        if "bin" not in message.get("target", {}).get("kind", []):
            continue
        if message.get("executable"):
            executable = Path(message["executable"])

    if executable is None or not executable.is_file():
        raise RuntimeError("cargo build did not report a bin executable artifact")
    return executable


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ensure_safe_package_dir(output: Path) -> None:
    """Refuse to mutate anything that isn't plausibly our own package output.

    Only ever proceeds against an empty directory, or a non-empty one that
    proves it's a package this script previously generated (a readable
    manifest.json with our plugin id). A non-empty directory that lacks that
    proof — even if every entry in it happens to be one of our filenames,
    e.g. a lone pre-existing README.md — is refused rather than partially
    overwritten. Never deletes or overwrites through a symlink.
    """
    if output.is_symlink():
        raise RuntimeError(f"refusing to use a symlink as --output: {output}")
    if output.exists() and not output.is_dir():
        raise RuntimeError(f"--output exists and is not a directory: {output}")
    if not output.exists():
        return

    entries = list(output.iterdir())
    if not entries:
        return

    for entry in entries:
        if entry.name not in ALLOWED_TOP_LEVEL_ENTRIES:
            raise RuntimeError(
                f"refusing to write into {output}: contains unexpected entry "
                f"{entry.name!r} (only reuse a directory this script created, "
                "or an empty one)"
            )
        if entry.is_symlink():
            raise RuntimeError(f"refusing to write into {output}: {entry.name} is a symlink")

    manifest_path = output / "manifest.json"
    if not manifest_path.exists():
        raise RuntimeError(
            f"refusing to write into non-empty {output}: no manifest.json to prove "
            "it's a package this script generated (point --output at an empty "
            "directory, or one this script already produced)"
        )
    try:
        existing = json.loads(manifest_path.read_text())
    except (json.JSONDecodeError, OSError) as error:
        raise RuntimeError(
            f"refusing to overwrite unreadable manifest.json in {output}: {error}"
        ) from error
    if existing.get("id") != PACKAGE_ID:
        raise RuntimeError(
            f"refusing to overwrite {manifest_path}: belongs to a different plugin id"
        )

    bin_dir = output / "bin"
    if bin_dir.exists():
        if not bin_dir.is_dir():
            raise RuntimeError(f"refusing to use non-directory bin/ in {output}")
        for entry in bin_dir.iterdir():
            if (
                entry.is_symlink()
                or not entry.is_file()
                or entry.name not in ALLOWED_TARGET_TRIPLES
            ):
                raise RuntimeError(
                    f"refusing to write into {bin_dir}: contains unexpected entry "
                    f"{entry.name!r}"
                )


def write_package(output: Path, triple: str, binary: Path) -> None:
    ensure_safe_package_dir(output)
    output.mkdir(parents=True, exist_ok=True)

    bin_dir = output / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    dest = bin_dir / triple
    if dest.exists():
        dest.unlink()
    shutil.copyfile(binary, dest, follow_symlinks=False)
    dest.chmod(0o755)

    digest = sha256_of(dest)
    size = dest.stat().st_size

    manifest = {
        "packageFormatVersion": "0.1.0-alpha",
        "contractVersion": "0.1.0-alpha",
        "id": PACKAGE_ID,
        "name": "Example Web Browser",
        "version": "0.1.0",
        "publisher": "Example Devs",
        "license": "Apache-2.0",
        "runtime": {
            "type": "stdio",
            "targets": {
                triple: {
                    "path": f"bin/{triple}",
                    "sha256": digest,
                    "bytes": size,
                }
            },
        },
        "grants": ["browser.browse"],
        "contributions": [
            {
                "kind": "browser",
                "id": CONTRIBUTION_ID,
                "title": "Web",
            }
        ],
    }
    manifest_path = output / "manifest.json"
    if manifest_path.exists():
        manifest_path.unlink()
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")

    readme = CRATE_DIR / "README.md"
    readme_dest = output / "README.md"
    if readme.is_file():
        if readme_dest.exists():
            readme_dest.unlink()
        shutil.copyfile(readme, readme_dest, follow_symlinks=False)

    print(f"package: {output}")
    print(f"target: {triple}")
    print(f"sha256: {digest}")
    print(f"bytes: {size}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="package output directory")
    parser.add_argument(
        "--home-url",
        default=DEFAULT_HOME_URL,
        help="compile-time home URL (default: %(default)s)",
    )
    args = parser.parse_args()

    try:
        home_url = validate_home_url(args.home_url)
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    try:
        triple = host_target_triple()
        binary = cargo_build(home_url, triple)
        write_package(args.output, triple, binary)
    except (subprocess.CalledProcessError, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
