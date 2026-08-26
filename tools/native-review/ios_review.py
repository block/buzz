#!/usr/bin/env python3
"""Run a Flutter integration journey on an isolated iOS Simulator with video evidence."""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import re
import secrets
import selectors
import shutil
import signal
import subprocess
import sys

_TOOL_ROOT = pathlib.Path(__file__).resolve().parent
if str(_TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(_TOOL_ROOT))
import time
from typing import Any

from evidence_bundle import EvidenceError, relay_safe_video

ROOT = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_TEST = ROOT / "mobile/integration_test/native_review_pairing_test.dart"
IOS_BUNDLE_ID = "com.buzz.buzzMobile"
RECORDING_READY_MARKER = "BUZZ_NATIVE_REVIEW_RECORDING_READY"
SUBPROCESS_ENV_ALLOWLIST = {
    "PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "SHELL", "USER", "LOGNAME",
    "TERM", "__CF_USER_TEXT_ENCODING", "DEVELOPER_DIR",
}


class ReviewError(RuntimeError):
    pass


def run(command: list[str], *, cwd: pathlib.Path = ROOT, check: bool = True, capture: bool = True,
        env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, check=check, text=True,
                          env=subprocess_environment() if env is None else env,
                          stdout=subprocess.PIPE if capture else None,
                          stderr=subprocess.PIPE if capture else None)


def git(*args: str) -> str:
    return run(["git", *args]).stdout.strip()


def create_review_device(device_type_name: str, run_id: str) -> dict[str, Any]:
    """Create a uniquely named simulator owned by this run; never reuse user state."""
    payload = json.loads(run(["xcrun", "simctl", "list", "devicetypes", "runtimes", "-j"]).stdout)
    device_types = [item for item in payload.get("devicetypes", []) if item.get("name") == device_type_name]
    if not device_types:
        raise ReviewError(f"no iOS Simulator device type named {device_type_name}")
    device_type = device_types[0]
    runtimes = [
        item for item in payload.get("runtimes", [])
        if item.get("isAvailable") and item.get("platform") == "iOS"
        and any(supported.get("identifier") == device_type["identifier"]
                for supported in item.get("supportedDeviceTypes", []))
    ]
    if not runtimes:
        raise ReviewError(f"no available iOS Simulator runtime supports {device_type_name}")
    runtime = max(runtimes, key=lambda item: tuple(int(part) for part in item["version"].split(".")))
    owned_name = f"Buzz Native Review {run_id}"
    udid = run(["xcrun", "simctl", "create", owned_name,
                device_type["identifier"], runtime["identifier"]]).stdout.strip()
    if not re.fullmatch(r"[0-9A-Fa-f-]{36}", udid):
        raise ReviewError("simctl create returned an invalid device identifier")
    return {"name": owned_name, "udid": udid, "runtimeIdentifier": runtime["identifier"],
            "deviceType": device_type_name, "owned": True}


def subprocess_environment() -> dict[str, str]:
    return {key: value for key, value in os.environ.items() if key in SUBPROCESS_ENV_ALLOWLIST}


def flutter_environment() -> dict[str, str]:
    env = subprocess_environment()
    env.update({"BUZZ_NATIVE_REVIEW": "1", "SIMCTL_CHILD_BUZZ_NATIVE_REVIEW": "1"})
    return env


def provenance() -> dict[str, Any]:
    status = git("status", "--porcelain=v1", "--untracked-files=all")
    return {"head_sha": git("rev-parse", "HEAD"), "dirty": bool(status), "status": status.splitlines()}


