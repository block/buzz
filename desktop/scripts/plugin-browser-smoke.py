#!/usr/bin/env python3
"""Tier 2 orchestration for `just plugin-browser-smoke`.

Builds the external example plugin, then builds and launches the real Buzz
**debug** binary with the e2e frontend bundled in and
`BUZZ_PLUGIN_BROWSER_SMOKE=1` in its environment. The frontend's own
`smokeDriver.e2e.ts` self-drives Settings install through uninstall from
inside the running app and reports each step to this script's fixture
server; this script only starts that server, launches/waits on the app, and
asserts every expected step arrived — including the real HTTP fetches the
native webview must have performed, not just the driver's own markers.

Every launch runs under a freshly generated app identity (Tauri
`identifier`/`productName` plus the compile-time `BUZZ_BUILD_DEMO_SLUG`
mechanism `desktop/src-tauri/src/build_identity.rs` already uses for named
demo builds) so this script never reads, writes, or clears the real user's
Buzz application state (identity key, keyring entry, config/data
directories, WebView storage). `BUZZ_PLUGIN_BROWSER_SMOKE_ROOT` alone only
isolates the plugin registry; it does not isolate the rest of app state.

See docs/plugin-browser-prototype.md for the full runtime validation plan.
By default this script first runs tier 1 (the harness-free native smoke
test, `cargo test --test browser_native_smoke`) to completion, then tier 2
(this file's own build-and-drive orchestration) — the two never run
concurrently, since both open a real GUI webview and share the same Cargo
target dir. Pass `--skip-native` to skip tier 1 during tier-2-only
debugging; CI and the `just plugin-browser-smoke` recipe always run both.
"""

import argparse
import contextlib
import http.server
import json
import os
import plistlib
import secrets
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DESKTOP_DIR = REPO_ROOT / "desktop"
TAURI_DIR = DESKTOP_DIR / "src-tauri"
EXAMPLE_DIR = REPO_ROOT / "examples" / "buzz-plugin-browser-example"

EXPECTED_DRIVER_STEPS = [
    "home-loaded",
    "refusal-visible-and-preserved",
    "refusal-unchanged-url",
    "destination-loaded",
    "disabled",
    "uninstalled",
]
EXPECTED_FETCHES = ["home", "destination"]

# Reported by the /home fixture page's own getUserMedia probe (embedded JS,
# not the frontend driver). "rejected" is the only acceptable outcome — the
# native adapter's `with_permission_handler` must always deny; "granted" or
# "timeout" (the callback never fired/settled) both fail the run.
EXPECTED_PERMISSION_KINDS = ["audio", "video"]
# /home loads exactly twice in this flow: the initial navigation, then the
# driver's Reload click during the refusal check. Each load re-runs the
# probe and appends a report — a count that differs (more or fewer) means
# either a load never probed or an extra, unexpected reload happened, and
# either way must fail explicitly rather than being silently accepted.
EXPECTED_HOME_LOADS = 2

DRIVER_TIMEOUT_SECONDS = 90
APP_STARTUP_TIMEOUT_SECONDS = 30
CARGO_BUILD_TIMEOUT_SECONDS = 900
NATIVE_TEST_TIMEOUT_SECONDS = 300
PROCESS_TABLE_TIMEOUT_SECONDS = 10
PROCESS_TABLE_MAX_BYTES = 4 * 1024 * 1024


def run_native_smoke_test() -> None:
    """Runs tier 1 (`cargo test --test browser_native_smoke`) to completion.

    Sequential, not concurrent, with tier 2's own build/launch below — both
    open a real GUI webview and share the same Cargo target dir.
    """
    run(
        ["cargo", "test", "--test", "browser_native_smoke", "--", "--nocapture"],
        cwd=str(TAURI_DIR),
        timeout=NATIVE_TEST_TIMEOUT_SECONDS,
    )


