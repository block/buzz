#!/usr/bin/env python3
"""Build and verify manually published relay images (Python 3.9+, stdlib only)."""

import argparse
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import uuid


PLATFORMS = ("linux/amd64", "linux/arm64")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")


def run(args, cwd=None, capture=True, timeout=120):
    """Run without a shell; stream long builds instead of retaining their logs."""
    result = subprocess.run(
        [str(a) for a in args], cwd=cwd, check=True, text=True,
        stdout=subprocess.PIPE if capture else None, timeout=timeout,
    )
    return result.stdout.strip() if capture else None


def source_sha(repo, ref, local=False):
    """Resolve once so moving branches cannot split architecture builds."""
    if not ref or ref.startswith(("-", "+", "^")) or ":" in ref or any(c.isspace() for c in ref):
        raise ValueError("Supply a branch, tag, PR head ref, or full commit SHA")
    if local:
        resolved = run(["git", "rev-parse", "--verify", "--end-of-options", ref + "^{commit}"], repo)
    else:
        private_ref = "refs/manual-image/" + uuid.uuid4().hex
        try:
            run(["git", "fetch", "--no-tags", "--no-write-fetch-head", "--", "origin",
                 ref + ":" + private_ref], repo, capture=False)
            resolved = run(["git", "rev-parse", "--verify", private_ref + "^{commit}"], repo)
        finally:
            run(["git", "update-ref", "-d", private_ref], repo)
    if not SHA.fullmatch(resolved):
        raise ValueError("Git did not resolve the source to a full commit SHA")
    return resolved


def repository_url(repo):
    """Normalize a GitHub origin without carrying credentials into image labels."""
    remote = run(["git", "remote", "get-url", "origin"], repo)
    match = re.fullmatch(r"(?:https://github\.com/|(?:[^/@:]+@)?github\.com:)([\w.-]+/[\w.-]+?)(?:\.git)?", remote)
    if not match:
        raise ValueError("Origin must be a GitHub HTTPS or SSH URL without embedded credentials")
    return "https://github.com/" + match[1]


def image_repository(value):
    """Require a registry/repository, never a caller-controlled release tag."""
    if not re.fullmatch(r"[a-z0-9.-]+(?::[0-9]+)?/[a-z0-9._/-]+", value):
        raise argparse.ArgumentTypeError("Use registry/owner/image without a tag or digest")
    return value


def inspect_image(image, template=None):
    command = ["docker", "buildx", "imagetools", "inspect", image]
    command += ["--format", template] if template else ["--raw"]
    return json.loads(run(command))


def verify_image(image, sha, source_url):
    """Verify the registry index and each runtime's actual configuration."""
    if not SHA.fullmatch(sha):
        raise ValueError("Expected a full source SHA")
    descriptor = inspect_image(image, "{{json .Manifest}}")
    digest = descriptor.get("digest", "")
    if not DIGEST.fullmatch(digest):
        raise ValueError("Registry did not return an image digest")
    repository = image.split("@", 1)[0]
    if ":" in repository.rsplit("/", 1)[-1]:
        repository = repository.rsplit(":", 1)[0]
    if "@" in image and image.split("@", 1)[1] != digest:
        raise ValueError("Registry digest differs from the requested digest")
    pinned = repository + "@" + digest
    index = inspect_image(pinned)
    runtimes = {}
    for manifest in index.get("manifests", []):
        platform = manifest.get("platform", {})
        name = platform.get("os", "") + "/" + platform.get("architecture", "")
        if name == "unknown/unknown":
            continue  # BuildKit attestations are not runnable platforms.
        if name not in PLATFORMS or name in runtimes:
            raise ValueError("Unexpected or duplicate runtime platform: " + name)
        child = manifest.get("digest", "")
        if not DIGEST.fullmatch(child):
            raise ValueError("Invalid runtime digest")
        config = inspect_image(repository + "@" + child, "{{json .Image}}")
        if config.get("os") + "/" + config.get("architecture") != name:
            raise ValueError("Runtime configuration platform does not match the index")
        labels = config.get("config", {}).get("Labels", {})
        if labels.get("org.opencontainers.image.revision") != sha:
            raise ValueError("Runtime source SHA mismatch: " + name)
        if labels.get("org.opencontainers.image.source") != source_url:
            raise ValueError("Runtime source repository mismatch: " + name)
        runtimes[name] = child
    if set(runtimes) != set(PLATFORMS):
        raise ValueError("Image must contain exactly one AMD64 and one ARM64 runtime")
    return {"image": pinned, "digest": digest, "platforms": runtimes,
            "source_sha": sha, "source_url": source_url,
            "ci_qualification": "skipped", "status": "registry-verified"}


