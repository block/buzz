#!/usr/bin/env python3
"""Run a problem set with a team manifest and produce leaderboard-ready results.

One command wraps ``harbor run`` with only leaderboard-legal settings — no
timeout or resource overrides are accepted or forwarded, so the resulting job
directory passes Harbor's static validation as produced. After the run it
writes a ``metadata.yaml`` template derived from the manifest and prints the
exact upload/submit commands.

Run inside the testbed environment so ``harbor`` and the adapter are
importable:

    uv run --project benchmarks/harbor-buzz-orchestra/testbed \
        benchmarks/harbor-buzz-orchestra/scripts/run_leaderboard.py \
        --dataset terminal-bench/terminal-bench-2-1 \
        --attempts 5 \
        --manifest benchmarks/harbor-buzz-orchestra/manifests/<TEAM>.yaml \
        --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/<ENDPOINTS>.json \
        --provisioner-config <PROVISIONER.json> \
        --agent-bin-dir <DIR with Linux buzz-acp/buzz-agent/buzz-dev-mcp>
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import shutil
import subprocess
import sys
from pathlib import Path

import yaml

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
AGENT_IMPORT = "harbor_buzz_orchestra:BuzzOrchestraAgent"
PROVISIONER_FACTORY = "harbor_buzz_testbed:provisioner_from_dict"
# Host-side: the harness speaks to the relay as the trial user via this CLI.
BINARIES = ("buzz",)
# Container-side: the production stack uploaded into each task container.
# These must be Linux builds matching the task image architecture.
AGENT_BINARIES = ("buzz-acp", "buzz-agent", "buzz-dev-mcp")
# Uploaded alongside the stack when --relay-gateway is set: bridges the
# agents' canonical relay address to the host gateway (the relay is
# host-header tenant-bound, so agents must present its canonical Host).
FORWARDER_BINARY = "relay-forwarder"

PROVIDER_ORGS = {"anthropic": "Anthropic", "openai": "OpenAI", "databricks": "Databricks"}


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0], formatter_class=argparse.RawDescriptionHelpFormatter
    )
    problems = parser.add_mutually_exclusive_group(required=True)
    problems.add_argument(
        "--dataset", "-d", help="Registry dataset (e.g. terminal-bench/terminal-bench-2-1)"
    )
    problems.add_argument(
        "--path", "-p", type=Path, help="Local task or dataset directory"
    )
    parser.add_argument(
        "--include-task", "-i", action="append", default=[],
        help="Task name to include from the dataset (glob, repeatable)",
    )
    parser.add_argument(
        "--exclude-task", "-x", action="append", default=[],
        help="Task name to exclude from the dataset (glob, repeatable)",
    )
    parser.add_argument(
        "--attempts", "-k", type=int, required=True,
        help="Runs per problem (leaderboards require 5)",
    )
    parser.add_argument("--manifest", type=Path, required=True, help="Team manifest YAML")
    parser.add_argument(
        "--endpoint-config", type=Path, required=True,
        help="JSON mapping manifest endpoint names to providers/API keys",
    )
    parser.add_argument(
        "--provisioner-config", type=Path, required=True,
        help="JSON config for the Buzz relay/Postgres provisioner",
    )
    parser.add_argument(
        "--buzz-bin-dir", type=Path, default=None,
        help="Directory with the host buzz CLI (default: repo target/release, then target/debug)",
    )
    parser.add_argument(
        "--agent-bin-dir", type=Path, required=True,
        help="Directory with Linux builds of buzz-acp/buzz-agent/buzz-dev-mcp "
        "to upload into each task container",
    )
    parser.add_argument(
        "--relay-gateway", default="",
        help="host:port of the benchmark relay as reachable from inside the "
        "task container (e.g. host.docker.internal:3600). When set, a "
        "loopback forwarder from --agent-bin-dir bridges the canonical "
        "relay address to this gateway",
    )
    parser.add_argument("--n-concurrent", "-n", type=int, default=4, help="Concurrent trials")
    parser.add_argument("--jobs-dir", type=Path, default=Path("jobs"), help="Job output root")
    parser.add_argument("--job-name", default=None, help="Job name (default: lb-<condition>-<UTC>)")
    parser.add_argument(
        "--upload", action="store_true", help="Upload to Harbor Hub when the job finishes"
    )
    parser.add_argument("--dry-run", action="store_true", help="Print the harbor command and exit")
    return parser.parse_args(argv)


def find_binaries(bin_dir: Path | None) -> dict[str, Path]:
    candidates = (
        [bin_dir]
        if bin_dir is not None
        else [PACKAGE_ROOT.parents[1] / "target" / kind for kind in ("release", "debug")]
    )
    for candidate in candidates:
        found = {name: candidate / name for name in BINARIES}
        if all(path.is_file() for path in found.values()):
            return found
    searched = ", ".join(str(c) for c in candidates)
    raise SystemExit(
        f"buzz binaries not found (need {', '.join(BINARIES)}; searched {searched}). "
        "Build them with `cargo build` or pass --buzz-bin-dir."
    )


def find_agent_binaries(bin_dir: Path, with_forwarder: bool = False) -> dict[str, Path]:
    """The Linux agent stack uploaded into each task container."""
    names = AGENT_BINARIES + ((FORWARDER_BINARY,) if with_forwarder else ())
    found = {name: bin_dir / name for name in names}
    missing = [name for name, path in found.items() if not path.is_file()]
    if missing:
        raise SystemExit(
            f"Linux agent binaries not found in {bin_dir}: {', '.join(missing)}. "
            "`just benchmark` builds them; or cross-compile with "
            "cargo --target <arch>-unknown-linux-musl and pass --agent-bin-dir."
        )
    return found


def build_command(
    args: argparse.Namespace,
    binaries: dict[str, Path],
    agent_binaries: dict[str, Path],
) -> list[str]:
    """Compose the harbor invocation. Standard settings only: any timeout or
    resource override would fail leaderboard static validation, so none are
    accepted or forwarded."""
    command = [
        "harbor", "run", "--yes",
        "--job-name", args.job_name,
        "--jobs-dir", str(args.jobs_dir),
        "-k", str(args.attempts),
        "--n-concurrent", str(args.n_concurrent),
    ]
    if args.dataset:
        command += ["--dataset", args.dataset]
    else:
        command += ["--path", str(args.path)]
    for pattern in args.include_task:
        command += ["--include-task-name", pattern]
    for pattern in args.exclude_task:
        command += ["--exclude-task-name", pattern]
    if args.upload:
        command.append("--upload")
    command += ["--agent", AGENT_IMPORT]
    kwargs = {
        "manifest": args.manifest,
        "provisioner_factory": PROVISIONER_FACTORY,
        "provisioner_config": args.provisioner_config,
        "artifact_root": PACKAGE_ROOT,
        "endpoint_config": args.endpoint_config,
        "buzz_acp_binary": agent_binaries["buzz-acp"],
        "buzz_agent_binary": agent_binaries["buzz-agent"],
        "buzz_dev_mcp_binary": agent_binaries["buzz-dev-mcp"],
        "buzz_cli_binary": binaries["buzz"],
        "run_id": args.job_name,
    }
    if args.relay_gateway:
        kwargs["relay_gateway"] = args.relay_gateway
        kwargs["forwarder_binary"] = agent_binaries[FORWARDER_BINARY]
    for key, value in kwargs.items():
        command += ["--agent-kwarg", f"{key}={value}"]
    return command


def write_metadata_template(args: argparse.Namespace, job_dir: Path) -> Path:
    """Derive a metadata.yaml template (harbor.leaderboard.metadata schema)
    from the manifest roster; display-name placeholders are for the submitter
    to confirm."""
    manifest = yaml.safe_load(args.manifest.read_text())
    endpoints = json.loads(args.endpoint_config.read_text())
    models, seen = [], set()
    for entry in manifest.get("roster", []):
        model = entry.get("model_revision") or entry["endpoint"]
        if model in seen:
            continue
        seen.add(model)
        provider = endpoints.get(entry["endpoint"], {}).get("provider", "FILL_ME")
        models.append(
            {
                "model_name": model,
                "model_provider": provider,
                "model_display_name": model,
                "model_org_display_name": PROVIDER_ORGS.get(provider, "FILL_ME"),
            }
        )
    metadata = {
        "agent_url": "https://github.com/block/buzz",
        "agent_display_name": f"Buzz Orchestra ({manifest.get('condition', 'team')})",
        "agent_org_display_name": "Block",
        "models": models,
    }
    path = job_dir / "metadata.yaml"
    path.write_text(yaml.safe_dump(metadata, sort_keys=False))
    return path


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    for label, path in (
        ("manifest", args.manifest),
        ("endpoint config", args.endpoint_config),
        ("provisioner config", args.provisioner_config),
    ):
        if not path.is_file():
            raise SystemExit(f"{label} not found: {path}")
    if args.job_name is None:
        condition = yaml.safe_load(args.manifest.read_text()).get("condition", "team")
        stamp = dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")
        args.job_name = f"lb-{condition}-{stamp}"

    if args.dry_run:
        # Dry runs print the command without requiring built binaries.
        bin_dir = args.buzz_bin_dir or PACKAGE_ROOT.parents[1] / "target" / "release"
        binaries = {name: bin_dir / name for name in BINARIES}
        agent_binaries = {
            name: args.agent_bin_dir / name
            for name in AGENT_BINARIES + (FORWARDER_BINARY,)
        }
        print(" ".join(build_command(args, binaries, agent_binaries)))
        return 0
    binaries = find_binaries(args.buzz_bin_dir)
    agent_binaries = find_agent_binaries(
        args.agent_bin_dir, with_forwarder=bool(args.relay_gateway)
    )
    command = build_command(args, binaries, agent_binaries)
    if shutil.which("harbor") is None:
        raise SystemExit(
            "harbor not on PATH — run via: uv run --project "
            f"{PACKAGE_ROOT / 'testbed'} {Path(__file__).resolve()} ..."
        )

    result = subprocess.run(command)
    job_dir = args.jobs_dir / args.job_name
    if result.returncode != 0:
        print(f"harbor run failed (exit {result.returncode}); job dir: {job_dir}")
        return result.returncode

    metadata_path = write_metadata_template(args, job_dir)
    print("\nLeaderboard-ready job complete.")
    print(f"  1. Review submitter details in {metadata_path}")
    print(f"  2. harbor upload {job_dir}")
    print(
        "  3. harbor leaderboard submit -l terminal-bench/terminal-bench-2-1 "
        f"-j <job UUID from upload> -m {metadata_path}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
