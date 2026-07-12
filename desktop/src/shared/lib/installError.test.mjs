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
