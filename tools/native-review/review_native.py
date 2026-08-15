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
import platform
import re
import secrets
import select
import shutil
import statistics
import subprocess
import sys

_TOOL_ROOT = pathlib.Path(__file__).resolve().parent
if str(_TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(_TOOL_ROOT))
import tempfile
import threading
import time
import urllib.parse
from typing import Any

from evidence_bundle import EvidenceError, finding_bundle, relay_safe_video
import review_publish
from review_publish import PublishError, publish_review

try:
    import yaml
except ImportError as exc:  # pragma: no cover - environment preflight
    raise SystemExit("PyYAML is required (activate the repository Hermit environment)") from exc

ROOT = pathlib.Path(os.environ.get("BUZZ_NATIVE_REVIEW_ROOT", pathlib.Path(__file__).resolve().parents[2])).resolve()
TOOL_ROOT = pathlib.Path(__file__).resolve().parent
PRODUCTION_BUNDLE_IDS = {"xyz.block.buzz.app", "xyz.block.sprout.app"}
PRODUCTION_KEYRINGS = {"buzz-desktop", "sprout-desktop"}
SECRET_NAME = re.compile(r"(AUTH|TOKEN|SECRET|PASSWORD|PRIVATE_KEY|COOKIE)", re.I)
ALLOWED_TOP = {"schema_version", "flow", "platforms", "fixture", "record", "steps", "cleanup"}
ALLOWED_STEP = {"name", "locate", "act", "expect", "expect_for", "timeout_ms", "measure"}
PERFORMANCE_SAMPLE_MINIMUM = 3


class HarnessError(RuntimeError):
    pass