class FixtureState:
    """Tracks driver-reported steps and the real GET fetches separately.

    A POST marker only proves the driver *believes* a step happened; the GET
    record proves the native webview actually issued the request the step
    claims. Both are required before the smoke run is considered complete.
    """

    def __init__(self):
        self.lock = threading.Lock()
        self.steps = []
        self.fetches = []
        # kind -> ordered list of {"outcome": str, "load": int} reports.
        # Never overwritten or reset across /home reloads — evidence is only
        # ever appended, so an earlier failure can't be masked by a later
        # "rejected" report, and a caller (the frontend driver) can tell a
        # fresh report from a previous load's already-recorded one by
        # comparing counts/load numbers rather than mere key presence.
        self.permission_reports = {}
        self.home_load_count = 0

    def record_step(self, step: str) -> None:
        with self.lock:
            if step not in self.steps:
                self.steps.append(step)

    def record_fetch(self, name: str) -> None:
        with self.lock:
            if name not in self.fetches:
                self.fetches.append(name)

    def next_home_load(self) -> int:
        with self.lock:
            self.home_load_count += 1
            return self.home_load_count

    def record_permission(self, kind: str, outcome: str, load: int) -> None:
        with self.lock:
            self.permission_reports.setdefault(kind, []).append(
                {"outcome": outcome, "load": load}
            )

    def snapshot(self):
        with self.lock:
            return (
                list(self.steps),
                list(self.fetches),
                {kind: list(reports) for kind, reports in self.permission_reports.items()},
            )


def make_handler(state: FixtureState):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            sys.stderr.write("[fixture] " + (fmt % args) + "\n")

        def _cors_headers(self) -> None:
            # The driver script runs in the main Tauri webview (a
            # tauri://localhost / http://tauri.localhost origin) and fetches
            # this fixture's http://127.0.0.1:<port> origin directly to post
            # markers — a genuinely cross-origin request. Without an
            # Access-Control-Allow-Origin response header, `fetch()` rejects
            # before the driver's own try/catch can tell this script apart
            # from a real failure.
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Content-Type")

        def _send_body(self, body: bytes, content_type: str) -> None:
            self.send_response(200)
            self._cors_headers()
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _send_html(self, title: str) -> None:
            body = f"<html><head><title>{title}</title></head><body>{title}</body></html>".encode()
            self._send_body(body, "text/html; charset=utf-8")

        def _send_home_page(self, load: int) -> None:
            # Probes real camera/mic denial from inside the native adapter's
            # webview — a source-level assertion that `with_permission_handler`
            # returns `Deny` proves nothing about runtime behavior. Each kind
            # reports exactly once: whichever of "granted"/"rejected"/"no-api"
            # settles first, or "timeout" if the promise never settles within
            # 5s (a hang is itself a failure, not silence). `load` (this
            # page-load's `home_load_count`) rides along on every report so a
            # caller that reloads /home can tell a fresh report from a
            # previous load's already-recorded one.
            body = f"""<html><head><title>home</title></head><body>home
<script>
(function () {{
  var LOAD = {load};
  function report(kind, outcome) {{
    fetch("/driver/permission-report?kind=" + kind + "&outcome=" + outcome + "&load=" + LOAD, {{
      method: "POST",
    }}).catch(function () {{}});
  }}
  function probe(kind) {{
    var settled = false;
    function settle(outcome) {{
      if (settled) return;
      settled = true;
      report(kind, outcome);
    }}
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {{
      settle("no-api");
      return;
    }}
    var constraints = kind === "audio" ? {{ audio: true }} : {{ video: true }};
    navigator.mediaDevices
      .getUserMedia(constraints)
      .then(function (stream) {{
        stream.getTracks().forEach(function (track) {{
          track.stop();
        }});
        settle("granted");
      }})
      .catch(function () {{
        settle("rejected");
      }});
    setTimeout(function () {{
      settle("timeout");
    }}, 5000);
  }}
  probe("audio");
  probe("video");
}})();
</script>
</body></html>""".encode()
            self._send_body(body, "text/html; charset=utf-8")

        def do_OPTIONS(self):
            self.send_response(204)
            self._cors_headers()
            self.end_headers()

        def do_GET(self):
            if self.path == "/home":
                state.record_fetch("home")
                self._send_home_page(state.next_home_load())
            elif self.path == "/destination":
                state.record_fetch("destination")
                self._send_html("destination")
            elif self.path == "/driver-status":
                steps, fetches, permission_reports = state.snapshot()
                # Summarized per kind so a caller can wait deterministically
                # (e.g. "count === 2, every outcome rejected") without
                # reimplementing the accumulate-never-reset semantics
                # client-side. Counts/outcomes accumulate across the whole
                # run and are never reset between /home loads.
                permissions = {
                    kind: {
                        "count": len(reports),
                        "outcomes": [report["outcome"] for report in reports],
                    }
                    for kind, reports in permission_reports.items()
                }
                body = json.dumps(
                    {"steps": steps, "fetches": fetches, "permissions": permissions}
                ).encode()
                self._send_body(body, "application/json")
            else:
                self.send_response(404)
                self._cors_headers()
                self.end_headers()

        def do_POST(self):
            if self.path.startswith("/driver/permission-report"):
                query = urllib.parse.urlparse(self.path).query
                params = urllib.parse.parse_qs(query)
                kind = (params.get("kind") or [""])[0]
                outcome = (params.get("outcome") or [""])[0]
                load = (params.get("load") or [""])[0]
                length = int(self.headers.get("Content-Length", 0) or 0)
                if length:
                    self.rfile.read(length)
                if kind and outcome and load.isdigit():
                    state.record_permission(kind, outcome, int(load))
                self.send_response(200)
                self._cors_headers()
                self.end_headers()
            elif self.path.startswith("/driver/"):
                step = self.path[len("/driver/") :]
                length = int(self.headers.get("Content-Length", 0) or 0)
                if length:
                    self.rfile.read(length)
                state.record_step(step)
                self.send_response(200)
                self._cors_headers()
                self.end_headers()
            else:
                self.send_response(404)
                self._cors_headers()
                self.end_headers()

    return Handler


