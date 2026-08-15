#!/usr/bin/env python3
"""Exact-SHA-bound native review orchestrator for Buzz Desktop."""
from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import http.server
import json
import os
import pathlib
import re
import secrets
import select
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from typing import Any

try:
    import yaml
except ImportError as exc:  # pragma: no cover - environment preflight
    raise SystemExit("PyYAML is required (activate the repository Hermit environment)") from exc

ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOL_ROOT = pathlib.Path(__file__).resolve().parent
PRODUCTION_BUNDLE_IDS = {"xyz.block.buzz.app", "xyz.block.sprout.app"}
PRODUCTION_KEYRINGS = {"buzz-desktop", "sprout-desktop"}
SECRET_NAME = re.compile(r"(AUTH|TOKEN|SECRET|PASSWORD|PRIVATE_KEY|COOKIE)", re.I)
ALLOWED_TOP = {"schema_version", "flow", "platforms", "fixture", "record", "steps", "cleanup"}
ALLOWED_STEP = {"name", "locate", "act", "expect", "expect_for", "timeout_ms", "measure"}


class HarnessError(RuntimeError):
    pass


def run(command: list[str], *, cwd: pathlib.Path = ROOT, env: dict[str, str] | None = None,
        check: bool = True, capture: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, env=env, check=check, text=True,
                          stdout=subprocess.PIPE if capture else None,
                          stderr=subprocess.PIPE if capture else None)


def git(*args: str) -> str:
    return run(["git", *args]).stdout.strip()


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def validate_locator(locator: Any, where: str) -> None:
    if not isinstance(locator, dict) or not locator or set(locator) - {"id", "role", "name"}:
        raise HarnessError(f"{where}: locator must contain only id, role, and/or name")
    if not all(isinstance(value, str) and value for value in locator.values()):
        raise HarnessError(f"{where}: locator values must be non-empty strings")


def validate_expectation(expectation: Any, where: str) -> None:
    allowed = {"exists", "not_exists", "focused", "enabled", "value"}
    if (
        not isinstance(expectation, dict)
        or len(expectation) != 1
        or set(expectation) - allowed
    ):
        raise HarnessError(f"{where}: expectation must contain exactly one supported condition")
    for key in ("exists", "not_exists"):
        if key in expectation:
            validate_locator(expectation[key], f"{where}.{key}")
    focused = expectation.get("focused")
    if focused is not None and not isinstance(focused, bool):
        validate_locator(focused, f"{where}.focused")
    if "enabled" in expectation and not isinstance(expectation["enabled"], bool):
        raise HarnessError(f"{where}.enabled must be boolean")
    if "value" in expectation and not isinstance(expectation["value"], str):
        raise HarnessError(f"{where}.value must be a string")


