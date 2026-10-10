#!/usr/bin/env python3
"""Native two-replica NIP-CL smoke test; requires an EMPTY isolated database."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[2]


def ports(count):
    sockets = []
    try:
        for _ in range(count):
            sock = socket.socket()
            sock.bind(("127.0.0.1", 0))
            sockets.append(sock)
        return [sock.getsockname()[1] for sock in sockets]
    finally:
        for sock in sockets:
            sock.close()


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/debug")
    parser.add_argument("--confirm-isolated-empty-database", action="store_true", required=True)
    args = parser.parse_args()
    root = args.evidence.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    binary = args.bin_dir.resolve()
    # Refuse a missing destination rather than fall back to a developer database.
    database = os.environ["LABEL_DATABASE_URL"]
    redis = os.environ["LABEL_REDIS_URL"]
    env = {key: value for key, value in os.environ.items() if not key.startswith(
        ("BUZZ_", "RELAY_", "DATABASE_", "REDIS_", "READ_DATABASE_", "OTEL_"))}
    env.update(DATABASE_URL=database, REDIS_URL=redis)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    status = subprocess.check_output(["git", "status", "--short"], cwd=REPO, text=True)
    identity = {"revision": revision, "status": status, "python": sys.version,
                "artifacts": {name: sha256(binary / name) for name in ("buzz", "buzz-relay", "buzz-admin")}}
    (root / "SOURCE.json").write_text(json.dumps(identity, indent=2))

    def run(name, command, command_env=env):
        with (root / (name + ".stdout")).open("wb") as out, (root / (name + ".stderr")).open("wb") as err:
            result = subprocess.run(command, env=command_env, cwd=root, stdout=out, stderr=err, timeout=120)
        if result.returncode:
            raise RuntimeError(f"{name}: exit {result.returncode}; inspect evidence")

    keys = {}
    for name in ("owner", "relay", "outsider"):
        raw = subprocess.check_output([str(binary / "buzz-admin"), "generate-key"], env=env, text=True)
        keys[name] = dict(line.split(":", 1) for line in raw.splitlines() if ":" in line)
        keys[name] = {key.strip(): value.strip() for key, value in keys[name].items()}
        (root / (name + "-key.json")).write_text(json.dumps(keys[name]))
    chosen = ports(7)
    config = {"proxy": chosen[0], "backends": chosen[1:3], "health": chosen[3:5], "metrics": chosen[5:7],
              "url": f"http://127.0.0.1:{chosen[0]}"}
    (root / "config.json").write_text(json.dumps(config))
    env.update(BUZZ_RELAY_PRIVATE_KEY=keys["relay"]["Secret key"],
               RELAY_OWNER_PUBKEY=keys["owner"]["Public key"], RELAY_URL=config["url"],
               BUZZ_NIP_CL_ENABLED="true", BUZZ_NIP_CL_WRITER_CUTOVER="offline-v1",
               BUZZ_REQUIRE_AUTH_TOKEN="false", BUZZ_REQUIRE_RELAY_MEMBERSHIP="false",
               BUZZ_AUTO_MIGRATE="false", BUZZ_MESH="false", BUZZ_GIT_CONFORMANCE_PROBE="false",
               BUZZ_PARTITION_MANAGER_CREATE_ENABLED="false", RUST_LOG="info",
               LABEL_EVIDENCE=str(root), LABEL_CLI=str(binary / "buzz"))
    run("MIGRATE", [str(binary / "buzz-admin"), "migrate"])
    children = []
    logs = []
    try:
        for index in range(2):
            replica_env = dict(env, BUZZ_BIND_ADDR=f"127.0.0.1:{config['backends'][index]}",
                               BUZZ_HEALTH_PORT=str(config["health"][index]),
                               BUZZ_METRICS_PORT=str(config["metrics"][index]),
                               BUZZ_GIT_REPO_PATH=str(root / f"git-{index}"),
                               BUZZ_GIT_PACK_CACHE_PATH=str(root / f"pack-{index}"))
            log = (root / f"RELAY_{index}.log").open("wb")
            logs.append(log)
            children.append(subprocess.Popen([str(binary / "buzz-relay")], env=replica_env,
                                              cwd=root, stdout=log, stderr=subprocess.STDOUT))
        proxy_log = (root / "PROXY_SERVER.log").open("wb")
        logs.append(proxy_log)
        children.append(subprocess.Popen([sys.executable, str(Path(__file__).with_name("proxy.py"))],
                                          env=env, cwd=root, stdout=proxy_log, stderr=subprocess.STDOUT))
        deadline = time.monotonic() + 60
        while True:
            if any(child.poll() is not None for child in children):
                raise RuntimeError("test process exited during readiness; inspect evidence")
            try:
                for port in config["health"]:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/_readiness", timeout=2) as response:
                        assert json.load(response)["status"] == "ready"
                request = urllib.request.Request(config["url"], headers={"Accept": "application/nostr+json"})
                with urllib.request.urlopen(request, timeout=2) as response:
                    info = json.load(response)
                assert "nip-cl" in info["supported_extensions"]
                assert info["self"] == keys["relay"]["Public key"]
                (root / "NIP11.json").write_text(json.dumps(info))
                break
            except (OSError, urllib.error.URLError, AssertionError):
                if time.monotonic() >= deadline:
                    raise RuntimeError("readiness/capability deadline exceeded")
                time.sleep(0.1)
        cli_env = dict(env, BUZZ_RELAY_URL=config["url"], BUZZ_PRIVATE_KEY=keys["owner"]["Secret key"])
        run("CREATE", [str(binary / "buzz"), "channels", "create", "--name", "labels-live",
                       "--type", "stream", "--visibility", "open", "--label", "team:infra",
                       "--command-file", str(root / "create.jsonl"), "--trusted-relay", keys["relay"]["Public key"]], cli_env)
        run("FLOWS", [sys.executable, str(Path(__file__).with_name("flows.py"))])
        print((root / "FLOWS.stdout").read_text())
        print(f"PASS evidence: {root}; database retained for inspection")
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
        for child in children:
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=10)
        for log in logs:
            log.close()
        (root / "TEARDOWN.json").write_text(json.dumps({"processes": [child.returncode for child in children],
                                                       "database_retained": True}))


if __name__ == "__main__":
    main()
