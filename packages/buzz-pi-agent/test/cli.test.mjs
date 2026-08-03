import assert from "node:assert/strict";
import { access, mkdtemp, stat } from "node:fs/promises";
import { constants } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { execFile, spawn } from "node:child_process";
import { test } from "node:test";

const execFileAsync = promisify(execFile);
const cli = new URL("../dist/cli.js", import.meta.url);

test("built CLI is executable and supports clean help/version PATH probes", async () => {
  await access(cli, constants.X_OK);
  assert.ok((await stat(cli)).mode & 0o111);
  const version = await execFileAsync(cli.pathname, ["--version"]);
  assert.equal(version.stdout, "0.1.0\n");
  assert.equal(version.stderr, "");
  const help = await execFileAsync(cli.pathname, ["--help"]);
  assert.match(help.stdout, /Serve ACP over stdin\/stdout/);
  assert.equal(help.stderr, "");
});

test("built CLI completes a real ACP initialize/shutdown stdio exchange", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-cli-smoke-"));
  const child = spawn(cli.pathname, [], {
    env: {
      ...process.env,
      PI_CODING_AGENT_DIR: join(root, "pi-agent"),
      BUZZ_PI_STATE_DIR: join(root, "state"),
      BUZZ_PI_LOG_LEVEL: "error",
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.stdin.end(
    `${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: 2 } })}\n` +
      `${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "shutdown", params: {} })}\n`,
  );
  const exit = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  assert.deepEqual(exit, { code: 0, signal: null }, stderr);
  const messages = stdout
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.equal(messages[0].id, 1);
  assert.equal(messages[0].result.agentInfo.name, "buzz-pi-agent");
  assert.deepEqual(messages[1], {
    jsonrpc: "2.0",
    id: 2,
    result: { shutdown: true },
  });
});
