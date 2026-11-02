import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./ci-hermit-retry.sh", import.meta.url));

// Run the wrapper on a fake tool that counts its calls, then runs `body` with
// $n set to the call number and $dir to a scratch directory.
function run(body, env = {}) {
  const dir = mkdtempSync(join(tmpdir(), "ci-hermit-retry-"));
  const calls = join(dir, "calls");
  writeFileSync(calls, "");
  const tool = join(dir, "tool");
  writeFileSync(
    tool,
    `#!/bin/bash
dir="${dir}"
echo call >> "$dir/calls"
n=$(( $(wc -l < "$dir/calls") ))
${body}
`,
  );
  chmodSync(tool, 0o755);
  const started = Date.now();
  const result = spawnSync("bash", [script, tool], {
    encoding: "utf8",
    timeout: 20_000,
    env: {
      ...process.env,
      CI_HERMIT_ATTEMPT_SECONDS: "1",
      CI_HERMIT_KILL_AFTER_SECONDS: "1",
      CI_HERMIT_BACKOFF_SECONDS: "0",
      ...env,
    },
  });
  return {
    dir,
    status: result.status,
    seconds: (Date.now() - started) / 1000,
    calls: readFileSync(calls, "utf8").split("\n").filter(Boolean).length,
    stdout: result.stdout,
  };
}

test("a first-attempt success runs the tool once and keeps its output", () => {
  const r = run("echo v1.2.3");
  assert.equal(r.status, 0);
  assert.equal(r.calls, 1);
  assert.equal(r.stdout, "v1.2.3\n");
});

// A stalled download: the first call hangs on a child that would write a
// marker at 2s if it outlived the attempt's 1s deadline.
test("a stalled attempt is stopped with its children and retried", async () => {
  const r = run(`
if [ "$n" -eq 1 ]; then
  ( sleep 2; touch "$dir/survived" ) &
  wait
fi
echo fetched`);
  assert.equal(r.status, 0, r.stdout);
  assert.equal(r.calls, 2);
  assert.match(r.stdout, /::warning::attempt 1\/3 timed out after 1s/);
  assert.match(r.stdout, /fetched\n$/);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
  await sleep(Math.max(0, 3000 - r.seconds * 1000));
  assert.ok(
    !existsSync(join(r.dir, "survived")),
    "the stalled attempt's child outlived it",
  );
});

test("an attempt that ignores TERM is killed", () => {
  const r = run("trap '' TERM; sleep 30", { CI_HERMIT_ATTEMPTS: "1" });
  assert.equal(r.status, 1, r.stdout);
  assert.match(r.stdout, /attempt 1\/1 timed out after 1s/);
  assert.ok(r.seconds < 10, `took ${r.seconds}s`);
});

test("persistent failure backs off between attempts and stops at the budget", () => {
  const r = run("exit 7", { CI_HERMIT_BACKOFF_SECONDS: "1" });
  assert.equal(r.status, 1);
  assert.equal(r.calls, 3);
  assert.match(r.stdout, /attempt 3\/3 failed \(exit 7\)/);
  assert.match(r.stdout, /::error::all 3 attempts failed/);
  // Backoff sleeps 1s after attempt 1 and 2s after attempt 2.
  assert.ok(r.seconds >= 3, `took ${r.seconds}s`);
});

test("no command is a usage error", () => {
  assert.equal(spawnSync("bash", [script]).status, 2);
});
