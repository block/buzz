import assert from "node:assert/strict";
import test from "node:test";

import { getInstallErrorMessage } from "./installError.ts";

test("getInstallErrorMessage: empty steps array returns fallback", () => {
  assert.equal(getInstallErrorMessage([]), "Install failed with no output.");
});

test("getInstallErrorMessage: failed step without hint contains step name and stderr", () => {
  const message = getInstallErrorMessage([
    {
      step: "adapter",
      command: "npm install -g @block/buzz-acp",
      success: false,
      stdout: "",
      stderr: "EACCES: permission denied",
      exitCode: 1,
    },
  ]);
  assert.match(message, /Step "adapter" failed:/);
  assert.match(message, /EACCES: permission denied/);
});

test("getInstallErrorMessage: failed step without hint does not contain hint-ish text", () => {
  const message = getInstallErrorMessage([
    {
      step: "adapter",
      command: "npm install -g @block/buzz-acp",
      success: false,
      stdout: "",
      stderr: "EACCES: permission denied",
      exitCode: 1,
    },
  ]);
  assert.doesNotMatch(message, /npm config set prefix/);
});

test("getInstallErrorMessage: failed step with hint starts with hint and still contains stderr", () => {
  const hint =
    "Fix the npm prefix ownership:\n  sudo chown -R $USER $(npm config get prefix)";
  const message = getInstallErrorMessage([
    {
      step: "adapter",
      command: "npm install -g @block/buzz-acp",
      success: false,
      stdout: "",
      stderr: "EACCES: permission denied, mkdir '/usr/local/lib'",
      exitCode: 1,
      hint,
    },
  ]);
  assert.ok(message.startsWith(hint), "message should start with hint");
  assert.match(message, /EACCES: permission denied/);
});

test("getInstallErrorMessage: failed step with empty stderr falls back to stdout", () => {
  const message = getInstallErrorMessage([
    {
      step: "node",
      command: "node --version",
      success: false,
      stdout: "some stdout output",
      stderr: "",
      exitCode: 1,
    },
  ]);
  assert.match(message, /some stdout output/);
});

test("getInstallErrorMessage: hint and step detail are separated by double newline for whitespace-pre-line rendering", () => {
  const hint = "Git Bash is required. Install it from git-scm.com.";
  const message = getInstallErrorMessage([
    {
      step: "shell",
      command: "bash -l -c 'npm install'",
      success: false,
      stdout: "",
      stderr: "bash: command not found",
      exitCode: 127,
      hint,
    },
  ]);
  assert.ok(
    message.includes("\n\n"),
    "hint and step detail should be separated by a blank line",
  );
  assert.ok(message.startsWith(hint));
});

test("getInstallErrorMessage: only reports the last (failing) step when multiple steps present", () => {
  const message = getInstallErrorMessage([
    {
      step: "node",
      command: "node --version",
      success: true,
      stdout: "v20.0.0",
      stderr: "",
      exitCode: 0,
    },
    {
      step: "adapter",
      command: "npm install -g @agentclientprotocol/claude-code-acp",
      success: false,
      stdout: "",
      stderr: "npm ERR! code E404",
      exitCode: 1,
    },
  ]);
  assert.match(message, /Step "adapter" failed:/);
  assert.match(message, /npm ERR! code E404/);
  assert.doesNotMatch(message, /Step "node"/);
});