def run(cmd, cwd=None, env=None, check=True, timeout=None):
    print(f"+ {' '.join(str(part) for part in cmd)}", flush=True)
    result = subprocess.run(cmd, cwd=cwd, env=env, timeout=timeout)
    if check and result.returncode != 0:
        raise SystemExit(f"command failed ({result.returncode}): {cmd}")
    return result.returncode


INFO_PLIST_PATH = TAURI_DIR / "Info.plist"


def wrap_in_minimal_app_bundle(binary: Path, workdir: Path, identifier: str) -> Path:
    """Wraps the bare `cargo build` executable in a minimal `.app` bundle.

    A plain `cargo build` executable is not a macOS application bundle: it
    has no Info.plist, so it carries none of the real bundle's
    `NSCameraUsageDescription`/`NSMicrophoneUsageDescription` entries
    (`desktop/src-tauri/Info.plist`, embedded only by the Tauri bundler,
    never by `cargo build`). This gives the launched process the same
    Info.plist/usage-string parity with the real shipped app that the bare
    executable lacks — it does not by itself establish what effect, if any,
    that parity has on getUserMedia()/TCC behavior; compare actual outcomes
    rather than assuming one. Reuses the real Info.plist verbatim except for
    `CFBundleExecutable`/`CFBundleIdentifier`, which must match this run's
    isolated identity.
    """
    app_dir = workdir / "BuzzPluginBrowserSmoke.app"
    macos_dir = app_dir / "Contents" / "MacOS"
    macos_dir.mkdir(parents=True, exist_ok=True)
    bundled_binary = macos_dir / binary.name
    shutil.copyfile(binary, bundled_binary)
    bundled_binary.chmod(0o755)

    plist = plistlib.loads(INFO_PLIST_PATH.read_bytes())
    plist["CFBundleExecutable"] = binary.name
    plist["CFBundleIdentifier"] = identifier
    plist["CFBundlePackageType"] = "APPL"
    (app_dir / "Contents").mkdir(parents=True, exist_ok=True)
    with open(app_dir / "Contents" / "Info.plist", "wb") as handle:
        plistlib.dump(plist, handle)

    return bundled_binary