def run(command: list[str], *, cwd: pathlib.Path = ROOT, env: dict[str, str] | None = None,
        check: bool = True, capture: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd,
                          env=scrubbed_environment(include_home=True) if env is None else env,
                          check=check, text=True,
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


def machine_fingerprint() -> dict[str, str]:
    """Return comparison-critical host attributes without user-specific data."""
    cpu = run(["sysctl", "-n", "machdep.cpu.brand_string"], check=False).stdout.strip()
    return {
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "cpu": cpu or platform.processor() or "unknown",
    }


class ProcessSampler:
    """Sample app CPU and resident memory while a native journey is active."""

    def __init__(self, pid: int, interval_seconds: float = 0.1):
        self.pid = pid
        self.interval_seconds = interval_seconds
        self.samples: list[dict[str, float]] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._sample, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def _sample(self) -> None:
        while not self._stop.is_set():
            result = run(["ps", "-p", str(self.pid), "-o", "%cpu=", "-o", "rss="], check=False)
            fields = result.stdout.split()
            if len(fields) == 2:
                try:
                    self.samples.append({
                        "elapsed_ms": time.monotonic() * 1000,
                        "cpu_percent": float(fields[0]),
                        "resident_mb": int(fields[1]) / 1024,
                    })
                except ValueError:
                    pass
            self._stop.wait(self.interval_seconds)

    def finish(self) -> dict[str, Any]:
        self._stop.set()
        self._thread.join(timeout=2)
        if not self.samples:
            return {"sample_count": 0}
        return {
            "sample_count": len(self.samples),
            "interval_ms": self.interval_seconds * 1000,
            "cpu_percent_median": statistics.median(item["cpu_percent"] for item in self.samples),
            "cpu_percent_peak": max(item["cpu_percent"] for item in self.samples),
            "resident_mb_median": statistics.median(item["resident_mb"] for item in self.samples),
            "resident_mb_peak": max(item["resident_mb"] for item in self.samples),
        }


def validate_locator(locator: Any, where: str) -> None:
    if not isinstance(locator, dict) or not locator or set(locator) - {"id", "role", "name"}:
        raise HarnessError(f"{where}: locator must contain only id, role, and/or name")
    if not all(isinstance(value, str) and value for value in locator.values()):
        raise HarnessError(f"{where}: locator values must be non-empty strings")


def validate_expectation(expectation: Any, where: str) -> None:
    allowed = {"exists", "not_exists", "focused", "enabled", "value", "scroll_y_greater_than", "scroll_y_less_than"}
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
    for key in ("scroll_y_greater_than", "scroll_y_less_than"):
        if key in expectation and not isinstance(expectation[key], (int, float)):
            raise HarnessError(f"{where}.{key} must be numeric")


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
    measurement_names: set[str] = set()
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
        if "measure" in step:
            measurement = step["measure"]
            if not isinstance(measurement, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_.-]*", measurement):
                raise HarnessError(f"{where}.measure must be a lowercase metric identifier")
            if measurement in measurement_names:
                raise HarnessError(f"duplicate measurement name: {measurement}")
            measurement_names.add(measurement)
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


def scrubbed_environment(*, include_home: bool = False) -> dict[str, str]:
    keep = {"PATH", "TMPDIR", "LANG", "LC_ALL", "SHELL", "USER", "LOGNAME", "TERM", "__CF_USER_TEXT_ENCODING"}
    env = {key: value for key, value in os.environ.items() if key in keep and not SECRET_NAME.search(key)}
    env["HOME"] = os.environ.get("HOME", "") if include_home else ""  # isolated per run unless tooling needs host caches
    return env


def fixture_environment(isolation: dict[str, str], review_pubkey: str) -> dict[str, str]:
    """Return fixed local fixture coordinates without inheriting host credentials."""
    parsed = urllib.parse.urlparse(isolation["relay_url"])
    port = parsed.port or 80
    if port != 3030:
        raise HarnessError("fixture seeding requires the isolated relay at loopback port 3030")
    return {
        **scrubbed_environment(include_home=True),
        "BUZZ_REVIEW_PUBKEY": review_pubkey,
        "BUZZ_COMMUNITY_HOST": f"{parsed.hostname}:{port}",
        "BUZZ_DB_HOST": "localhost",
        "BUZZ_DB_PORT": "5471",
        "BUZZ_DB_USER": "buzz",
        "BUZZ_DB_PASS": "buzz_dev",
        "BUZZ_DB_NAME": "buzz",
        "BUZZ_DB_DOCKER_CONTAINER": "buzz-harness-postgres-1",
    }


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
    run(["swift", "build", "-c", "release", "--package-path", str(TOOL_ROOT / "swift")],
        env=scrubbed_environment(include_home=True), capture=False)
    if not binary.is_file():
        raise HarnessError(f"Swift build succeeded without driver binary: {binary}")
    return binary


class Driver:
    def __init__(self, binary: pathlib.Path, pid: int, semantic_snapshot: pathlib.Path):
        self.process = subprocess.Popen([str(binary), "serve", "--pid", str(pid),
                                         "--semantic-snapshot", str(semantic_snapshot)], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        env=scrubbed_environment(include_home=True), text=True, bufsize=1)

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
    for command in ("swift", "xcrun", "git", "ffmpeg"):
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
    hard_names = {"platform", "swift", "xcrun", "git", "ffmpeg", "repository", "native-driver-build"}
    failed = [c for c in checks if not c.get("ok") and (require_permissions or c.get("name") in hard_names)]
    result = {"ok": not failed, "checks": checks}
    print(json.dumps(result, indent=2))
    return result


def prepare_fixture(run_dir: pathlib.Path, isolation: dict[str, str]) -> dict[str, Any]:
    admin = ROOT / "target" / "debug" / "buzz-admin"
    if not admin.is_file():
        run(["cargo", "build", "-p", "buzz-admin"], env=scrubbed_environment(include_home=True), capture=False)
    generated = run([str(admin), "generate-key"], env=scrubbed_environment(include_home=True)).stdout
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
    try:
        run([str(ROOT / "scripts/setup-desktop-test-data.sh")],
            env=fixture_environment(isolation, fixture["identity_pubkey"]), capture=False)
    except Exception:
        secret_path.unlink(missing_ok=True)
        raise
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
    build_env = scrubbed_environment(include_home=True)
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
    if "scroll_y_greater_than" in expectation:
        scroll_y = driver.request("selected").get("element", {}).get("scrollY")
        return isinstance(scroll_y, (int, float)) and scroll_y > expectation["scroll_y_greater_than"]
    if "scroll_y_less_than" in expectation:
        scroll_y = driver.request("selected").get("element", {}).get("scrollY")
        return isinstance(scroll_y, (int, float)) and scroll_y < expectation["scroll_y_less_than"]
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


def cleanup_review_state(run_dir: pathlib.Path, isolation: dict[str, str],
                         fixture: dict[str, Any] | None) -> None:
    errors = []
    env = scrubbed_environment()
    env["HOME"] = str(run_dir / "home")
    try:
        run([str(ROOT / "scripts/reset-desktop-standalone-state.sh"),
             isolation["bundle_id"], isolation["keyring_service"]], env=env)
    except Exception as exc:
        errors.append(f"desktop state reset failed: {exc}")
    if fixture:
        try:
            pathlib.Path(fixture["secret_path"]).unlink(missing_ok=True)
        except Exception as exc:
            errors.append(f"review identity removal failed: {exc}")
    if errors:
        raise HarnessError("; ".join(errors))


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
        "artifacts": {}, "steps": [], "measurements": {}, "performance": {"machine": machine_fingerprint()},
        "cleanup": {"status": "not_started"}}
    process: subprocess.Popen[str] | None = None
    driver: Driver | None = None
    fixture: dict[str, Any] | None = None
    probe_server: http.server.ThreadingHTTPServer | None = None
    sampler: ProcessSampler | None = None
    try:
        if not doctor(require_permissions=True)["ok"]:
            raise HarnessError("doctor failed; grant required permissions and rerun")
        fixture = prepare_fixture(run_dir, isolation)
        probe_server, probe_url = semantic_probe_server(run_dir / "state" / "semantic.json")
        process, app_binary, app_pid = build_and_launch(run_dir, isolation, fixture, probe_url)
        receipt["provenance"]["artifact_path"] = str(app_binary)
        receipt["provenance"]["artifact_sha256"] = sha256(app_binary)
        driver = Driver(build_driver(), app_pid, run_dir / "state" / "semantic.json")
        sampler = ProcessSampler(app_pid)
        sampler.start()
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
                if measurement := step.get("measure"):
                    receipt["measurements"][measurement] = {"value": step_receipt["duration_ms"], "unit": "ms", "step": step["name"]}
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
        if sampler:
            receipt["performance"]["process"] = sampler.finish()
        video = run_dir / "video.mp4"
        if video.is_file():
            try:
                relay_safe_video(video, run_dir / "video-share.mp4")
                receipt["artifacts"]["share_video"] = "video-share.mp4"
            except EvidenceError as exc:
                cleanup_errors.append(f"shareable video finalization failed: {exc}")
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
                cleanup_review_state(run_dir, isolation, fixture)
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