def load_journey(path: pathlib.Path) -> dict[str, Any]:
    try:
        journey = yaml.safe_load(path.read_text())
    except (OSError, yaml.YAMLError) as exc:
        raise HarnessError(f"cannot read journey {path}: {exc}") from exc
    if not isinstance(journey, dict) or set(journey) != ALLOWED_TOP:
        raise HarnessError(f"journey must contain exactly {sorted(ALLOWED_TOP)}")
    if journey["schema_version"] != 1 or journey["platforms"] != ["macos"]:
        raise HarnessError("only schema_version 1 and platforms: [macos] are supported")
    if journey["fixture"] != "local_review_channel":
        raise HarnessError("desktop MVP permits only fixture: local_review_channel")
    if not isinstance(journey["flow"], str) or not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", journey["flow"]):
        raise HarnessError("flow must be a lowercase filesystem-safe identifier")
    record = journey["record"]
    if not isinstance(record, dict) or set(record) != {"video", "screenshots", "accessibility"}:
        raise HarnessError("record requires exactly video, screenshots, accessibility")
    if record["video"] not in ("window", "off") or not all(isinstance(record[k], bool) for k in ("screenshots", "accessibility")):
        raise HarnessError("invalid record policy")
    steps = journey["steps"]
    if not isinstance(steps, list) or not steps:
        raise HarnessError("journey requires at least one step")
    for index, step in enumerate(steps):
        where = f"steps[{index}]"
        if not isinstance(step, dict) or set(step) - ALLOWED_STEP or not {"name", "act", "expect"} <= set(step):
            raise HarnessError(f"{where}: requires name/act/expect and contains an unsupported field")
        if not isinstance(step["name"], str) or not step["name"]:
            raise HarnessError(f"{where}.name must be non-empty")
        locators = step.get("locate")
        if locators is not None:
            if not isinstance(locators, list) or not locators:
                raise HarnessError(f"{where}.locate must be a non-empty list")
            for locator in locators:
                validate_locator(locator, f"{where}.locate")
        action = step["act"]
        action_type = action.get("type") if isinstance(action, dict) else None
        if action_type not in {"activate", "click", "move_pointer", "press", "scroll", "type_text", "wait"}:
            raise HarnessError(f"{where}.act has unsupported type")
        if set(action) - {"type", "duration_ms", "key", "modifiers", "text", "delta_y"}:
            raise HarnessError(f"{where}.act contains an unsupported field")
        if action_type in {"click", "move_pointer"} and locators is None:
            raise HarnessError(f"{where}: {action_type} requires locate")
        if action_type == "press":
            if not isinstance(action.get("key"), str) or not action["key"]:
                raise HarnessError(f"{where}: press requires key")
            modifiers = action.get("modifiers", [])
            if not isinstance(modifiers, list) or not all(isinstance(item, str) and item for item in modifiers):
                raise HarnessError(f"{where}: press modifiers must be strings")
        if action_type == "type_text" and not isinstance(action.get("text"), str):
            raise HarnessError(f"{where}: type_text requires text")
        if action_type == "scroll" and not isinstance(action.get("delta_y"), int):
            raise HarnessError(f"{where}: scroll requires integer delta_y")
        validate_expectation(step["expect"], f"{where}.expect")
        if "expect_for" in step:
            sustained = step["expect_for"]
            if not isinstance(sustained, dict) or set(sustained) != {"duration_ms", "condition"}:
                raise HarnessError(f"{where}.expect_for requires duration_ms and condition")
            validate_expectation(sustained["condition"], f"{where}.expect_for.condition")
        timeout = step.get("timeout_ms", 5000)
        if not isinstance(timeout, int) or not 0 < timeout <= 60000:
            raise HarnessError(f"{where}.timeout_ms must be 1..60000")
    cleanup = journey["cleanup"]
    if not isinstance(cleanup, dict) or set(cleanup) != {"terminate_app", "remove_state"} or not all(isinstance(v, bool) for v in cleanup.values()):
        raise HarnessError("cleanup requires boolean terminate_app and remove_state")
    return journey


def isolation_manifest(run_id: str, relay_url: str) -> dict[str, str]:
    parsed = urllib.parse.urlparse(relay_url)
    if parsed.scheme not in {"ws", "http"} or parsed.hostname not in {"localhost", "127.0.0.1", "::1"}:
        raise HarnessError(f"refusing non-loopback review relay: {relay_url}")
    slug = re.sub(r"[^a-z0-9-]", "-", run_id.lower())
    bundle_id = f"xyz.block.buzz.app.dev.native-review.{slug}"
    keyring = f"buzz-desktop-dev.native-review.{slug}"
    if bundle_id in PRODUCTION_BUNDLE_IDS or not bundle_id.startswith("xyz.block.buzz.app.dev.native-review."):
        raise HarnessError(f"refusing unsafe bundle identifier: {bundle_id}")
    if keyring in PRODUCTION_KEYRINGS or not keyring.startswith("buzz-desktop-dev.native-review."):
        raise HarnessError(f"refusing unsafe keyring service: {keyring}")
    return {"bundle_id": bundle_id, "keyring_service": keyring, "relay_url": relay_url}


