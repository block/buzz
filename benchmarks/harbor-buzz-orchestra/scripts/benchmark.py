#!/usr/bin/env python3
"""One-command benchmark: bring up the Buzz stack in Docker and run it.

``just benchmark`` wraps this script. Defaults are leaderboard-eligible out
of the box (Terminal-Bench 2.1, 5 attempts per problem, the Sonnet+Haiku
team); every ``run_leaderboard.py`` selector passes through unchanged. The
script owns everything around the run:

- A dedicated ``buzz-benchmark`` compose project reusing the production
  bundle (``deploy/compose/compose.yml``) plus the benchmark port overlay,
  on its own ports (relay :3600, Postgres :5633, metrics :9602) so it never
  collides with a dev stack. Secrets and identities are generated once into
  the gitignored ``.benchmark/`` state dir and reused across runs.
- One pinned *user* identity for the whole benchmark environment: it owns
  every trial channel and posts every task, like one human running many
  teams. Channels are kept (not archived) after each trial.
- ``--gui`` adds that user to the relay membership list and opens the Buzz
  desktop app logged in as them, so a human can watch the teams work live.

Run inside the testbed environment (the just recipe does this):

    uv run --project benchmarks/harbor-buzz-orchestra/testbed \
        benchmarks/harbor-buzz-orchestra/scripts/benchmark.py [--gui] [...]
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import secrets
import shutil
import subprocess
import sys
import time
from pathlib import Path

from harbor_buzz_orchestra.container_runtime import (
    REMOTE_CA_BUNDLE,
    VERIFIER_DEPS,
    install_verifier_deps_command,
    seed_trust_store_command,
)
from harbor_buzz_orchestra.verifier_health import verifier_broken

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
REPO_ROOT = PACKAGE_ROOT.parents[1]
STATE_DIR = PACKAGE_ROOT / ".benchmark"

COMPOSE_PROJECT = "buzz-benchmark"
COMPOSE_FILES = (
    REPO_ROOT / "deploy" / "compose" / "compose.yml",
    PACKAGE_ROOT / "testbed" / "compose.benchmark.yml",
)
# Stand-in for a task image in the container preflight: the base most
# Terminal-Bench images derive from, and like them it ships neither
# ca-certificates nor curl.
CANARY_IMAGE = "ubuntu:24.04"
RELAY_HTTP_PORT = 3600
PG_HOST_PORT = 5633
METRICS_HOST_PORT = 9602
GUI_BUNDLE_IDENTIFIER = "xyz.block.buzz.app.benchmark"

# Harbor accepts two dataset forms and resolves them from different places:
#   org/name[@ref]  -> the hub package registry (PackageDatasetClient)
#   name[@version]  -> registry.json (JsonRegistryClient)
# Terminal-Bench 2.1 is only published as a package: `terminal-bench@2.0` in
# registry.json is the older 2.0 cut. Verified 2026-07-26 against the pinned
# harbor 0.16.1 — terminal-bench/terminal-bench-2-1 resolves to 89 tasks.
# validate_dataset() checks whichever form is given against the right source.
DEFAULT_DATASET = "terminal-bench/terminal-bench-2-1"
DEFAULT_ATTEMPTS = 5
# Scales every Harbor phase deadline: agent, verifier, setup, image build.
#
# 3.0 rather than 1.0 because this study is asking what the team can solve, and
# a clock that stops a still-working agent answers a different question. It is
# deliberately generous: at 1.0, three trials of the first solo sweep were cut
# off mid-turn, and no multiplier that merely clears those would tell us whether
# a fourth was about to be. Time is still measured — it is one of the study's
# four headline numbers — it is just measured rather than enforced.
#
# The cost is submittability, not validity: Harbor's leaderboard validator
# rejects any job whose multiplier is set at all (no_job_overrides, in
# harbor/leaderboard/static_validation.py:733), so these scores are ours to
# report with the multiplier stated, not to post. Pass --timeout-multiplier 1.0
# for a run meant for the leaderboard.
DEFAULT_TIMEOUT_MULTIPLIER = 3.0
# The longest `[agent] timeout_sec` in Terminal-Bench 2.1, from the 89 cached
# task.toml files: 600s(1) 750s(1) 900s(48) 1200s(5) 1800s(17) 2400s(2)
# 3600s(13) 7200s(1) 12000s(1). Harbor multiplies each task's own value, so this
# is what the largest one becomes — and the manifest's trial_budget has to clear
# it, or the harness's own wait_for cuts the biggest tasks short of the deadline
# Harbor granted them. Checked before every run; see check_budget_clears_clock.
MAX_TASK_TIMEOUT_SECONDS = 12000
# The Databricks bring-up pair is the default because it is the slate the
# study actually runs on (doc 04 §6); the Anthropic manifests predate it and
# need a key this project does not provision.
DEFAULT_MANIFEST = PACKAGE_ROOT / "manifests" / "tb-solo-luna.yaml"
DEFAULT_ENDPOINTS = PACKAGE_ROOT / "testbed" / "endpoints" / "databricks-live.json"
SCHEMA_SQL = PACKAGE_ROOT / "testbed" / "sql" / "benchmark_schema.sql"

# Linux builds of the production agent stack, uploaded into each task
# container per trial. Built once in a rust:alpine container (musl → fully
# static, runs on any Linux task image of the same architecture) and cached.
AGENT_BINARIES = ("buzz-acp", "buzz-agent", "buzz-dev-mcp")
# Std-only loopback forwarder (not a workspace crate): agents dial the
# relay's canonical localhost address inside the task container and the
# forwarder bridges to the Docker host gateway. Compiled with plain rustc
# in the same cross-build step.
FORWARDER_SOURCE = PACKAGE_ROOT / "forwarder" / "relay_forwarder.rs"
FORWARDER_BINARY = "relay-forwarder"
LINUX_TARGET_DIR = STATE_DIR / "linux-target"
RUST_IMAGE = "rust:1.95-alpine"

_spec = importlib.util.spec_from_file_location(
    "run_leaderboard", Path(__file__).resolve().parent / "run_leaderboard.py"
)
run_leaderboard = importlib.util.module_from_spec(_spec)
sys.modules.setdefault("run_leaderboard", run_leaderboard)
_spec.loader.exec_module(run_leaderboard)


# The host every Terminal-Bench verifier reaches before it can score anything:
# each one pip/uv-installs pytest into the task container as its first act.
PYPI_TLS_HOST = "files.pythonhosted.org"

# Substrings that mark a certificate issuer as a corporate interception CA
# rather than a public one. Matching on the operator's own name is deliberate:
# a public CA never puts a company name in the O= field of a TLS server chain,
# and every intercepting proxy does, because the whole point is that it minted
# the certificate itself.
INTERCEPTION_ISSUER_MARKERS = (
    "cloudflare gateway",
    "zscaler",
    "netskope",
    "palo alto",
    "forcepoint",
    "bluecoat",
    "blue coat",
    "mitmproxy",
    "charles proxy",
    "fiddler",
)


# Harbor exception types that describe the agent's own performance rather than
# a fault in the harness. A trial that ends this way has a legitimate score and
# must not be re-run on resume; see scored_tasks().
AGENT_FAULT_EXCEPTIONS = frozenset({"AgentTimeoutError"})


class InterceptedTLSError(RuntimeError):
    """The path to PyPI is being MITM'd, so no verifier can install pytest."""