def load_receipts(paths: list[pathlib.Path], label: str) -> list[dict[str, Any]]:
    receipts = []
    for path in paths:
        try:
            receipt = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise HarnessError(f"cannot read {label} receipt {path}: {exc}") from exc
        if receipt.get("status") != "passed" or receipt.get("cleanup", {}).get("status") != "passed":
            raise HarnessError(f"{label} receipt is not a clean pass: {path}")
        if receipt.get("provenance", {}).get("dirty"):
            raise HarnessError(f"{label} receipt was captured from a dirty tree: {path}")
        receipts.append(receipt)
    return receipts


def metric_value(receipt: dict[str, Any], metric: str) -> float:
    if metric.startswith("process."):
        value = receipt.get("performance", {}).get("process", {}).get(metric.removeprefix("process."))
    else:
        value = receipt.get("measurements", {}).get(metric, {}).get("value")
    if not isinstance(value, (int, float)):
        raise HarnessError(f"receipt {receipt.get('run_id')} has no numeric metric {metric}")
    return float(value)


def cohort_summary(receipts: list[dict[str, Any]], metrics: list[str]) -> dict[str, Any]:
    return {
        "sample_count": len(receipts),
        "head_sha": receipts[0]["provenance"]["head_sha"],
        "artifact_sha256": [receipt["provenance"]["artifact_sha256"] for receipt in receipts],
        "metrics": {
            metric: {
                "median": statistics.median(values := [metric_value(receipt, metric) for receipt in receipts]),
                "minimum": min(values),
                "maximum": max(values),
                "samples": values,
            }
            for metric in metrics
        },
    }


