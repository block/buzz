import assert from "node:assert/strict";
import test from "node:test";

import { runtimeLabel } from "./runtimeLabel.ts";

test("runtimeLabel names the bundled buzz-agent runtime", () => {
  assert.equal(runtimeLabel("buzz-agent"), "Buzz Agent");
});

test("runtimeLabel names Claude by catalog id, adapter command, and alias", () => {
  for (const value of [
    "claude",
    "claude-agent-acp",
    "claude-code-acp",
    "claude-code",
    "claudecode",
  ]) {
    assert.equal(runtimeLabel(value), "Claude Code", value);
  }
});

test("runtimeLabel names Codex by catalog id and adapter command", () => {
  assert.equal(runtimeLabel("codex"), "Codex");
  assert.equal(runtimeLabel("codex-acp"), "Codex");
});

test("runtimeLabel keeps the existing Goose and Aider labels", () => {
  assert.equal(runtimeLabel("goose"), "Goose");
  assert.equal(runtimeLabel("aider"), "Aider");
});

test("runtimeLabel matches a command given as a path, with an .exe suffix, or in another case", () => {
  assert.equal(runtimeLabel("/usr/local/bin/claude-agent-acp"), "Claude Code");
  assert.equal(
    runtimeLabel("C:\\Users\\me\\AppData\\buzz-agent.exe"),
    "Buzz Agent",
  );
  assert.equal(runtimeLabel("  Codex-ACP "), "Codex");
});

test("runtimeLabel falls back to the raw command for custom harnesses", () => {
  assert.equal(runtimeLabel("my-custom-harness"), "my-custom-harness");
  assert.equal(runtimeLabel("/opt/bin/my-harness"), "/opt/bin/my-harness");
});