def build_debug_binary(env: dict) -> Path:
    """Builds the debug binary and returns its executable path.

    Parses `--message-format=json` compiler-artifact output rather than
    assuming `target/debug/buzz-desktop` — a workspace or CI
    `CARGO_TARGET_DIR`/`CARGO_BUILD_TARGET_DIR` override would silently make
    that assumption point at a stale or nonexistent path.
    """
    cmd = ["cargo", "build", "--message-format=json-render-diagnostics"]
    print(f"+ {' '.join(cmd)}", flush=True)
    proc = subprocess.run(
        cmd,
        cwd=str(TAURI_DIR),
        env=env,
        capture_output=True,
        text=True,
        timeout=CARGO_BUILD_TIMEOUT_SECONDS,
    )
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        raise SystemExit(f"cargo build failed ({proc.returncode})")

    executable = None
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            message.get("reason") == "compiler-artifact"
            and message.get("executable")
            and message.get("target", {}).get("name") == "buzz-desktop"
        ):
            executable = Path(message["executable"])
    if executable is None or not executable.exists():
        raise SystemExit(
            "cargo build produced no `buzz-desktop` compiler-artifact executable"
        )
    return executable


def run_bounded_stdout(cmd: list, timeout: float, max_bytes: int) -> str:
    """Runs `cmd`, capturing stdout with both a wall-clock and byte cap.

    A stalled child is killed at `timeout` (raises `subprocess.TimeoutExpired`).
    A child producing more than `max_bytes` of stdout is killed immediately
    rather than buffered without bound (raises `RuntimeError`). EOF on stdout
    does not by itself mean the child has exited (it may have closed stdout
    and kept running), so the post-EOF reap is itself bounded by the same
    deadline. On every exit path — including an unexpected select/read error —
    a still-running child is killed and reaped before this function returns,
    so no owned child outlives the call.
    """
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + timeout
    chunks = []
    total = 0
    try:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(cmd, timeout)
            ready, _, _ = select.select([proc.stdout], [], [], min(remaining, 0.5))
            if not ready:
                continue
            chunk = os.read(proc.stdout.fileno(), 65536)
            if not chunk:
                break
            total += len(chunk)
            if total > max_bytes:
                raise RuntimeError(
                    f"{cmd[0]} produced more than {max_bytes} bytes of stdout"
                )
            chunks.append(chunk)
        try:
            returncode = proc.wait(timeout=max(deadline - time.monotonic(), 0))
        except subprocess.TimeoutExpired:
            raise subprocess.TimeoutExpired(cmd, timeout) from None
    except BaseException:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=5)
        raise
    finally:
        proc.stdout.close()
    if returncode != 0:
        raise subprocess.CalledProcessError(returncode, cmd)
    return b"".join(chunks).decode("utf-8", errors="replace")


def assert_registry_and_processes_clean(smoke_root: Path) -> None:
    """Independently verifies uninstall actually happened.

    The driver's `/driver/uninstalled` marker only proves the frontend
    *believes* uninstall finished. Check the registry file and the process
    table directly rather than trusting that self-report.
    """
    registry_path = smoke_root / "registry.json"
    if registry_path.exists():
        try:
            registry = json.loads(registry_path.read_text())
        except json.JSONDecodeError as exc:
            raise SystemExit(f"registry.json is not valid JSON after uninstall: {exc}")
        plugins = registry.get("plugins", {})
        if plugins:
            raise SystemExit(
                "registry still lists plugin(s) after the uninstall driver "
                f"marker: {list(plugins)}"
            )

    ps_stdout = run_bounded_stdout(
        ["/bin/ps", "-eo", "command"],
        timeout=PROCESS_TABLE_TIMEOUT_SECONDS,
        max_bytes=PROCESS_TABLE_MAX_BYTES,
    )
    leftover = [line for line in ps_stdout.splitlines() if str(smoke_root) in line]
    if leftover:
        raise SystemExit(
            f"plugin child process(es) still running after uninstall: {leftover}"
        )

    # Registry removal is durable and authoritative, but package directory
    # removal is a separate, fallible step (see the host's uninstall():
    # a failed `package_removal` is reported to the caller, not swallowed,
    # and is recovered by the next startup reconcile — not by this run).
    # An empty registry alone does not prove the package bytes are gone.
    leftover_files = [
        str(path)
        for path in smoke_root.rglob("*")
        if path.is_file() and path.name != "registry.json"
    ]
    if leftover_files:
        raise SystemExit(
            f"plugin package file(s) still on disk after uninstall: {leftover_files}"
        )