def write_receipt(path, receipt):
    """Publish a receipt atomically only after successful verification."""
    path = Path(path).resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as f:
        temporary = Path(f.name)
        json.dump(receipt, f, indent=2)
        f.write("\n")
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def build(args):
    repo = Path(run(["git", "rev-parse", "--show-toplevel"]))
    source_url = repository_url(repo)
    ref = args.local_ref or args.ref
    sha = source_sha(repo, ref, local=args.local_ref is not None)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    build_id = "local-" + stamp + "-" + uuid.uuid4().hex[:8]
    tag = "manual-" + sha[:12] + "-" + build_id
    image = args.image + ":" + tag
    receipt_path = Path(args.receipt or (tempfile.gettempdir() + "/buzz-" + tag + ".json")).resolve()
    if receipt_path.exists():
        raise ValueError("Receipt already exists; select a new path so failure cannot leave a stale success")
    plan = {"source_ref": ref, "source_sha": sha, "source_url": source_url,
            "tag": image, "target": "runtime", "platforms": PLATFORMS,
            "build_id": build_id, "receipt": str(receipt_path)}
    print(json.dumps(plan, indent=2), flush=True)
    if args.dry_run:
        return
    run(["docker", "info", "--format", "{{.ServerVersion}}"])
    # Keep the builder and its cache across runs, without changing the selected builder.
    check = subprocess.run(["docker", "buildx", "inspect", args.builder],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
    with tempfile.TemporaryDirectory(prefix="buzz-image-") as temporary:
        temporary = Path(temporary)
        if check.returncode:
            config = temporary / "buildkitd.toml"
            config.write_text("[worker.oci]\n  max-parallelism = 2\n")
            run(["docker", "buildx", "create", "--name", args.builder,
                 "--driver", "docker-container", "--buildkitd-config", config])
        run(["docker", "buildx", "inspect", args.builder, "--bootstrap"], capture=False)
        worktree = temporary / "source"
        run(["git", "worktree", "add", "--detach", worktree, sha], repo, capture=False)
        try:
            metadata = temporary / "build.json"
            command = ["docker", "buildx", "build", "--builder", args.builder,
                       "--file", "Dockerfile", "--target", "runtime",
                       "--platform", ",".join(PLATFORMS),
                       "--build-arg", "BUZZ_SOURCE_SHA=" + sha,
                       "--build-arg", "BUZZ_BUILD_ID=" + build_id,
                       "--build-arg", "BUZZ_BUILD_URL=" + source_url + "/commit/" + sha,
                       "--label", "org.opencontainers.image.revision=" + sha,
                       "--label", "org.opencontainers.image.source=" + source_url,
                       "--label", "xyz.buzz.manual-build=true",
                       "--tag", image, "--metadata-file", metadata,
                       "--cache-from", "type=registry,ref=" + args.image + "-buildcache:amd64",
                       "--cache-from", "type=registry,ref=" + args.image + "-buildcache:arm64",
                       "--push", "."]
            run(command, worktree, capture=False, timeout=7200)
            expected = json.loads(metadata.read_text()).get("containerimage.digest", "")
            if not DIGEST.fullmatch(expected):
                raise ValueError("Build did not return a valid published digest")
            receipt = verify_image(image, sha, source_url)
            if receipt["digest"] != expected:
                raise ValueError("Published tag differs from the build output")
            receipt.update({"tag": image, "source_ref": ref, "build_id": build_id})
            write_receipt(receipt_path, receipt)
            print(json.dumps(receipt, indent=2))
            print("Receipt: " + str(receipt_path))
        finally:
            run(["git", "worktree", "remove", "--force", worktree], repo, capture=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    local = commands.add_parser("build", help="Build a selected committed snapshot and publish it")
    refs = local.add_mutually_exclusive_group(required=True)
    refs.add_argument("--ref", help="Fetch this branch, tag, SHA or PR head ref from origin")
    refs.add_argument("--local-ref", help="Use a committed local ref, including unpushed commits")
    modes = local.add_mutually_exclusive_group(required=True)
    modes.add_argument("--push", action="store_true")
    modes.add_argument("--dry-run", action="store_true")
    local.add_argument("--image", type=image_repository, default="ghcr.io/block/buzz")
    local.add_argument("--builder", default="buzz-manual")
    local.add_argument("--receipt", help="New path for the verified JSON receipt")
    verify = commands.add_parser("verify", help="Verify an existing manual multi-architecture image")
    verify.add_argument("--image", required=True)
    verify.add_argument("--sha", required=True)
    verify.add_argument("--source-url", required=True)
    verify.add_argument("--receipt", required=True)
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(args)
        else:
            if Path(args.receipt).exists():
                raise ValueError("Receipt already exists; use a new output path")
            receipt = verify_image(args.image, args.sha, args.source_url)
            write_receipt(args.receipt, receipt)
            print(json.dumps(receipt, indent=2))
    except (ValueError, OSError, subprocess.SubprocessError, KeyError, TypeError) as error:
        print("Manual image publishing failed: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