def pypi_tls_issuer(timeout: float = 10.0) -> str | None:
    """The O=/CN= of whoever signed the certificate PyPI is serving us.

    Returns None when the issuer cannot be determined — no network, no client, a
    malformed handshake. Unknown is not the same as intercepted, and a preflight
    that cannot see must not block the run.

    ``curl`` first, because it is the only one of the two that goes through an
    egress proxy. On a host whose only route out is a CONNECT proxy, bare
    ``openssl s_client`` never reaches PyPI at all: the direct connection is reset
    at the first payload byte, no certificate is presented, and this returns None
    — so the guard silently skips on exactly the locked-down runners where a
    corporate gateway is most likely to be sitting in the path. curl reads
    ``https_proxy`` from the environment and handles CONNECT and proxy auth, so it
    inspects the certificate the verifier will actually be shown. openssl stays as
    the fallback for a host without curl.
    """
    try:
        completed = subprocess.run(
            ["curl", "-sSv", "--max-time", str(int(timeout)), f"https://{PYPI_TLS_HOST}/"],
            capture_output=True, text=True, timeout=timeout + 5,
        )
    except (OSError, subprocess.SubprocessError):
        pass
    else:
        # curl writes the handshake trace to stderr as "*  issuer: C=..; O=..".
        for line in completed.stderr.splitlines():
            stripped = line.lstrip("* ").strip()
            if stripped.startswith("issuer:"):
                return stripped[len("issuer:"):].strip()

    try:
        completed = subprocess.run(
            [
                "openssl", "s_client",
                "-connect", f"{PYPI_TLS_HOST}:443",
                "-servername", PYPI_TLS_HOST,
            ],
            input="", capture_output=True, text=True, timeout=timeout,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    for line in completed.stdout.splitlines():
        if line.startswith("issuer="):
            return line[len("issuer="):].strip()
    return None


def check_tls_not_intercepted() -> None:
    """Refuse to start a sweep that cannot possibly be scored.

    Terminal-Bench verifiers install pytest from PyPI inside the task
    container, which trusts only public roots. Under a TLS-intercepting
    gateway that install fails, the verifier never runs, and the trial is
    recorded as reward 0 — indistinguishable, in the results, from an agent
    that got the answer wrong.

    That is not hypothetical. Cloudflare WARP connected partway through an
    89-task sweep and turned it into 0/89: 88 verifiers died on
    ``invalid peer certificate: UnknownIssuer`` while the agents worked
    correctly and published sound reports. It cost two hours and $17.71 and
    read, at a glance, like the model had collapsed. Ten seconds of handshake
    here is cheap against that.

    A clean network makes this a no-op, so it costs nothing on a CI box or an
    EC2 runner where no interception exists.
    """
    issuer = pypi_tls_issuer()
    if issuer is None:
        print(f"  TLS to {PYPI_TLS_HOST}: issuer unknown (skipping check)")
        return
    lowered = issuer.lower()
    marker = next((m for m in INTERCEPTION_ISSUER_MARKERS if m in lowered), None)
    if marker is None:
        print(f"  TLS to {PYPI_TLS_HOST}: {issuer}")
        return
    raise InterceptedTLSError(
        f"TLS to {PYPI_TLS_HOST} is being intercepted by: {issuer}\n"
        "  Task containers do not trust that CA, so every verifier's "
        "`pip install pytest` will fail and every trial will score 0 for a "
        "reason that has nothing to do with the agent.\n"
        "  Disconnect the interception client (for Cloudflare WARP: "
        "`warp-cli disconnect`) and re-run, or run on a host without it."
    )


class ContainerEgressError(RuntimeError):
    """A task container cannot install what every verifier needs."""


def container_scoring_probe(timeout: float = 180.0) -> tuple[str, str] | None:
    """Run the trials' own container setup against a canary image.

    Returns ``(trust_store, verifier_deps)`` as the two shells report them, or
    None when the probe could not be carried out at all — no docker, no image,
    a pull that timed out. As with the TLS check above, a preflight that cannot
    see must not block the run.

    ``ubuntu:24.04`` is the canary because it is the base most Terminal-Bench
    images derive from and it ships neither ``ca-certificates`` nor ``curl`` —
    the exact starting state that made the A1 sweep lose trials. The docker CLI
    injects the host's proxy settings into every container it starts, so the
    canary reaches the network by the same path a trial does.
    """
    try:
        import certifi
    except ImportError:
        return None
    script = f"{seed_trust_store_command()}; {install_verifier_deps_command()}"
    try:
        completed = subprocess.run(
            [
                "docker", "run", "--rm",
                "-v", f"{certifi.where()}:{REMOTE_CA_BUNDLE}:ro",
                "--entrypoint", "sh", CANARY_IMAGE, "-c", script,
            ],
            capture_output=True, text=True, timeout=timeout,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    lines = (completed.stdout or "").split()
    if len(lines) < 2:
        return None
    return lines[0], lines[1]


def check_container_can_be_scored() -> None:
    """Refuse a sweep whose verifiers will not be able to install pytest.

    81 of the 89 Terminal-Bench ``test.sh`` files run ``apt-get install -y
    curl`` to fetch the uv installer before they can run a single test. If a
    fresh container cannot do that, every one of those trials records reward 0
    for a reason that has nothing to do with the agent — the failure mode that
    cost the A1 sweep two hours and read as a model collapse.

    This runs the trials' own two setup commands, not an approximation of them,
    so a pass here means the real thing works.
    """
    probe = container_scoring_probe()
    if probe is None:
        print("  container scoring probe: could not run (skipping check)")
        return
    trust_store, verifier_deps = probe
    print(f"  container trust store: {trust_store}")
    print(f"  container {'/'.join(VERIFIER_DEPS)}: {verifier_deps}")
    if verifier_deps in ("present", "installed"):
        return
    raise ContainerEgressError(
        f"a fresh {CANARY_IMAGE} container cannot install "
        f"{'/'.join(VERIFIER_DEPS)} (trust store: {trust_store}).\n"
        "  81 of 89 Terminal-Bench verifiers apt-get curl before they can run "
        "any test, so they will score 0 for a reason that is not the agent's.\n"
        "  Check egress from containers: `docker run --rm ubuntu:24.04 sh -c "
        "'apt-get update'`, and the proxy in ~/.docker/config.json."
    )


def scored_tasks(job_dir: Path) -> set[str]:
    """Bare task names in ``job_dir`` that produced a trustworthy score.

    "Trustworthy" is narrower than "finished" but wider than "passed".

    A launch failure, a crash, or a verifier that could not install its own
    tests is work still owed: the reward of 0 was never a measurement of the
    agent, and re-running is the only way to find out what the agent would have
    done.

    A timeout is the opposite. Exhausting the task's own ``timeout_sec`` is a
    real Terminal-Bench outcome that scores zero, and it is the same outcome
    every other agent on the leaderboard is subject to. Re-running only the
    trials that failed — and keeping whichever result comes back better — is
    resampling failures, which inflates the score by construction. So a timeout
    counts as done, and the zero stands.

    ``AGENT_FAULT_EXCEPTIONS`` is therefore a list of outcomes attributable to
    the agent rather than to the harness around it.
    """
    done: set[str] = set()
    for result_path in sorted(job_dir.glob("*/result.json")):
        try:
            data = json.loads(result_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        exception = data.get("exception_info") or {}
        if exception and exception.get("exception_type") not in AGENT_FAULT_EXCEPTIONS:
            continue
        rewards = (data.get("verifier_result") or {}).get("rewards") or {}
        if not isinstance(rewards.get("reward"), (int, float)):
            continue
        if verifier_broken(result_path.parent):
            continue
        task = str(data.get("task_name", ""))
        if task:
            done.add(task.rsplit("/", 1)[-1])
    return done


def resume_exclusions(args: argparse.Namespace) -> list[str]:
    """Task names to skip because a previous job already scored them.

    A sweep is 89 independent trials but costs two hours and real money as a
    unit, so losing it to an infrastructure fault partway through means paying
    twice for the tasks that already worked. Harbor retries within a run
    (``--n-retries``) but has no concept of resuming one, so resumption is
    expressed the only way it can be: exclude what is already done and write
    the remainder to a new job directory.

    Splitting across two directories costs nothing at read time — summarize.py
    groups by condition and manifest hash, not by job — so a resumed sweep adds
    up to one row exactly as an uninterrupted one would. That only holds while
    the manifest is unchanged, which is also precisely when resuming is
    legitimate: edit a persona and the earlier trials are a different
    condition, and the hash will say so rather than silently pooling them.
    """
    if not args.resume_from:
        return []
    job_dir = args.jobs_dir / args.resume_from
    if not job_dir.is_dir():
        raise DatasetError(f"--resume-from: no such job directory: {job_dir}")
    done = scored_tasks(job_dir)
    if not done:
        print(f"  resume: {args.resume_from} scored nothing re-usable; running all tasks")
        return []
    print(f"  resume: skipping {len(done)} task(s) already scored in {args.resume_from}")
    return sorted(done)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    problems = parser.add_mutually_exclusive_group()
    problems.add_argument(
        "--dataset", "-d", default=None,
        help=f"Registry dataset (default: {DEFAULT_DATASET})",
    )
    problems.add_argument(
        "--path", "-p", type=Path, help="Local task or dataset directory"
    )
    parser.add_argument(
        "--include-task", "-i", action="append", default=[],
        help="Task name to include (glob, repeatable)",
    )
    parser.add_argument(
        "--exclude-task", "-x", action="append", default=[],
        help="Task name to exclude (glob, repeatable)",
    )
    parser.add_argument(
        "--attempts", "-k", type=int, default=DEFAULT_ATTEMPTS,
        help=f"Runs per problem (default: {DEFAULT_ATTEMPTS}, the leaderboard requirement)",
    )
    parser.add_argument(
        "--manifest", type=Path, default=DEFAULT_MANIFEST,
        help=f"Team manifest YAML (default: {DEFAULT_MANIFEST.name})",
    )
    parser.add_argument(
        "--endpoint-config", type=Path, default=DEFAULT_ENDPOINTS,
        help=f"Endpoint provider/API-key mapping (default: {DEFAULT_ENDPOINTS.name})",
    )
    parser.add_argument("--n-concurrent", "-n", type=int, default=4, help="Concurrent trials")
    parser.add_argument(
        "--timeout-multiplier", type=float, default=DEFAULT_TIMEOUT_MULTIPLIER,
        metavar="N",
        help="Scale every Harbor phase deadline (agent, verifier, setup, build) "
             f"by N (default: {DEFAULT_TIMEOUT_MULTIPLIER}). Pass 1.0 for a run "
             "that can be submitted to the leaderboard; anything else makes the "
             "job local-only.",
    )
    parser.add_argument(
        "--jobs-dir", type=Path, default=PACKAGE_ROOT / "jobs", help="Job output root"
    )
    parser.add_argument("--job-name", default=None, help="Job name (default: lb-<condition>-<UTC>)")
    parser.add_argument(
        "--resume-from", default=None, metavar="JOB",
        help="Skip tasks a previous job already scored, and run only the rest. "
             "Only trials whose verifier actually ran count as done. Use the "
             "same manifest, or the two halves will not group together.",
    )
    parser.add_argument(
        "--upload", action="store_true", help="Upload to Harbor Hub when the job finishes"
    )
    parser.add_argument(
        "--gui", action="store_true",
        help="Open the Buzz desktop app as the benchmark user to watch the run live",
    )
    parser.add_argument(
        "--fresh", action="store_true",
        help="Reset first: drop the stack's Docker volumes and the benchmark "
             "GUI's app state (keys in state.json are kept)",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Print the underlying harbor command and exit (no stack bring-up)",
    )
    return parser.parse_args(argv)


# -- state: secrets and identities, generated once --------------------------


def load_state() -> dict[str, str]:
    """Generate-or-load the benchmark environment's keys and secrets."""
    from harbor_buzz_testbed.keys import generate_keypair, keypair_from_secret

    STATE_DIR.mkdir(mode=0o700, exist_ok=True)
    state_path = STATE_DIR / "state.json"
    if state_path.is_file():
        state = json.loads(state_path.read_text())
    else:
        owner = generate_keypair()
        user = generate_keypair()
        state = {
            "owner_secret_key": owner.secret_key,
            "user_secret_key": user.secret_key,
            "postgres_password": secrets.token_urlsafe(24),
            "redis_password": secrets.token_urlsafe(24),
            "s3_access_key": secrets.token_hex(10),
            "s3_secret_key": secrets.token_hex(20),
            "git_hook_hmac_secret": secrets.token_hex(32),
            "relay_private_key": generate_keypair().secret_key,
        }
        state_path.touch(mode=0o600)
        state_path.write_text(json.dumps(state, indent=2))
    state["owner_pubkey"] = keypair_from_secret(state["owner_secret_key"]).pubkey
    state["user_pubkey"] = keypair_from_secret(state["user_secret_key"]).pubkey
    return state


def print_user_identity(state: dict[str, str], *, show_secret: bool = False) -> None:
    """Show the pinned benchmark user's identity.

    The private half is only useful for importing into the desktop GUI's
    onboarding, so it is printed only when the GUI was actually asked for.
    The key guards nothing outside this local stack, but every run's stdout
    is captured to a log that outlives the run, and a private key that is
    printed unconditionally is one that eventually gets pasted somewhere it
    shouldn't be.
    """
    print(f"benchmark user pubkey: {state['user_pubkey']}")
    if not show_secret:
        return
    from harbor_buzz_testbed.keys import encode_nsec

    print(
        f"benchmark user nsec:   {encode_nsec(state['user_secret_key'])} "
        "(import this in the GUI onboarding to watch as the benchmark user)"
    )


def write_env_file(state: dict[str, str]) -> Path:
    """Compose interpolation env — regenerated from state on every run."""
    env_path = STATE_DIR / ".env"
    lines = {
        "BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/block/buzz:main"),
        "BUZZ_DOMAIN": "localhost",
        "RELAY_URL": f"ws://localhost:{RELAY_HTTP_PORT}",
        "BUZZ_MEDIA_BASE_URL": f"http://localhost:{RELAY_HTTP_PORT}/media",
        "BUZZ_MEDIA_SERVER_DOMAIN": "localhost",
        "BUZZ_CORS_ORIGINS": f"http://localhost:{RELAY_HTTP_PORT}",
        "BUZZ_REQUIRE_AUTH_TOKEN": "true",
        "BUZZ_REQUIRE_RELAY_MEMBERSHIP": "true",
        "BUZZ_ALLOW_NIP_OA_AUTH": "true",
        "BUZZ_AUTO_MIGRATE": "true",
        "BUZZ_GIT_CONFORMANCE_PROBE": "true",
        "RUST_LOG": "buzz_relay=info,buzz_db=info,buzz_auth=info",
        "RELAY_OWNER_PUBKEY": state["owner_pubkey"],
        "BUZZ_RELAY_PRIVATE_KEY": state["relay_private_key"],
        "BUZZ_GIT_HOOK_HMAC_SECRET": state["git_hook_hmac_secret"],
        "POSTGRES_DB": "buzz",
        "POSTGRES_USER": "buzz",
        "POSTGRES_PASSWORD": state["postgres_password"],
        "REDIS_PASSWORD": state["redis_password"],
        "BUZZ_S3_ACCESS_KEY": state["s3_access_key"],
        "BUZZ_S3_SECRET_KEY": state["s3_secret_key"],
        "BUZZ_S3_BUCKET": "buzz-media",
        "BUZZ_HTTP_PORT": str(RELAY_HTTP_PORT),
        "BUZZ_PG_HOST_PORT": str(PG_HOST_PORT),
        "BUZZ_METRICS_HOST_PORT": str(METRICS_HOST_PORT),
    }
    env_path.touch(mode=0o600)
    env_path.write_text("".join(f"{k}={v}\n" for k, v in lines.items()))
    return env_path


def postgres_dsn(state: dict[str, str]) -> str:
    return (
        f"postgresql://buzz:{state['postgres_password']}"
        f"@127.0.0.1:{PG_HOST_PORT}/buzz"
    )


def trial_timeout_seconds(manifest: Path) -> int:
    """The manifest's per-trial wall-clock budget, for lifetime estimates only."""
    try:
        import yaml

        return int(yaml.safe_load(manifest.read_text())["trial_budget"]["timeout_seconds"])
    except (OSError, ValueError, KeyError, TypeError):
        # The manifest is validated properly downstream; a bad one must fail
        # there with a real message, not here inside a token warning.
        return 900


def environment_override_argv(manifest: Path) -> list[str]:
    """Forward the manifest's container-resource block to Harbor as flags.

    Harbor takes these on the command line, but the study needs them pinned in
    the manifest: they change what a condition *is*, so a run at 8 GB and a run
    at 2 GB must not share a hash and pool into one number. Reading them back
    out here is the seam between the two.

    Only keys the manifest actually sets are forwarded. An absent block emits
    nothing, so Harbor keeps honouring each task's own declared request and
    manifests written before this existed behave exactly as they did.
    """
    try:
        import yaml

        block = yaml.safe_load(manifest.read_text()).get("environment") or {}
    except (OSError, ValueError, AttributeError):
        # Same contract as trial_timeout_seconds: a malformed manifest must
        # fail downstream with a real validation message, not here.
        return []
    argv: list[str] = []
    for key, flag in (
        ("override_cpus", "--override-cpus"),
        ("override_memory_mb", "--override-memory-mb"),
        ("override_storage_mb", "--override-storage-mb"),
    ):
        value = block.get(key)
        if value is not None:
            argv += [flag, str(value)]
    return argv


def check_budget_clears_clock(manifest: Path, multiplier: float) -> None:
    """Refuse a multiplier the harness's own ceiling would silently undo.

    Two deadlines bound a trial. Harbor enforces each task's `timeout_sec`,
    scaled by the multiplier. The harness separately wraps the wait for `DONE:`
    in the manifest's `trial_budget.timeout_seconds`, and passes that same value
    to buzz-acp as its turn cap. Whichever is smaller is the real deadline.

    Raising only the multiplier therefore buys nothing on the tasks that need it
    most: the 12000s task at 3x gets 36000s from Harbor and is still killed by us
    at 12000s. Worse, it fails as a timeout — indistinguishable from the agent
    genuinely running out of clock, which is exactly the confusion the multiplier
    was raised to remove.

    So this is an error rather than a warning: the two numbers are one setting
    expressed in two files, and a run where they disagree measures neither.
    """
    if multiplier <= 0:
        raise DatasetError("--timeout-multiplier must be positive")
    required = int(MAX_TASK_TIMEOUT_SECONDS * multiplier)
    budget = trial_timeout_seconds(manifest)
    if budget < required:
        raise DatasetError(
            f"--timeout-multiplier {multiplier} gives Harbor's longest task "
            f"{required}s, but {manifest.name} caps every trial at "
            f"{budget}s (trial_budget.timeout_seconds), so the harness would cut "
            f"it short and record a timeout. Raise trial_budget.timeout_seconds "
            f"to at least {required} in that manifest, or lower the multiplier."
        )


def databricks_oauth_token() -> tuple[str, int] | None:
    """The bearer and expiry from buzz-agent's own OAuth cache, if it has one.

    A task container has no PKCE cache and must not open a browser, so the
    Databricks bearer has to be a plain string in the environment. Rather than
    ask for one to be pasted, reuse the token `buzz-agent` already minted on
    this host — same workspace, same scopes.
    """
    cache = Path.home() / ".config" / "buzz-agent" / "oauth" / "databricks"
    files = sorted(cache.glob("*.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    for path in files:
        try:
            payload = json.loads(path.read_text())
            return payload["access_token"], int(payload["expires_at"])
        except (OSError, ValueError, KeyError, TypeError):
            continue
    return None


def resolve_databricks_token(needed_seconds: int) -> None:
    """Populate DATABRICKS_TOKEN from the OAuth cache and vet its lifetime.

    Expiry is a first-class preflight concern, not a detail: these are
    short-lived OAuth bearers, and one that dies mid-sweep does not look like
    an auth failure. It looks like a sudden cluster of model errors across
    every remaining trial — the most expensive kind of bad data, because the
    runs still complete and still cost money.
    """
    if os.environ.get("DATABRICKS_TOKEN"):
        print("DATABRICKS_TOKEN: using the value already in the environment")
        return
    found = databricks_oauth_token()
    if found is None:
        return  # write_provisioner_config raises the actionable error.
    token, expires_at = found
    remaining = expires_at - int(time.time())
    if remaining <= 0:
        raise SystemExit(
            "the cached Databricks OAuth token expired "
            f"{-remaining // 60} min ago — re-authenticate buzz-agent, or "
            "export a long-lived DATABRICKS_TOKEN"
        )
    os.environ["DATABRICKS_TOKEN"] = token
    print(f"DATABRICKS_TOKEN: from buzz-agent's OAuth cache, {remaining // 60} min left")
    if remaining < needed_seconds:
        print(
            f"  WARNING: this run needs at least {needed_seconds // 60} min "
            "(one task's attempts at the manifest trial timeout) and the real "
            "sweep is longer. The token will expire mid-run, and every trial "
            "after that point fails at the gateway while still consuming its "
            "timeout. Export a long-lived DATABRICKS_TOKEN for real sweeps.",
            file=sys.stderr,
        )


def required_key_envs(endpoint_config: Path) -> set[str]:
    """The API-key environment variable names this endpoint config asks for."""
    try:
        endpoints = json.loads(endpoint_config.read_text())
    except (OSError, json.JSONDecodeError):
        return set()  # write_provisioner_config raises the actionable error.
    return {
        entry["api_key_env"]
        for entry in endpoints.values()
        if isinstance(entry, dict) and "api_key_env" in entry
    }


def resolve_openai_key() -> None:
    """Accept the conventional ``OPENAI_API_KEY`` for an OpenAI endpoint.

    buzz-agent reads ``OPENAI_COMPAT_API_KEY`` — its OpenAI path is a
    generic OpenAI-*compatible* client, so the variable is deliberately not the
    vendor's name. But ``OPENAI_API_KEY`` is what is actually exported in a
    shell profile, and requiring the operator to re-export it under a second
    name is a preflight failure waiting to happen for no benefit.
    """
    if os.environ.get("OPENAI_COMPAT_API_KEY"):
        print("OPENAI_COMPAT_API_KEY: using the value already in the environment")
        return
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        return  # write_provisioner_config raises the actionable error.
    os.environ["OPENAI_COMPAT_API_KEY"] = key
    print("OPENAI_COMPAT_API_KEY: taken from OPENAI_API_KEY")


def write_provisioner_config(
    state: dict[str, str], endpoint_config: Path
) -> Path:
    """Resolve per-endpoint API keys from the environment and write the
    provisioner config: pinned user, keep-channels teardown."""
    endpoints = json.loads(endpoint_config.read_text())
    llm_api_keys: dict[str, str] = {}
    for name, entry in endpoints.items():
        env_var = entry["api_key_env"]
        key = os.environ.get(env_var)
        if not key:
            raise SystemExit(
                f"endpoint {name!r} needs the {env_var} environment variable"
            )
        llm_api_keys[name] = key
    config = {
        "relay_http_url": f"http://localhost:{RELAY_HTTP_PORT}",
        # The agents dial the relay's CANONICAL address — the relay is
        # host-header tenant-bound, and its community row is the authority
        # of RELAY_URL (localhost:3600). Inside the task container that
        # loopback address is served by a tiny forwarder bridging to the
        # Docker host gateway (see --relay-gateway below).
        "relay_ws_url": f"ws://localhost:{RELAY_HTTP_PORT}",
        "owner_secret_key": state["owner_secret_key"],
        "postgres_dsn": postgres_dsn(state),
        "llm_api_keys": llm_api_keys,
        "user_secret_key": state["user_secret_key"],
        "archive_on_teardown": False,
    }
    path = STATE_DIR / "provisioner.json"
    path.touch(mode=0o600)
    path.write_text(json.dumps(config, indent=2))
    return path


# -- docker stack ------------------------------------------------------------


def compose_command(*args: str) -> list[str]:
    command = [
        "docker", "compose",
        "--project-name", COMPOSE_PROJECT,
        "--project-directory", str(STATE_DIR),
        "--env-file", str(STATE_DIR / ".env"),
    ]
    for file in COMPOSE_FILES:
        command += ["-f", str(file)]
    return command + list(args)


def stale_credential_volume(state: dict[str, str]) -> bool:
    """True when Postgres is up but rejects THIS clone's password — the
    volume was initialized by another checkout's ``.benchmark/`` state
    (compose project name is machine-global, state dir is per-clone)."""
    import psycopg

    try:
        psycopg.connect(postgres_dsn(state), connect_timeout=5).close()
    except psycopg.OperationalError as error:
        return "password authentication failed" in str(error)
    return False


def bring_up_stack(state: dict[str, str]) -> None:
    """Compose bring-up (idempotent), self-healing the one known-fatal
    failure: a stale Postgres volume from a different clone. Nothing in
    that volume is usable (we can't even authenticate to it), so drop the
    volumes and retry once rather than aborting with instructions."""
    try:
        subprocess.run(compose_command("up", "-d", "--wait"), check=True)
    except subprocess.CalledProcessError:
        if not stale_credential_volume(state):
            raise
        print(
            "benchmark Postgres volume was initialized by a different "
            "checkout's .benchmark/ state — dropping the stale volumes and "
            "retrying..."
        )
        subprocess.run(compose_command("down", "-v"), check=True)
        subprocess.run(compose_command("up", "-d", "--wait"), check=True)


def reset_environment() -> None:
    """--fresh: drop the stack's Docker volumes and the benchmark GUI's
    app state, together — GUI records (workspaces, read state) only stay
    coherent as long as the database they reference exists. Keys in
    ``state.json`` are kept, so the same nsec works after the reset."""
    subprocess.run(compose_command("down", "-v"), check=True)
    if sys.platform == "darwin":
        for domain in ("WebKit", "Caches", "Application Support"):
            shutil.rmtree(
                Path.home() / "Library" / domain / GUI_BUNDLE_IDENTIFIER,
                ignore_errors=True,
            )


def ensure_stack(state: dict[str, str]) -> None:
    """Bring the compose stack up (idempotent) and apply the benchmark schema."""
    import psycopg

    bring_up_stack(state)

    deadline = time.monotonic() + 60
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with psycopg.connect(postgres_dsn(state)) as conn:
                conn.execute(SCHEMA_SQL.read_text())
                conn.commit()
            return
        except psycopg.Error as error:  # containers healthy but PG settling
            last_error = error
            time.sleep(2)
    raise SystemExit(f"benchmark schema apply failed: {last_error}")


# -- buzz binaries -----------------------------------------------------------


def ensure_binaries() -> dict[str, Path]:
    """Find the host buzz CLI, building it once if missing."""
    try:
        return run_leaderboard.find_binaries(None)
    except SystemExit:
        print("host buzz CLI missing — building (cargo build, first run only)...")
    cargo = REPO_ROOT / "bin" / "cargo"
    subprocess.run(
        [str(cargo), "build", "-p", "buzz-cli"],
        cwd=REPO_ROOT,
        check=True,
    )
    return run_leaderboard.find_binaries(None)


def linux_triple() -> str:
    """The musl triple matching the Docker engine that runs task containers."""
    arch = subprocess.run(
        ["docker", "version", "--format", "{{.Server.Arch}}"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    try:
        return {
            "arm64": "aarch64-unknown-linux-musl",
            "amd64": "x86_64-unknown-linux-musl",
        }[arch]
    except KeyError:
        raise SystemExit(f"unsupported Docker architecture: {arch!r}") from None


def ensure_agent_binaries() -> Path:
    """Cross-build the static Linux agent stack once, cached in .benchmark/.

    The agents run *inside* each Harbor task container as the real
    buzz-acp → buzz-agent → buzz-dev-mcp stack, so the binaries must be
    Linux ELF for the task image architecture. musl-static means they run
    on any Linux base image (glibc or not). The relay loopback forwarder
    is compiled in the same step with plain rustc (std-only, no deps).
    """
    triple = linux_triple()
    bin_dir = LINUX_TARGET_DIR / triple / "release"
    targets = AGENT_BINARIES + (FORWARDER_BINARY,)
    if all((bin_dir / name).is_file() for name in targets):
        return bin_dir
    print(f"Linux agent binaries missing — cross-building for {triple} "
          f"in {RUST_IMAGE} (first run only, ~2 min)...")
    LINUX_TARGET_DIR.mkdir(parents=True, exist_ok=True)
    (STATE_DIR / "cargo-registry").mkdir(exist_ok=True)
    packages = [arg for name in AGENT_BINARIES for arg in ("-p", name)]
    forwarder_src = FORWARDER_SOURCE.relative_to(REPO_ROOT)
    subprocess.run(
        [
            "docker", "run", "--rm",
            "-v", f"{REPO_ROOT}:/src:ro",
            "-v", f"{LINUX_TARGET_DIR}:/target",
            "-v", f"{STATE_DIR / 'cargo-registry'}:/usr/local/cargo/registry",
            "-e", "CARGO_TARGET_DIR=/target",
            "-w", "/src",
            RUST_IMAGE,
            "sh", "-c",
            "apk add --no-cache musl-dev >/dev/null && "
            f"cargo build --release --locked --target {triple} "
            + " ".join(packages)
            + f" && rustc --edition 2021 -O --target {triple}"
            f" -o /target/{triple}/release/{FORWARDER_BINARY}"
            f" /src/{forwarder_src}",
        ],
        check=True,
    )
    missing = [n for n in targets if not (bin_dir / n).is_file()]
    if missing:
        raise SystemExit(f"cross-build produced no {', '.join(missing)} in {bin_dir}")
    return bin_dir


# -- GUI ---------------------------------------------------------------------


def launch_gui(state: dict[str, str]) -> subprocess.Popen:
    """Open the Buzz desktop app logged in as the benchmark user.

    The relay runs closed (membership required), so the user pubkey is first
    added to the relay membership list via buzz-admin inside the container —
    NIP-OA auth tags cover the agents, but the GUI authenticates as a plain
    member, exactly like a human.
    """
    subprocess.run(
        compose_command(
            "exec", "-T", "relay",
            "buzz-admin", "add-member", "--pubkey", state["user_pubkey"],
        ),
        check=True,
    )

    desktop_dir = REPO_ROOT / "desktop"
    if not (desktop_dir / "node_modules").is_dir():
        subprocess.run(["pnpm", "install"], cwd=desktop_dir, check=True)

    # tauri dev needs sidecar files present; stub them and drop in the real
    # CLI binary (mirrors the just staging recipe).
    target = subprocess.run(
        ["rustc", "-vV"], capture_output=True, text=True, check=True
    ).stdout
    triple = next(
        line.split(": ", 1)[1] for line in target.splitlines() if line.startswith("host: ")
    )
    sidecar_dir = desktop_dir / "src-tauri" / "binaries"
    sidecar_dir.mkdir(parents=True, exist_ok=True)
    binaries = ensure_binaries()
    for name in ("buzz-acp", "buzz-agent", "buzz-dev-mcp", "git-credential-nostr", "buzz"):
        stub = sidecar_dir / f"{name}-{triple}"
        if not stub.exists():
            stub.touch()
    real_cli = sidecar_dir / f"buzz-{triple}"
    real_cli.write_bytes(binaries["buzz"].read_bytes())
    real_cli.chmod(0o755)

    print(
        f"Opening Buzz GUI as the benchmark user ({state['user_pubkey'][:16]}…).\n"
        "Watch, don't type — a message from you mid-trial would taint the run."
    )
    # Distinct bundle identifier: the desktop app persists workspaces (incl.
    # their relay URLs) in per-identifier WebKit localStorage, and a stored
    # workspace's relay URL overrides BUZZ_RELAY_URL by design. Reusing the
    # default identifier means any past local-dev session's ws://localhost:3000
    # workspace silently shadows the benchmark relay. An identifier of our own
    # keeps that state isolated both ways.
    tauri_config = json.dumps(
        {"identifier": GUI_BUNDLE_IDENTIFIER, "productName": "Buzz Benchmark"}
    )
    return subprocess.Popen(
        ["pnpm", "exec", "tauri", "dev", "--config", tauri_config],
        cwd=desktop_dir,
        env={
            **os.environ,
            "BUZZ_RELAY_URL": f"ws://localhost:{RELAY_HTTP_PORT}",
            "BUZZ_PRIVATE_KEY": state["user_secret_key"],
        },
    )


# -- dataset preflight ---------------------------------------------------------


class DatasetError(RuntimeError):
    """Raised when the requested dataset cannot be resolved in the registry."""


def registry_datasets(timeout: float = 20.0) -> set[str] | None:
    """Return every ``name@version`` in Harbor's registry, or None if offline.

    Distinguishing "the registry says no" from "we could not reach the registry"
    matters: the first is a typo the operator must fix, the second is a network
    problem that the rest of preflight is already responsible for reporting.
    """
    import urllib.error
    import urllib.request

    from harbor.constants import DEFAULT_REGISTRY_URL

    url = os.environ.get("HARBOR_REGISTRY_URL", DEFAULT_REGISTRY_URL)
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            entries = json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, OSError):
        return None
    if not isinstance(entries, list):
        return None
    return {
        f"{entry['name']}@{entry['version']}"
        for entry in entries
        if isinstance(entry, dict) and "name" in entry and "version" in entry
    }


def package_dataset_tasks(dataset: str, timeout: float = 90.0) -> int | None:
    """Task count for an ``org/name[@ref]`` dataset, or None if unresolvable."""
    import asyncio

    from harbor.registry.client.package import PackageDatasetClient

    async def resolve() -> int:
        metadata = await PackageDatasetClient()._get_dataset_metadata(dataset)
        return len(metadata.task_ids)

    try:
        return asyncio.run(asyncio.wait_for(resolve(), timeout))
    except Exception:  # noqa: BLE001 — unreachable and unpublished look alike here
        return None


def validate_dataset(dataset: str) -> None:
    """Fail fast on a dataset string Harbor cannot resolve.

    Cheap to run and it forecloses a whole class of wasted sweeps: the dataset
    is only dereferenced once trials start, so a bad name otherwise surfaces
    after the Docker stack, the relay, and the cross-compiled binaries are all
    up.

    The two forms resolve from different registries, so they are checked
    differently — an ``org/name`` package reference is not in registry.json and
    a registry.json name is not a package.
    """
    if "/" in dataset:
        tasks = package_dataset_tasks(dataset)
        if tasks is None:
            raise DatasetError(
                f"dataset package {dataset!r} did not resolve. Check the org and "
                "name, that the ref exists (default 'latest'), and that the hub "
                "registry is reachable"
            )
        print(f"  dataset {dataset}: {tasks} tasks", file=sys.stderr)
        return

    if "@" not in dataset:
        raise DatasetError(
            f"dataset {dataset!r} is missing a version; a registry.json dataset "
            "is 'name@version' (e.g. 'terminal-bench@2.0'), and a hub package is "
            "'org/name' (e.g. 'terminal-bench/terminal-bench-2-1')"
        )
    available = registry_datasets()
    if available is None:
        print(
            f"  warning: cannot reach the Harbor registry; {dataset!r} unverified",
            file=sys.stderr,
        )
        return
    if dataset in available:
        return
    name = dataset.split("@", 1)[0]
    versions = sorted(entry for entry in available if entry.startswith(f"{name}@"))
    hint = (
        f" available versions of {name!r}: {', '.join(versions)}"
        if versions
        else f" no dataset named {name!r} is published"
    )
    raise DatasetError(f"dataset {dataset!r} is not in the Harbor registry;{hint}")


# -- main ---------------------------------------------------------------------


def qualify_task_pattern(pattern: str, dataset: str) -> str:
    """Let ``-i openssl-selfsigned-cert`` mean what the operator obviously meant.

    Harbor matches task filters against ``PackageTaskId.get_name()``, which for
    a package dataset is ``org/name`` — so a bare task name matches nothing and
    the run dies with "No tasks matched", after the Docker stack is already up.
    The name printed by every listing, and written in this repo's docs, is the
    bare one. Qualify it here rather than making every caller remember.

    Only bare patterns are touched: one that already contains ``/`` is the
    operator being explicit, and one for a ``--path`` dataset is matched against
    a bare name already.
    """
    if "/" in pattern or "/" not in dataset:
        return pattern
    return f"*/{pattern}"


def leaderboard_argv(
    args: argparse.Namespace, provisioner_config: Path, agent_bin_dir: Path
) -> list[str]:
    argv: list[str] = []
    dataset = "" if args.path else (args.dataset or DEFAULT_DATASET)
    if args.path:
        argv += ["--path", str(args.path)]
    else:
        argv += ["--dataset", dataset]
    for pattern in args.include_task:
        argv += ["--include-task", qualify_task_pattern(pattern, dataset)]
    for pattern in list(args.exclude_task) + resume_exclusions(args):
        argv += ["--exclude-task", qualify_task_pattern(pattern, dataset)]
    argv += [
        "--attempts", str(args.attempts),
        "--manifest", str(args.manifest),
        "--endpoint-config", str(args.endpoint_config),
        "--provisioner-config", str(provisioner_config),
        "--agent-bin-dir", str(agent_bin_dir),
        # The relay as reachable from inside a task container: Docker's
        # host alias, bridged to the canonical localhost address by the
        # uploaded forwarder. Override the alias with
        # BUZZ_BENCHMARK_DOCKER_HOST if your engine exposes the host
        # differently.
        "--relay-gateway",
        f"{os.environ.get('BUZZ_BENCHMARK_DOCKER_HOST', 'host.docker.internal')}"
        f":{RELAY_HTTP_PORT}",
        "--n-concurrent", str(args.n_concurrent),
        "--jobs-dir", str(args.jobs_dir),
    ]
    argv += environment_override_argv(args.manifest)
    if args.timeout_multiplier != 1.0:
        # Forwarded only when it changes something: Harbor's validator rejects
        # the flag being *set*, not just being non-unity, so a submittable run
        # must not carry it at all.
        argv += ["--timeout-multiplier", str(args.timeout_multiplier)]
    if args.job_name:
        argv += ["--job-name", args.job_name]
    if args.upload:
        argv.append("--upload")
    if args.dry_run:
        argv.append("--dry-run")
    return argv


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    # Before anything expensive: a local --path needs no registry, a registry
    # dataset must resolve.
    if not args.path:
        try:
            validate_dataset(args.dataset or DEFAULT_DATASET)
        except DatasetError as error:
            print(f"error: {error}", file=sys.stderr)
            return 2
    # Checked here rather than only where it is used, so a typo fails before the
    # stack comes up instead of after. resume_exclusions() re-raises as a
    # backstop; this is the path that reports it.
    if args.resume_from and not (args.jobs_dir / args.resume_from).is_dir():
        print(
            f"error: --resume-from: no such job directory: "
            f"{args.jobs_dir / args.resume_from}",
            file=sys.stderr,
        )
        return 2
    # Both deadlines are one setting in two files; catch a disagreement here
    # rather than as 89 unexplained timeouts. Only for the registry dataset:
    # MAX_TASK_TIMEOUT_SECONDS describes Terminal-Bench 2.1, and a local --path
    # task (the M1 smoke) sets its own, much smaller, budget.
    if not args.path:
        try:
            check_budget_clears_clock(args.manifest, args.timeout_multiplier)
        except DatasetError as error:
            print(f"error: {error}", file=sys.stderr)
            return 2
    if args.timeout_multiplier != 1.0:
        print(
            f"timeout multiplier: {args.timeout_multiplier}x on every Harbor "
            "phase — scores are local-only (not leaderboard-submittable), and "
            "time is reported rather than enforced"
        )
    # Before the stack, the binaries, and the money: a sweep run through a TLS
    # interception proxy cannot be scored at all. Skipped for --dry-run, which
    # prints a command and touches nothing — refusing there would block the one
    # invocation that is safe to make while the proxy is still connected.
    if not args.dry_run:
        try:
            check_tls_not_intercepted()
        except InterceptedTLSError as error:
            print(f"error: {error}", file=sys.stderr)
            return 2
        # The host reaching PyPI does not mean a container can. Egress from
        # inside is a separate path -- the docker proxy, the image's trust
        # store, apt -- and it is the one every verifier actually depends on.
        try:
            check_container_can_be_scored()
        except ContainerEgressError as error:
            print(f"error: {error}", file=sys.stderr)
            return 2
    state = load_state()
    print_user_identity(state, show_secret=args.gui)
    write_env_file(state)
    # Resolve only the credentials this endpoint config actually asks for. A
    # run on OpenAI must not die because a Databricks OAuth token it will never
    # send happens to have expired.
    needed = required_key_envs(args.endpoint_config)
    if "DATABRICKS_TOKEN" in needed:
        resolve_databricks_token(trial_timeout_seconds(args.manifest) * args.attempts)
    if "OPENAI_COMPAT_API_KEY" in needed:
        resolve_openai_key()
    provisioner_config = write_provisioner_config(state, args.endpoint_config)

    if args.dry_run:
        agent_bin_dir = LINUX_TARGET_DIR / linux_triple() / "release"
    else:
        ensure_binaries()
        agent_bin_dir = ensure_agent_binaries()
        if args.fresh:
            reset_environment()
        ensure_stack(state)
        if args.gui:
            launch_gui(state)

    return run_leaderboard.main(
        leaderboard_argv(args, provisioner_config, agent_bin_dir)
    )


if __name__ == "__main__":
    sys.exit(main())