def provenance() -> dict[str, Any]:
    head = git("rev-parse", "HEAD")
    status = git("status", "--porcelain=v1", "--untracked-files=all")
    diff = run(["git", "diff", "--binary", "HEAD"]).stdout
    untracked_hashes = []
    for line in status.splitlines():
        if line.startswith("?? "):
            path = ROOT / line[3:]
            if path.is_file():
                untracked_hashes.append((line[3:], sha256(path)))
    dirty_payload = json.dumps({"diff": diff, "untracked": untracked_hashes}, sort_keys=True).encode()
    return {
        "head_sha": head,
        "dirty": bool(status),
        "dirty_state_sha256": hashlib.sha256(dirty_payload).hexdigest() if status else None,
        "status": status.splitlines(),
    }


def scrubbed_environment() -> dict[str, str]:
    keep = {"PATH", "TMPDIR", "LANG", "LC_ALL", "SHELL", "USER", "LOGNAME", "TERM", "SSH_AUTH_SOCK", "__CF_USER_TEXT_ENCODING"}
    env = {key: value for key, value in os.environ.items() if key in keep and not SECRET_NAME.search(key)}
    env["HOME"] = ""  # filled per run
    return env


def driver_binary() -> pathlib.Path:
    override = os.environ.get("BUZZ_NATIVE_REVIEW_DRIVER")
    if override:
        return pathlib.Path(override).resolve()
    return TOOL_ROOT / "swift" / ".build" / "release" / "buzz-native-driver"


def build_driver() -> pathlib.Path:
    binary = driver_binary()
    if os.environ.get("BUZZ_NATIVE_REVIEW_DRIVER"):
        if not binary.is_file():
            raise HarnessError(f"configured driver does not exist: {binary}")
        return binary
    run(["swift", "build", "-c", "release", "--package-path", str(TOOL_ROOT / "swift")], capture=False)
    if not binary.is_file():
        raise HarnessError(f"Swift build succeeded without driver binary: {binary}")
    return binary


class Driver:
    def __init__(self, binary: pathlib.Path, pid: int, semantic_snapshot: pathlib.Path):
        self.process = subprocess.Popen([str(binary), "serve", "--pid", str(pid),
                                         "--semantic-snapshot", str(semantic_snapshot)], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)

    def request(self, command: str, **payload: Any) -> dict[str, Any]:
        assert self.process.stdin and self.process.stdout
        self.process.stdin.write(json.dumps({"command": command, **payload}) + "\n")
        self.process.stdin.flush()
        timeout_seconds = 45 if command in {"record_start", "record_stop"} else 15
        ready, _, _ = select.select([self.process.stdout], [], [], timeout_seconds)
        if not ready:
            self.process.kill()
            self.process.wait(timeout=5)
            stderr = self.process.stderr.read() if self.process.stderr else ""
            detail = f"; driver stderr:\n{stderr.strip()}" if stderr.strip() else ""
            raise HarnessError(f"native driver timed out during {command}{detail}")
        line = self.process.stdout.readline()
        if not line:
            stderr = self.process.stderr.read() if self.process.stderr else ""
            raise HarnessError(f"native driver exited during {command}: {stderr.strip()}")
        response = json.loads(line)
        if not response.get("ok"):
            raise HarnessError(str(response.get("error", f"driver {command} failed")))
        return response

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.request("shutdown")
            except Exception:
                self.process.terminate()
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream:
                stream.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()