def compare_performance(baseline_paths: list[pathlib.Path], candidate_paths: list[pathlib.Path],
                        budget_path: pathlib.Path, output: pathlib.Path | None = None) -> dict[str, Any]:
    try:
        budget = yaml.safe_load(budget_path.read_text())
    except (OSError, yaml.YAMLError) as exc:
        raise HarnessError(f"cannot read performance budget {budget_path}: {exc}") from exc
    if not isinstance(budget, dict) or set(budget) != {"schema_version", "flow", "minimum_samples", "metrics"}:
        raise HarnessError("performance budget requires exactly schema_version, flow, minimum_samples, and metrics")
    if budget["schema_version"] != 1 or not isinstance(budget["flow"], str):
        raise HarnessError("unsupported performance budget schema")
    minimum = budget["minimum_samples"]
    if not isinstance(minimum, int) or minimum < PERFORMANCE_SAMPLE_MINIMUM:
        raise HarnessError(f"minimum_samples must be at least {PERFORMANCE_SAMPLE_MINIMUM}")
    metrics = budget["metrics"]
    if not isinstance(metrics, dict) or not metrics:
        raise HarnessError("performance budget requires at least one metric")
    allowed_limits = {"max", "max_regression_percent"}
    for name, limits in metrics.items():
        if (not isinstance(name, str) or not name or not isinstance(limits, dict) or not limits
                or set(limits) - allowed_limits
                or not all(isinstance(value, (int, float)) and not isinstance(value, bool) and value >= 0 for value in limits.values())):
            raise HarnessError(f"invalid limits for performance metric {name}")

    baseline = load_receipts(baseline_paths, "baseline")
    candidate = load_receipts(candidate_paths, "candidate")
    if len(baseline) < minimum or len(candidate) < minimum:
        raise HarnessError(f"performance comparison requires at least {minimum} clean samples per cohort")
    all_receipts = baseline + candidate
    flows = {receipt.get("flow") for receipt in all_receipts}
    machines = {json.dumps(receipt.get("performance", {}).get("machine"), sort_keys=True) for receipt in all_receipts}
    if flows != {budget["flow"]}:
        raise HarnessError(f"receipt flows {sorted(str(item) for item in flows)} do not match budget flow {budget['flow']}")
    if len(machines) != 1:
        raise HarnessError("baseline and candidate receipts were captured on incompatible machines")
    for label, cohort in (("baseline", baseline), ("candidate", candidate)):
        if len({receipt["provenance"].get("head_sha") for receipt in cohort}) != 1:
            raise HarnessError(f"{label} cohort mixes source revisions")

    baseline_summary = cohort_summary(baseline, list(metrics))
    candidate_summary = cohort_summary(candidate, list(metrics))
    verdicts = {}
    failures = []
    for name, limits in metrics.items():
        baseline_median = baseline_summary["metrics"][name]["median"]
        candidate_median = candidate_summary["metrics"][name]["median"]
        candidate_maximum = candidate_summary["metrics"][name]["maximum"]
        regression = None if baseline_median == 0 and candidate_median > 0 else (
            0.0 if baseline_median == 0 else (candidate_median - baseline_median) / baseline_median * 100
        )
        reasons = []
        if "max" in limits and candidate_maximum > limits["max"]:
            reasons.append(f"maximum {candidate_maximum:.3f} exceeds absolute maximum {limits['max']}")
        if "max_regression_percent" in limits:
            if regression is None:
                reasons.append("relative regression is undefined because the baseline median is zero")
            elif regression > limits["max_regression_percent"]:
                reasons.append(f"regression {regression:.2f}% exceeds {limits['max_regression_percent']}%")
        verdicts[name] = {"status": "failed" if reasons else "passed", "regression_percent": regression, "reasons": reasons}
        failures.extend(f"{name}: {reason}" for reason in reasons)
    result = {
        "schema_version": 1, "status": "failed" if failures else "passed", "flow": budget["flow"],
        "machine": all_receipts[0]["performance"]["machine"], "baseline": baseline_summary,
        "candidate": candidate_summary, "verdicts": verdicts, "failures": failures,
    }
    serialized = json.dumps(result, indent=2, allow_nan=False)
    if output:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(serialized + "\n")
    print(serialized)
    if failures:
        raise HarnessError("performance budget failed: " + "; ".join(failures))
    return result


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
    benchmark_parser = sub.add_parser("benchmark")
    benchmark_parser.add_argument("journey", type=pathlib.Path)
    benchmark_parser.add_argument("--runs", type=int, default=5)
    benchmark_parser.add_argument("--relay", default="ws://localhost:3030")
    benchmark_parser.add_argument("--output", type=pathlib.Path, default=ROOT / "test-results/native-review")
    compare_parser = sub.add_parser("compare")
    compare_parser.add_argument("--baseline", type=pathlib.Path, action="append", required=True)
    compare_parser.add_argument("--candidate", type=pathlib.Path, action="append", required=True)
    compare_parser.add_argument("--budget", type=pathlib.Path, required=True)
    compare_parser.add_argument("--output", type=pathlib.Path)
    bundle_parser = sub.add_parser("finding-bundle")
    bundle_parser.add_argument("receipt", type=pathlib.Path)
    bundle_parser.add_argument("--output", type=pathlib.Path, required=True)
    bundle_parser.add_argument("--match", help="case-insensitive regular expression selecting log lines")
    bundle_parser.add_argument("--context", type=int, default=8)
    bundle_parser.add_argument("--start", type=float, help="clip start in seconds")
    bundle_parser.add_argument("--duration", type=float, help="clip duration in seconds")
    publish_parser = sub.add_parser("publish-review")
    publish_parser.add_argument("receipt", type=pathlib.Path)
    publish_parser.add_argument("--summary", type=pathlib.Path, required=True)
    publish_parser.add_argument("--channel", required=True)
    publish_parser.add_argument("--reply-to", required=True)
    publish_parser.add_argument("--highlights", type=pathlib.Path)
    publish_parser.add_argument("--mention", action="append", default=[])
    args = parser.parse_args()
    try:
        if args.command == "doctor":
            return 0 if doctor(args.require_permissions)["ok"] else 1
        if args.command == "validate":
            load_journey(args.journey); print(f"valid: {args.journey}"); return 0
        if args.command == "compare":
            compare_performance(args.baseline, args.candidate, args.budget, args.output); return 0
        if args.command == "publish-review":
            result = publish_review(args.receipt.resolve(), args.summary.resolve(), args.channel,
                                    args.reply_to, args.highlights.resolve() if args.highlights else None,
                                    args.mention)
            print(json.dumps(result, indent=2)); return 0
        if args.command == "finding-bundle":
            result = finding_bundle(args.receipt.resolve(), args.output.resolve(), match=args.match, context=args.context,
                                    start=args.start, duration=args.duration)
            print(json.dumps({"output": str(args.output.resolve()), "manifest": result}, indent=2)); return 0
        if args.command == "benchmark":
            if args.runs < PERFORMANCE_SAMPLE_MINIMUM:
                raise HarnessError(f"benchmark requires at least {PERFORMANCE_SAMPLE_MINIMUM} runs")
            receipts = [run_journey(args.journey, args.relay, args.output) / "receipt.json" for _ in range(args.runs)]
            print(json.dumps({"receipts": [str(path) for path in receipts]}, indent=2)); return 0
        run_journey(args.journey, args.relay, args.output); return 0
    except (HarnessError, EvidenceError, PublishError) as exc:
        print(f"native-review: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
