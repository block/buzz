import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./ci-apt-retry.sh", import.meta.url));

// Run the wrapper with a fake `sudo` on PATH that records each call instead
// of touching the system's apt or dpkg state.
function run(args, env = {}) {
  const dir = mkdtempSync(join(tmpdir(), "ci-apt-retry-"));
  const calls = join(dir, "calls");
  writeFileSync(calls, "");
  writeFileSync(
    join(dir, "sudo"),
    `#!/usr/bin/env bash
case "$1" in
  tee) cat > "${dir}/conf" ;;
  dpkg) echo repair >> "${calls}"; [ -n "\${STALL_REPAIR:-}" ] && sleep 30; true ;;
  *) echo cmd >> "${calls}"; "$@" ;;
esac
`,
  );
  chmodSync(join(dir, "sudo"), 0o755);
  const started = Date.now();
  const result = spawnSync("bash", [script, ...args], {
    encoding: "utf8",
    timeout: 20_000,
    env: {
      ...process.env,
      PATH: `${dir}:${process.env.PATH}`,
      CI_APT_ATTEMPT_SECONDS: "1",
      CI_APT_REPAIR_SECONDS: "1",
      ...env,
    },
  });
  return {
    status: result.status,
    seconds: (Date.now() - started) / 1000,
    calls: readFileSync(calls, "utf8").split("\n").filter(Boolean),
    conf: (() => {
      try {
        return readFileSync(join(dir, "conf"), "utf8");
      } catch {
        return "";
      }
    })(),
    stdout: result.stdout,
  };
}

test("success runs the command once and writes apt options", () => {
  const r = run(["sudo", "true"]);
  assert.equal(r.status, 0);
  assert.deepEqual(r.calls, ["cmd"]);
  assert.match(r.conf, /Acquire::https::Timeout "30";/);
});

test("a failed attempt is repaired and retried", () => {
  const dir = mkdtempSync(join(tmpdir(), "ci-apt-retry-flaky-"));
  const marker = join(dir, "seen");
  const r = run([
    "sudo",
    "bash",
    "-c",
    `[ -e ${marker} ] || { touch ${marker}; exit 1; }`,
  ]);
  assert.equal(r.status, 0);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd"]);
});

test("persistent failure stops after the attempt budget, without a final repair", () => {
  const r = run(["sudo", "false"]);
  assert.equal(r.status, 1);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd", "repair", "cmd"]);
  assert.match(r.stdout, /::error::all 3 attempts failed/);
});

test("a stalled attempt is killed at its deadline and retried", () => {
  const r = run(["sudo", "sleep", "30"], { CI_APT_ATTEMPTS: "2" });
  assert.equal(r.status, 1);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd"]);
  assert.match(r.stdout, /timed out after 1s/);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});

test("a stalled dpkg repair is bounded and the next attempt still runs", () => {
  const r = run(["sudo", "bash", "-c", "exit 23"], {
    CI_APT_ATTEMPTS: "2",
    STALL_REPAIR: "1",
  });
  assert.equal(r.status, 1);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd"]);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});

test("no command is a usage error", () => {
  assert.equal(run([]).status, 2);
});