def doctor(require_permissions: bool = False) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []
    checks.append({"name": "platform", "ok": sys.platform == "darwin", "detail": sys.platform})
    for command in ("swift", "xcrun", "git"):
        path = shutil.which(command)
        checks.append({"name": command, "ok": path is not None, "detail": path or "not found"})
    checks.append({"name": "repository", "ok": (ROOT / "desktop/src-tauri/tauri.conf.json").is_file(), "detail": str(ROOT)})
    try:
        binary = build_driver() if all(c["ok"] for c in checks[:4]) else driver_binary()
        result = run([str(binary), "doctor"], check=False)
        native = json.loads(result.stdout) if result.stdout else {"ok": False, "error": result.stderr.strip()}
        checks.extend(native.get("checks", []))
    except (HarnessError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        checks.append({"name": "native-driver", "ok": False, "detail": str(exc)})
    hard_names = {"platform", "swift", "xcrun", "git", "repository", "native-driver-build"}
    failed = [c for c in checks if not c.get("ok") and (require_permissions or c.get("name") in hard_names)]
    result = {"ok": not failed, "checks": checks}
    print(json.dumps(result, indent=2))
    return result


def prepare_fixture(run_dir: pathlib.Path, isolation: dict[str, str]) -> dict[str, Any]:
    admin = ROOT / "target" / "debug" / "buzz-admin"
    if not admin.is_file():
        run(["cargo", "build", "-p", "buzz-admin"], capture=False)
    generated = run([str(admin), "generate-key"]).stdout
    secret_match = re.search(r"Secret key:\s+(\S+)", generated)
    public_match = re.search(r"Public key:\s+(\S+)", generated)
    if not secret_match or not public_match:
        raise HarnessError("buzz-admin generate-key returned an unrecognized response")
    secret_path = run_dir / "state" / "identity.key"
    secret_path.parent.mkdir(parents=True, exist_ok=True)
    secret_path.write_text(secret_match.group(1) + "\n")
    secret_path.chmod(0o600)
    fixture = {
        "kind": "local_review_channel", "identity_pubkey": public_match.group(1),
        "secret_path": str(secret_path), "relay_url": isolation["relay_url"],
        "seed": "scripts/setup-desktop-test-data.sh", "cleanup_scope": "run-local app state and keyring only",
    }
    parsed = urllib.parse.urlparse(isolation["relay_url"])
    port = parsed.port or 80
    seed_env = {**os.environ, "BUZZ_REVIEW_PUBKEY": fixture["identity_pubkey"],
                "BUZZ_COMMUNITY_HOST": f"{parsed.hostname}:{port}"}
    if port == 3030:
        seed_env.update({"BUZZ_DB_HOST": "localhost", "BUZZ_DB_PORT": "5471", "BUZZ_DB_USER": "buzz",
                         "BUZZ_DB_PASS": "buzz_dev", "BUZZ_DB_NAME": "buzz",
                         "BUZZ_DB_DOCKER_CONTAINER": "buzz-harness-postgres-1"})
    run([str(ROOT / "scripts/setup-desktop-test-data.sh")], env=seed_env, capture=False)
    (run_dir / "manifest" / "fixture.json").write_text(json.dumps({k: v for k, v in fixture.items() if k != "secret_path"}, indent=2))
    return fixture


def semantic_probe_server(path: pathlib.Path) -> tuple[http.server.ThreadingHTTPServer, str]:
    class Handler(http.server.BaseHTTPRequestHandler):
        def end_headers(self) -> None:
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "POST, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.send_header("Access-Control-Allow-Private-Network", "true")
            super().end_headers()

        def do_OPTIONS(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
            self.send_response(204)
            self.end_headers()

        def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
            length = int(self.headers.get("Content-Length", "0"))
            payload = self.rfile.read(length)
            try:
                value = json.loads(payload)
                if not isinstance(value, list):
                    raise ValueError("snapshot must be an array")
                temporary = path.with_suffix(".json.tmp")
                temporary.write_text(json.dumps(value))
                temporary.replace(path)
                self.send_response(204)
            except (ValueError, json.JSONDecodeError, OSError):
                self.send_response(400)
            self.end_headers()

        def log_message(self, _format: str, *_args: Any) -> None:
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}/snapshot"


def build_and_launch(run_dir: pathlib.Path, isolation: dict[str, str], fixture: dict[str, Any],
                     probe_url: str) -> tuple[subprocess.Popen[str], pathlib.Path, int]:
    # Build with the repository toolchain but no inherited credentials. Launch
    # the resulting executable separately so only the app receives isolated HOME.
    dev_url = (
        "http://localhost:1420?nativeReview=1"
        f"&reviewRelay={urllib.parse.quote(isolation['relay_url'], safe='')}"
        f"&reviewPubkey={urllib.parse.quote(fixture['identity_pubkey'], safe='')}"
    )
    config = json.dumps({
        "build": {"devUrl": dev_url},
        "identifier": isolation["bundle_id"], "productName": "Buzz Native Review",
        "bundle": {"externalBin": []},
    }, separators=(",", ":"))
    build_env = scrubbed_environment()
    build_env["HOME"] = os.environ["HOME"]
    build_env["VITE_NATIVE_REVIEW"] = "1"
    build_env["VITE_NATIVE_REVIEW_RELAY"] = isolation["relay_url"]
    build_env["VITE_NATIVE_REVIEW_PUBKEY"] = fixture["identity_pubkey"]
    build_env["VITE_NATIVE_REVIEW_PROBE_URL"] = probe_url
    run(["pnpm", "exec", "tauri", "build", "--debug", "--bundles", "app", "--config", config],
        cwd=ROOT / "desktop", env=build_env, capture=False)
    app_binary = (ROOT / "desktop" / "src-tauri" / "target" / "debug" / "bundle" / "macos" /
                  "Buzz Native Review.app" / "Contents" / "MacOS" / "buzz-desktop")
    if not app_binary.is_file():
        raise HarnessError(f"Tauri build succeeded without app binary: {app_binary}")

    env = scrubbed_environment()
    env["HOME"] = str(run_dir / "home")
    pathlib.Path(env["HOME"]).mkdir(parents=True, exist_ok=True)
    env.update({
        "BUZZ_PRIVATE_KEY": pathlib.Path(fixture["secret_path"]).read_text().strip(),
        "BUZZ_RELAY_URL": isolation["relay_url"], "BUZZ_DEV_KEYRING_SERVICE": isolation["keyring_service"],
        "BUZZ_NATIVE_REVIEW": "1", "BUZZ_NATIVE_REVIEW_CHANNEL": "general",
    })
    log = (run_dir / "logs" / "app.log").open("w")
    process = subprocess.Popen([str(app_binary)], cwd=ROOT, env=env, stdout=log,
                               stderr=subprocess.STDOUT, text=True)
    log.close()
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise HarnessError(f"Tauri exited during launch; see {run_dir / 'logs/app.log'}")
        if run(["ps", "-p", str(process.pid), "-o", "comm="], check=False).stdout.rstrip().endswith("/buzz-desktop"):
            return process, app_binary, process.pid
        time.sleep(0.25)
    process.terminate()
    raise HarnessError("timed out waiting for native Buzz process")


def wait_for_visible_window(driver: Driver, process: subprocess.Popen[str], timeout_seconds: float = 30) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    last_status: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise HarnessError("Tauri exited while waiting for its initial window")
        last_status = driver.request("window_status")
        if last_status.get("visible"):
            return last_status
        time.sleep(0.1)
    detail = last_status.get("detail") if last_status else "driver returned no status"
    raise HarnessError(f"timed out after {timeout_seconds:g}s waiting for visible native window: {detail}")


def locate_required(driver: Driver, locators: list[dict[str, str]], timeout_ms: int) -> dict[str, Any]:
    """Wait for a semantic target to materialize, then return the locator used."""
    deadline = time.monotonic() + timeout_ms / 1000
    while True:
        found = driver.request("locate", locators=locators, required=False).get("element")
        if found is not None:
            return found
        if time.monotonic() >= deadline:
            raise HarnessError(f"no accessibility element matched ordered locators within {timeout_ms}ms")
        time.sleep(0.025)


def expectation_holds(driver: Driver, expectation: dict[str, Any]) -> bool:
    if "exists" in expectation:
        return driver.request("locate", locators=[expectation["exists"]], required=False).get("element") is not None
    if "not_exists" in expectation:
        return driver.request("locate", locators=[expectation["not_exists"]], required=False).get("element") is None
    if "focused" in expectation and isinstance(expectation["focused"], dict):
        found = driver.request("locate", locators=[expectation["focused"]], required=False).get("element")
        return bool(found and found.get("focused"))
    if "focused" in expectation:
        return bool(driver.request("focused").get("focused")) == expectation["focused"]
    if "enabled" in expectation:
        return bool(driver.request("selected").get("element", {}).get("enabled")) == expectation["enabled"]
    if "value" in expectation:
        return driver.request("selected").get("element", {}).get("value") == expectation["value"]
    return False


def wait_expectation(driver: Driver, expectation: dict[str, Any], timeout_ms: int) -> None:
    deadline = time.monotonic() + timeout_ms / 1000
    while True:
        if expectation_holds(driver, expectation):
            return
        if time.monotonic() >= deadline:
            raise HarnessError(f"postcondition not met within {timeout_ms}ms: {expectation}")
        time.sleep(0.025)


def capture_step(driver: Driver, run_dir: pathlib.Path, slug: str, record: dict[str, Any]) -> dict[str, str]:
    artifacts: dict[str, str] = {}
    if record["screenshots"]:
        path = run_dir / "screenshots" / f"{slug}.png"
        driver.request("screenshot", path=str(path))
        artifacts["screenshot"] = str(path.relative_to(run_dir))
    if record["accessibility"]:
        path = run_dir / "accessibility" / f"{slug}.json"
        response = driver.request("snapshot")
        path.write_text(json.dumps({key: value for key, value in response.items() if key != "ok"}, indent=2))
        artifacts["accessibility"] = str(path.relative_to(run_dir))
    return artifacts


def run_journey(path: pathlib.Path, relay_url: str, output_root: pathlib.Path) -> pathlib.Path:
    journey = load_journey(path)
    run_id = f"{journey['flow']}-{dt.datetime.now().strftime('%Y%m%dT%H%M%S')}-{secrets.token_hex(3)}"
    isolation = isolation_manifest(run_id, relay_url)
    run_dir = output_root / git("rev-parse", "--short=12", "HEAD") / journey["flow"] / run_id
    for child in ("manifest", "logs", "screenshots", "accessibility", "state", "home"):
        (run_dir / child).mkdir(parents=True, exist_ok=True)
    (run_dir / "journey.yaml").write_text(path.read_text())
    prov = provenance()
    (run_dir / "manifest" / "git.json").write_text(json.dumps(prov, indent=2))
    (run_dir / "manifest" / "isolation.json").write_text(json.dumps(isolation, indent=2))
    started = utc_now()
    receipt: dict[str, Any] = {"schema_version": 1, "run_id": run_id, "flow": journey["flow"], "status": "failed",
        "started_at": started, "finished_at": started, "failure": None, "provenance": prov, "isolation": isolation,
        "artifacts": {}, "steps": [], "cleanup": {"status": "not_started"}}
    process: subprocess.Popen[str] | None = None
    driver: Driver | None = None
    fixture: dict[str, Any] | None = None
    probe_server: http.server.ThreadingHTTPServer | None = None
    try:
        if not doctor(require_permissions=True)["ok"]:
            raise HarnessError("doctor failed; grant required permissions and rerun")
        fixture = prepare_fixture(run_dir, isolation)
        probe_server, probe_url = semantic_probe_server(run_dir / "state" / "semantic.json")
        process, app_binary, app_pid = build_and_launch(run_dir, isolation, fixture, probe_url)
        receipt["provenance"]["artifact_path"] = str(app_binary)
        receipt["provenance"]["artifact_sha256"] = sha256(app_binary)
        driver = Driver(build_driver(), app_pid, run_dir / "state" / "semantic.json")
        receipt["provenance"]["initial_window"] = wait_for_visible_window(driver, process)
        if journey["record"]["video"] == "window":
            video = run_dir / "video.mp4"
            driver.request("record_start", path=str(video))
            receipt["artifacts"]["video"] = "video.mp4"
        for index, step in enumerate(journey["steps"]):
            slug = f"{index + 1:02d}-{re.sub(r'[^a-z0-9-]', '-', step['name'].lower())}"
            step_start = time.monotonic_ns()
            selected = None
            step_receipt: dict[str, Any] = {"name": step["name"], "status": "failed", "started_monotonic_ns": step_start}
            receipt["steps"].append(step_receipt)
            try:
                if step.get("locate"):
                    selected = locate_required(driver, step["locate"], step.get("timeout_ms", 5000))
                    step_receipt["locator"] = selected.get("locator")
                driver.request("act", action=step["act"], element=selected)
                wait_expectation(driver, step["expect"], step.get("timeout_ms", 5000))
                if sustained := step.get("expect_for"):
                    until = time.monotonic() + sustained["duration_ms"] / 1000
                    while time.monotonic() < until:
                        if not expectation_holds(driver, sustained["condition"]):
                            raise HarnessError(f"sustained postcondition failed: {sustained['condition']}")
                        time.sleep(0.025)
                step_receipt["status"] = "passed"
            finally:
                step_receipt["finished_monotonic_ns"] = time.monotonic_ns()
                step_receipt["duration_ms"] = (step_receipt["finished_monotonic_ns"] - step_start) / 1_000_000
                step_receipt["artifacts"] = capture_step(driver, run_dir, slug, journey["record"])
        if journey["record"]["video"] == "window":
            driver.request("record_stop")
        receipt["status"] = "passed"
    except Exception as exc:
        receipt["failure"] = str(exc)
        if driver:
            try:
                receipt["artifacts"]["failure"] = capture_step(driver, run_dir, "failure", {"screenshots": True, "accessibility": True})
            except Exception as capture_exc:
                receipt["artifacts"]["capture_failure"] = str(capture_exc)
            try:
                driver.request("record_stop")
            except Exception:
                pass
    finally:
        cleanup_errors = []
        if probe_server:
            probe_server.shutdown()
            probe_server.server_close()
        if driver:
            try:
                driver.close()
            except Exception as exc:
                cleanup_errors.append(f"native driver cleanup failed: {exc}")
        if process and journey["cleanup"]["terminate_app"]:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill(); cleanup_errors.append("Tauri launcher required SIGKILL")
        if journey["cleanup"]["remove_state"]:
            try:
                run([str(ROOT / "scripts/reset-desktop-standalone-state.sh"), isolation["bundle_id"], isolation["keyring_service"]],
                    env={**os.environ, "HOME": str(run_dir / "home")})
                if fixture:
                    pathlib.Path(fixture["secret_path"]).unlink(missing_ok=True)
            except Exception as exc:
                cleanup_errors.append(str(exc))
        receipt["cleanup"] = {"status": "failed" if cleanup_errors else "passed", "errors": cleanup_errors}
        if cleanup_errors:
            receipt["status"] = "failed"
        receipt["finished_at"] = utc_now()
        (run_dir / "receipt.json").write_text(json.dumps(receipt, indent=2))
        report = f"# Native review: {journey['flow']}\n\n**{receipt['status'].upper()}** at `{prov['head_sha']}`.\n\nReceipt: `receipt.json`\n"
        if receipt["failure"]:
            report += f"\nFailure: `{receipt['failure']}`\n"
        (run_dir / "report.md").write_text(report)
    print(run_dir)
    if receipt["status"] != "passed":
        raise HarnessError(receipt["failure"] or "journey or cleanup failed")
    return run_dir


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    doctor_parser = sub.add_parser("doctor")
    doctor_parser.add_argument("--require-permissions", action="store_true")
    validate_parser = sub.add_parser("validate")
    validate_parser.add_argument("journey", type=pathlib.Path)
    run_parser = sub.add_parser("run")
    run_parser.add_argument("journey", type=pathlib.Path)
    run_parser.add_argument("--relay", default="ws://localhost:3030")
    run_parser.add_argument("--output", type=pathlib.Path, default=ROOT / "test-results/native-review")
    args = parser.parse_args()
    try:
        if args.command == "doctor":
            return 0 if doctor(args.require_permissions)["ok"] else 1
        if args.command == "validate":
            load_journey(args.journey); print(f"valid: {args.journey}"); return 0
        run_journey(args.journey, args.relay, args.output); return 0
    except HarnessError as exc:
        print(f"native-review: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