@contextlib.contextmanager
def bounded_cleanup(errors: list):
    """Runs one cleanup step, recording (not swallowing) any failure."""

    def add_step(label, fn):
        try:
            fn()
        except Exception as exc:  # noqa: BLE001 - cleanup must not raise mid-teardown
            errors.append(f"{label}: {exc}")

    yield add_step


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--skip-native",
        action="store_true",
        help="Skip tier 1 (browser_native_smoke). Test-only, for tier-2-only "
        "debugging; the just recipe and CI always run both.",
    )
    return parser.parse_args()


def main() -> int:
    if sys.platform != "darwin":
        print("plugin-browser-smoke: skipped, macOS only")
        return 0

    args = parse_args()
    if not args.skip_native:
        run_native_smoke_test()

    workdir = Path(tempfile.mkdtemp(prefix="buzz-plugin-browser-smoke-"))
    package_dir = workdir / "example-package"
    smoke_root = workdir / "plugin-root"
    app_log_path = workdir / "app.log"
    state = FixtureState()

    # Unique per-run identity: never touches the real user's app-data
    # directory, keyring service, deep-link scheme, or single-instance lock
    # (xyz.block.buzz.app). `demo_slug` feeds both the compile-time
    # BUZZ_BUILD_DEMO_SLUG mechanism (keyring/nest/deep-link, see
    # build_identity.rs) and a matching Tauri `identifier` override (Tauri's
    # own app_data_dir/single-instance/WebView storage), so every
    # community-scoped and Tauri-scoped path this run touches is isolated.
    demo_slug = f"browser-smoke-{secrets.token_hex(6)}"

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), make_handler(state))
    port = server.server_address[1]
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    base_url = f"http://127.0.0.1:{port}"
    print(f"fixture server listening on {base_url}", flush=True)

    app_process = None
    app_log_file = None
    cleanup_errors: list = []
    try:
        # 1. Build the external example, pointed at this fixture's /home.
        run(
            [
                sys.executable,
                str(EXAMPLE_DIR / "build.py"),
                "--output",
                str(package_dir),
                "--home-url",
                f"{base_url}/home",
            ]
        )

        # 2. Build the e2e frontend bundle, then the real debug binary.
        run(["pnpm", "build:e2e"], cwd=str(DESKTOP_DIR))

        identifier = f"xyz.block.buzz.app.smoke.{demo_slug}"
        build_env = dict(os.environ)
        build_env["BUZZ_BUILD_DEMO_SLUG"] = demo_slug
        # JSON Merge Patch (RFC 7396): setting `devUrl` to `null` deletes the
        # key from the merged config. Without this, a plain `cargo build`
        # still carries the base tauri.conf.json `devUrl` and the resulting
        # debug binary (debug_assertions is true either way) loads
        # http://localhost:1420 instead of the bundled e2e dist — a stale or
        # absent dev server there fails as "Community connection failed",
        # which looks like a product bug, not a build-mode mistake.
        build_env["TAURI_CONFIG"] = json.dumps(
            {
                "identifier": identifier,
                "productName": "Buzz Plugin Browser Smoke",
                "build": {"devUrl": None},
            }
        )
        raw_binary = build_debug_binary(build_env)
        print(f"built {raw_binary}", flush=True)
        binary = wrap_in_minimal_app_bundle(raw_binary, workdir, identifier)
        print(f"wrapped in app bundle: {binary}", flush=True)

        smoke_root.mkdir(parents=True, exist_ok=True)
        run_env = dict(os.environ)
        run_env["BUZZ_BUILD_DEMO_SLUG"] = demo_slug
        run_env["BUZZ_PLUGIN_BROWSER_SMOKE"] = "1"
        run_env["BUZZ_PLUGIN_BROWSER_SMOKE_DIR"] = str(package_dir)
        run_env["BUZZ_PLUGIN_BROWSER_SMOKE_ROOT"] = str(smoke_root)

        print(f"launching {binary} (log: {app_log_path})", flush=True)
        app_log_file = open(app_log_path, "wb")
        app_process = subprocess.Popen(
            [str(binary)],
            cwd=str(TAURI_DIR),
            env=run_env,
            stdout=app_log_file,
            stderr=subprocess.STDOUT,
        )

        deadline = time.monotonic() + DRIVER_TIMEOUT_SECONDS
        steps, fetches, permissions = [], [], {}
        while time.monotonic() < deadline:
            if app_process.poll() is not None:
                app_log_file.flush()
                tail = app_log_path.read_text(errors="replace")[-4000:]
                raise SystemExit(
                    f"app exited early with status {app_process.returncode} "
                    f"before all driver steps arrived; steps={steps} fetches={fetches} "
                    f"permissions={permissions}\n--- app log tail ---\n{tail}"
                )
            steps, fetches, permissions = state.snapshot()
            if (
                all(step in steps for step in EXPECTED_DRIVER_STEPS)
                and all(name in fetches for name in EXPECTED_FETCHES)
                and all(
                    len(permissions.get(kind, [])) >= EXPECTED_HOME_LOADS
                    for kind in EXPECTED_PERMISSION_KINDS
                )
            ):
                break
            time.sleep(0.5)
        else:
            missing_steps = [s for s in EXPECTED_DRIVER_STEPS if s not in steps]
            missing_fetches = [f for f in EXPECTED_FETCHES if f not in fetches]
            missing_permissions = [
                k
                for k in EXPECTED_PERMISSION_KINDS
                if len(permissions.get(k, [])) < EXPECTED_HOME_LOADS
            ]
            app_log_file.flush()
            tail = app_log_path.read_text(errors="replace")[-4000:]
            raise SystemExit(
                "timed out waiting for driver steps/fetches/permissions; "
                f"missing_steps={missing_steps} missing_fetches={missing_fetches} "
                f"missing_permissions={missing_permissions} saw_steps={steps} "
                f"saw_fetches={fetches} saw_permissions={permissions}\n"
                f"--- app log tail ---\n{tail}"
            )

        # Every report across the whole run must be "rejected" (evidence is
        # never overwritten or reset, see FixtureState.record_permission),
        # and the count must be exactly EXPECTED_HOME_LOADS — more reports
        # than expected is just as much a defect (an extra unaccounted
        # reload/probe) as fewer.
        bad_permissions = {
            kind: reports
            for kind, reports in permissions.items()
            if len(reports) != EXPECTED_HOME_LOADS
            or any(report["outcome"] != "rejected" for report in reports)
        }
        if bad_permissions:
            app_log_file.flush()
            tail = app_log_path.read_text(errors="replace")[-4000:]
            raise SystemExit(
                "native adapter did not deny every media permission request "
                f"as required: {bad_permissions} (expected exactly "
                f'{EXPECTED_HOME_LOADS} "rejected" report(s) per kind)\n'
                f"--- app log tail ---\n{tail}"
            )

        assert_registry_and_processes_clean(smoke_root)

        print(
            f"all driver steps observed: steps={steps} fetches={fetches} "
            f"permissions={permissions}"
        )
        return 0
    finally:
        with bounded_cleanup(cleanup_errors) as add_step:

            def stop_app():
                if app_process is not None and app_process.poll() is None:
                    app_process.send_signal(signal.SIGTERM)
                    try:
                        app_process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        app_process.kill()
                        app_process.wait(timeout=10)

            add_step("stop app process", stop_app)
            add_step(
                "close app log",
                lambda: app_log_file.close() if app_log_file else None,
            )

            def preserve_app_log():
                # `raise SystemExit` above only prints a 4000-char tail; the
                # full log lives under `workdir`, which "remove temp workdir"
                # below deletes unconditionally. Copy it to a stable path
                # first so a failed run leaves something to inspect.
                if app_log_path.exists():
                    dest = Path(tempfile.gettempdir()) / f"{demo_slug}.app.log"
                    shutil.copyfile(app_log_path, dest)
                    print(f"full app log preserved at {dest}", flush=True)

            add_step("preserve app log", preserve_app_log)
            add_step("shutdown fixture server", server.shutdown)
            add_step(
                "join fixture server thread",
                lambda: server_thread.join(timeout=5),
            )

            def remove_workdir():
                shutil.rmtree(workdir)

            add_step("remove temp workdir", remove_workdir)

        if cleanup_errors:
            for error in cleanup_errors:
                print(f"cleanup error: {error}", file=sys.stderr)
            raise SystemExit(f"{len(cleanup_errors)} cleanup step(s) failed; see above")


if __name__ == "__main__":
    sys.exit(main())
