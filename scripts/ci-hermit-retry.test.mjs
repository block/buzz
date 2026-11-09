import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

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

test("relay setup retries downloads, checks both tools, and publishes only complete setup", () => {
  for (const scenario of [
    "success",
    "recover",
    "unavailable",
    "corrupt-hermit",
    "corrupt-pgschema",
  ]) {
    const dir = mkdtempSync(join(tmpdir(), "ci-relay-tools-"));
    try {
      mkdirSync(join(dir, "bin"));
      mkdirSync(join(dir, "scripts"));
      const hermit = gzipSync(`#!/bin/bash
case "$*" in
  *"info --json pgschema") cat "$RUNNER_TEMP/metadata.json" ;;
  *"env --raw") echo RELAY_TOOLS_READY=1 ;;
  *) exit 2 ;;
esac
`);
      const pgschema = Buffer.from(
        '#!/bin/bash\ntouch "$RUNNER_TEMP/pgschema-ran"\n',
      );
      const corrupt = '#!/bin/bash\ntouch "$RUNNER_TEMP/corrupt-ran"\n';
      writeFileSync(join(dir, "hermit.gz"), hermit);
      writeFileSync(join(dir, "pgschema"), pgschema);
      writeFileSync(join(dir, "corrupt-hermit.gz"), gzipSync(corrupt));
      writeFileSync(join(dir, "corrupt-pgschema"), corrupt);
      writeFileSync(
        join(dir, "metadata.json"),
        JSON.stringify([
          {
            Source: "https://example.org/pgschema-1.7.4-linux-amd64",
            SHA256: createHash("sha256").update(pgschema).digest("hex"),
          },
        ]),
      );
      const setup = readFileSync(
        new URL("./ci-relay-tools.sh", import.meta.url),
        "utf8",
      ).replace(
        "ef781d50bbfa57c8b0f03bb7f442c953a00a5726e9758d19f6c549ffd3e6e676",
        createHash("sha256").update(hermit).digest("hex"),
      );
      // Use fixture artifact checksums, but run the real verifier and installers.
      writeFileSync(join(dir, "scripts/ci-relay-tools.sh"), setup);
      writeFileSync(
        join(dir, "scripts/ci-hermit-retry.sh"),
        readFileSync(script),
      );
      writeFileSync(
        join(dir, "bin/hermit"),
        readFileSync(new URL("../bin/hermit", import.meta.url)),
      );
      writeFileSync(
        join(dir, "bin/curl"),
        `#!/bin/bash
for argument in "$@"; do
  case "$argument" in https:*) url="$argument" ;; esac
done
while [ "$1" != -o ]; do shift; done
destination=$2
echo "$url" >> "$RUNNER_TEMP/requests"
if [ "$SCENARIO" = unavailable ] || { [ "$SCENARIO" = recover ] && [ "$(wc -l < "$RUNNER_TEMP/requests")" -eq 1 ]; }; then
  exit 22
fi
case "$url" in
  *hermit-linux-amd64.gz*)
    artifact=hermit.gz
    [ "$SCENARIO" != corrupt-hermit ] || artifact=corrupt-hermit.gz ;;
  *)
    artifact=pgschema
    [ "$SCENARIO" != corrupt-pgschema ] || artifact=corrupt-pgschema ;;
esac
cp "$RUNNER_TEMP/$artifact" "$destination"
`,
      );
      for (const path of [
        "scripts/ci-relay-tools.sh",
        "scripts/ci-hermit-retry.sh",
        "bin/hermit",
        "bin/curl",
      ]) {
        chmodSync(join(dir, path), 0o755);
      }
      const envFile = join(dir, "github-env");
      writeFileSync(envFile, "EXISTING=1\n");
      const result = spawnSync(
        "bash",
        ["scripts/ci-hermit-retry.sh", "scripts/ci-relay-tools.sh"],
        {
          cwd: dir,
          encoding: "utf8",
          timeout: 30_000,
          env: {
            ...process.env,
            PATH: `${join(dir, "bin")}:${process.env.PATH}`,
            RUNNER_TEMP: dir,
            GITHUB_ENV: envFile,
            SCENARIO: scenario,
            CI_HERMIT_ATTEMPTS: "2",
            CI_HERMIT_ATTEMPT_SECONDS: "10",
            CI_HERMIT_KILL_AFTER_SECONDS: "1",
            CI_HERMIT_BACKOFF_SECONDS: "0",
          },
        },
      );
      const succeeds = scenario === "success" || scenario === "recover";
      assert.equal(
        result.status,
        succeeds ? 0 : 1,
        result.stdout + result.stderr,
      );
      assert.equal(
        existsSync(join(dir, "corrupt-ran")),
        false,
        "unverified code executed",
      );
      assert.equal(existsSync(join(dir, "pgschema-ran")), succeeds);
      assert.equal(
        readFileSync(envFile, "utf8"),
        succeeds
          ? `EXISTING=1\nHERMIT_EXE=${dir}/buzz-relay-tools/hermit\nHERMIT_SELF_UPDATE=false\nPGSCHEMA_BIN=${dir}/buzz-relay-tools/pgschema\nRELAY_TOOLS_READY=1\n`
          : "EXISTING=1\n",
      );
      const requests = readFileSync(join(dir, "requests"), "utf8")
        .trim()
        .split("\n");
      assert.equal(
        requests.length,
        {
          success: 2,
          recover: 3,
          unavailable: 2,
          "corrupt-hermit": 2,
          "corrupt-pgschema": 4,
        }[scenario],
      );
      assert.equal(
        new Set(requests).size,
        requests.length,
        "retries reused a failed download URL",
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
});
