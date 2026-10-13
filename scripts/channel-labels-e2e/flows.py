"""Exercise the built CLI through two actual relay processes and a fault proxy."""
import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import urllib.request

ROOT = Path(os.environ["LABEL_EVIDENCE"])
BINARY = Path(os.environ["LABEL_CLI"])
CONFIG = json.loads((ROOT / "config.json").read_text())
OWNER = json.loads((ROOT / "owner-key.json").read_text())
RELAY = json.loads((ROOT / "relay-key.json").read_text())
CHANNEL = json.loads((ROOT / "CREATE.stdout").read_text())["channel_id"]
ENV = {key: value for key, value in os.environ.items() if not key.startswith(("BUZZ_", "RELAY_"))}
ENV.update(BUZZ_RELAY_URL=CONFIG["url"], BUZZ_PRIVATE_KEY=OWNER["Secret key"])
TRUST = ["--trusted-relay", RELAY["Public key"]]


def cli(name, args, expected=0, identity=None):
    env = ENV.copy()
    if identity:
        env["BUZZ_PRIVATE_KEY"] = json.loads((ROOT / (identity + "-key.json")).read_text())["Secret key"]
    result = subprocess.run([str(BINARY)] + args, env=env, capture_output=True, text=True, timeout=45)
    (ROOT / (name + ".stdout")).write_text(result.stdout)
    (ROOT / (name + ".stderr")).write_text(result.stderr)
    assert result.returncode == expected, (name, result.returncode, result.stdout, result.stderr)
    print("PASS", name, "exit", result.returncode, flush=True)
    return json.loads(result.stdout) if result.stdout else None


def get(name):
    return cli(name, ["channels", "labels", "get", "--channel", CHANNEL] + TRUST)[0]


def update(name, add=(), remove=(), expected=0, identity=None):
    args = ["channels", "labels", "update", "--channel", CHANNEL,
            "--command-file", str(ROOT / (name + ".jsonl"))] + TRUST
    for value in add:
        args += ["--add-label", value]
    for value in remove:
        args += ["--remove-label", value]
    return cli(name, args, expected, identity)


assert get("initial")["labels"] == ["team:infra"]
(ROOT / "drop-next-ack").touch()
lost = update("lost-ack", add=["uncertain"], expected=2)
assert lost["outcome"] == "unknown"
assert "uncertain" in get("after-lost-ack")["labels"]
update("opposing", remove=["uncertain"])
before = get("before-retry")
retry = cli("retry-lost", ["channels", "labels", "retry", "--command-file", str(ROOT / "lost-ack.jsonl")])
assert retry["event_id"] == lost["event_id"] and retry["outcome"] == "committed"
after = get("after-retry")
assert after["event"]["id"] == before["event"]["id"] and after["labels"] == ["team:infra"]
update("noop", add=["team:infra"])
assert get("after-noop")["event"]["id"] == after["event"]["id"]
cli("ordinary", ["channels", "topic", "--channel", CHANNEL, "--topic", "ordinary metadata preserves labels"])
assert get("after-ordinary")["labels"] == ["team:infra"]
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
    list(executor.map(lambda name: update(name, add=[name]), ["parallel-a", "parallel-b"]))
assert get("after-parallel")["labels"] == ["parallel-a", "parallel-b", "team:infra"]
found = cli("find", ["channels", "labels", "find", "--label", "team:infra", "--limit", "1"] + TRUST)
assert found[0]["channel_id"] == CHANNEL
update("denied", add=["forbidden"], expected=4, identity="outsider")
assert "forbidden" not in get("after-denied")["labels"]
update("remove-all", remove=["parallel-a", "parallel-b", "team:infra"])
empty = get("empty")
assert empty["labels"] == [] and all(tag[0] not in ["l", "L"] for tag in empty["event"]["tags"])
cli("ordinary-empty", ["channels", "purpose", "--channel", CHANNEL, "--purpose", "empty set remains empty"])
assert get("after-empty-publisher")["labels"] == []
# The proxy must have withheld a committed reply, not failed before sending.
entries = [json.loads(line) for line in (ROOT / "PROXY.jsonl").read_text().splitlines()]
submissions = [entry for entry in entries if entry["event"] == lost["event_id"]]
assert len(submissions) == 2, submissions
assert submissions[0]["dropped"] and submissions[0]["ack"]["accepted"]
assert submissions[1]["ack"]["message"] == "duplicate: nip-cl-committed"
assert submissions[0]["body_sha256"] == submissions[1]["body_sha256"]
assert submissions[0]["auth_sha256"] != submissions[1]["auth_sha256"]
assert {entry["backend"] for entry in entries if entry["path"] == "/events"} == set(CONFIG["backends"])
print("PASS exact signed command recovered; acknowledgement lost after commit; both relay processes exercised", flush=True)
for port in CONFIG["health"]:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/_readiness", timeout=5) as response:
        assert json.load(response)["status"] == "ready"
print("PASS both replicas ready after workflows", flush=True)