def wait_for_recording(recorder: subprocess.Popen[str], timeout_seconds: float = 15) -> None:
    if recorder.stderr is None:
        raise ReviewError("simulator recorder has no diagnostic stream")
    selector = selectors.DefaultSelector()
    selector.register(recorder.stderr, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout_seconds
    try:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ReviewError("timed out waiting for Simulator recording")
            if not selector.select(remaining):
                raise ReviewError("timed out waiting for Simulator recording")
            line = recorder.stderr.readline()
            if "Recording started" in line:
                return
            if recorder.poll() is not None:
                raise ReviewError(f"simulator recorder exited: {line.strip()}")
    finally:
        selector.close()


def wait_for_recording_ready(process: subprocess.Popen[str], log: pathlib.Path,
                             timeout_seconds: float = 180) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if log.is_file() and RECORDING_READY_MARKER in log.read_text(errors="replace"):
            return
        if process.poll() is not None:
            raise ReviewError("Flutter integration journey exited before recording readiness")
        time.sleep(0.1)
    raise ReviewError("timed out waiting for Flutter recording readiness")


def start_flutter_review(test: pathlib.Path, udid: str, log: pathlib.Path) -> tuple[subprocess.Popen[str], Any]:
    log_handle = log.open("w")
    command = ["flutter", "drive", "--driver", "test_driver/integration_test.dart",
               "--target", str(test.relative_to(ROOT / "mobile")), "-d", udid,
               "--keep-app-running", "--dart-define=BUZZ_NATIVE_REVIEW=true"]
    try:
        process = subprocess.Popen(command, cwd=ROOT / "mobile", stdout=log_handle,
                                   stderr=subprocess.STDOUT, env=flutter_environment(), text=True)
    except Exception:
        log_handle.close()
        raise
    return process, log_handle


def run_review(test: pathlib.Path, device_name: str, output_root: pathlib.Path) -> pathlib.Path:
    if sys.platform != "darwin" or not shutil.which("xcrun") or not shutil.which("flutter"):
        raise ReviewError("iOS native review requires macOS, Xcode simctl, and Flutter")
    if not test.is_file() or ROOT not in test.resolve().parents:
        raise ReviewError(f"test must be a repository integration test: {test}")
    prov = provenance()
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    run_id = f"ios-{dt.datetime.now().strftime('%Y%m%dT%H%M%S')}-{secrets.token_hex(3)}"
    flow = f"ios_{test.stem}"
    run_dir = output_root / git("rev-parse", "--short=12", "HEAD") / flow / run_id
    run_dir.mkdir(parents=True)
    receipt: dict[str, Any] = {
        "schema_version": 1, "run_id": run_id, "flow": flow, "status": "failed",
        "started_at": started, "finished_at": started, "failure": None,
        "provenance": prov,
        "isolation": {"kind": "disposable_ios_simulator", "owned": True},
        "artifacts": {}, "steps": [], "measurements": {},
        "performance": {"machine": {
            "system": sys.platform, "release": "ios-simulator",
            "machine": os.uname().machine, "cpu": os.uname().machine,
        }},
        "cleanup": {"status": "not_started"},
    }
    device: dict[str, Any] | None = None
    udid: str | None = None
    recorder: subprocess.Popen[str] | None = None
    flutter_process: subprocess.Popen[str] | None = None
    flutter_log: Any | None = None
    try:
        device = create_review_device(device_name, run_id)
        udid = device["udid"]
        receipt["isolation"]["simulator"] = {
            "name": device["name"], "device_type": device_name, "udid": udid,
            "runtime": device["runtimeIdentifier"], "owned": True,
        }
        run(["xcrun", "simctl", "boot", udid])
        run(["xcrun", "simctl", "bootstatus", udid, "-b"], capture=False)
        log = run_dir / "flutter.log"
        flutter_process, flutter_log = start_flutter_review(test, udid, log)
        receipt["artifacts"]["log"] = "flutter.log"
        wait_for_recording_ready(flutter_process, log)
        run(["xcrun", "simctl", "launch", udid, IOS_BUNDLE_ID])
        video = run_dir / "video.mp4"
        recorder = subprocess.Popen(["xcrun", "simctl", "io", udid, "recordVideo", "--codec=h264", "--force", str(video)],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                    env=subprocess_environment(), text=True)
        wait_for_recording(recorder)
        receipt["artifacts"]["video"] = "video.mp4"
        returncode = flutter_process.wait()
        flutter_log.close()
        flutter_log = None
        screenshot = run_dir / "final.png"
        run(["xcrun", "simctl", "io", udid, "screenshot", str(screenshot)])
        receipt["artifacts"]["screenshot"] = "final.png"
        time.sleep(0.5)
        if returncode:
            raise ReviewError(f"Flutter integration journey failed with exit {returncode}")
        receipt["status"] = "passed"
    except Exception as exc:
        receipt["failure"] = str(exc)
    finally:
        errors = []
        if flutter_process and flutter_process.poll() is None:
            flutter_process.terminate()
            try:
                flutter_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                flutter_process.kill()
                flutter_process.wait(timeout=5)
        if flutter_log:
            flutter_log.close()
        if recorder and recorder.poll() is None:
            recorder.send_signal(signal.SIGINT)
            try:
                recorder.wait(timeout=30)
            except subprocess.TimeoutExpired:
                recorder.kill(); errors.append("recorder required SIGKILL")
        if udid:
            try:
                run(["xcrun", "simctl", "shutdown", udid], check=False)
            except Exception as exc:
                errors.append(str(exc))
            try:
                run(["xcrun", "simctl", "delete", udid])
            except Exception as exc:
                errors.append(str(exc))
        video = run_dir / "video.mp4"
        if video.is_file():
            try:
                relay_safe_video(video, run_dir / "video-share.mp4")
                receipt["artifacts"]["share_video"] = "video-share.mp4"
            except EvidenceError as exc:
                errors.append(f"shareable video finalization failed: {exc}")
        receipt["cleanup"] = {"status": "failed" if errors else "passed", "errors": errors}
        if errors:
            receipt["status"] = "failed"
        receipt["finished_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
        (run_dir / "receipt.json").write_text(json.dumps(receipt, indent=2, allow_nan=False) + "\n")
    print(run_dir)
    if receipt["status"] != "passed":
        raise ReviewError(receipt["failure"] or "iOS journey or cleanup failed")
    return run_dir


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test", type=pathlib.Path, default=DEFAULT_TEST)
    parser.add_argument("--device", default="iPhone 17 Pro")
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "test-results/native-review")
    args = parser.parse_args()
    try:
        run_review(args.test.resolve(), args.device, args.output.resolve())
        return 0
    except ReviewError as exc:
        print(f"ios-native-review: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
