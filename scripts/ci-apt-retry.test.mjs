import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./ci-apt-retry.sh", import.meta.url));

// The wrapper and these fakes rely on GNU timeout, util-linux flock/setsid and
// procps pkill/pgrep. CI runs this suite on Ubuntu; skip it elsewhere.
const skip =
  process.platform !== "linux" &&
  "needs Linux (GNU timeout, flock, setsid, procps)";

// Run the wrapper with a fake `sudo` on PATH that records each call instead
// of touching the system's apt or dpkg state.
function run(args, { PATH: extraPath, ...env } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "ci-apt-retry-"));
  const calls = join(dir, "calls");
  writeFileSync(calls, "");
  writeFileSync(
    join(dir, "sudo"),
    `#!/usr/bin/env bash
case "$1" in
  tee) cat > "${dir}/conf" ;;
  pkill) "$@" ;;
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
      PATH: [dir, extraPath, process.env.PATH].filter(Boolean).join(":"),
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

test("success runs the command once and writes apt options", { skip }, () => {
  const r = run(["sudo", "true"]);
  assert.equal(r.status, 0);
  assert.deepEqual(r.calls, ["cmd"]);
  assert.match(r.conf, /Acquire::https::Timeout "30";/);
});

test("a failed attempt is repaired and retried", { skip }, () => {
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

test("persistent failure stops after the attempt budget, without a final repair", { skip }, () => {
  const r = run(["sudo", "false"]);
  assert.equal(r.status, 1);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd", "repair", "cmd"]);
  assert.match(r.stdout, /::error::all 3 attempts failed/);
});

test("a stalled attempt is killed at its deadline and retried", { skip }, () => {
  const r = run(["sudo", "sleep", "30"], { CI_APT_ATTEMPTS: "2" });
  assert.equal(r.status, 1);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd"]);
  assert.match(r.stdout, /timed out after 1s/);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});

test("a stalled dpkg repair is bounded and the next attempt still runs", { skip }, () => {
  const r = run(["sudo", "bash", "-c", "exit 23"], {
    CI_APT_ATTEMPTS: "2",
    STALL_REPAIR: "1",
  });
  assert.equal(r.status, 1);
  assert.deepEqual(r.calls, ["cmd", "repair", "cmd"]);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});

test("no command is a usage error", { skip }, () => {
  assert.equal(run([]).status, 2);
});

// playwright install-deps reaches apt-get through `sudo sh -c "..."`, and sudo
// runs that in a new process group, so timeout(1)'s group signal misses it.
// Model that: the first attempt starts an `apt-get` in its own session that
// holds a lock and blocks; the attempt's own process exits on TERM. The retry
// must find the lock free, so the survivor has to have been stopped.
// Writes a fake `apt-get` that takes a lock: the first call holds it and
// blocks, later calls succeed only if the lock is free.
function escapingAptGet() {
  const dir = mkdtempSync(join(tmpdir(), "ci-apt-retry-escape-"));
  const lock = join(dir, "lock");
  writeFileSync(
    join(dir, "apt-get"),
    // #!/bin/bash, not env, so the process is named apt-get.
    `#!/bin/bash
exec 9>"${lock}"
flock -n 9 || { echo "lock held"; exit 1; }
[ -e "${dir}/seen" ] && exit 0
touch "${dir}/seen"
mkfifo "${dir}/fifo"; exec 8<>"${dir}/fifo"
read -r -t 30 -u 8 || true
`,
  );
  chmodSync(join(dir, "apt-get"), 0o755);
  const lockIsFree = () =>
    spawnSync("flock", ["-n", lock, "true"]).status === 0;
  return { dir, lockIsFree };
}

test("a package manager that escapes the attempt's process group is stopped before the retry", { skip }, () => {
  const { dir } = escapingAptGet();
  const r = run(["bash", "-c", "setsid apt-get & wait $!"], {
    CI_APT_ATTEMPTS: "2",
    PATH: dir,
  });
  assert.equal(r.status, 0, r.stdout);
  assert.match(r.stdout, /attempt 1\/2 timed out/);
  assert.doesNotMatch(r.stdout, /lock held/);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});

test("a package manager that escapes the final attempt is stopped before the wrapper exits", { skip }, () => {
  const { dir, lockIsFree } = escapingAptGet();
  const r = run(["bash", "-c", "setsid apt-get & wait $!"], {
    CI_APT_ATTEMPTS: "1",
    PATH: dir,
  });
  assert.equal(r.status, 1, r.stdout);
  assert.deepEqual(r.calls, []);
  assert.ok(lockIsFree(), "apt-get outlived the wrapper");
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});
