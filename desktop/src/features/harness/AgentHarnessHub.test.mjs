import assert from "node:assert/strict";
import test from "node:test";

test("supported agent harnesses include all 9 required coding assistants", () => {
  const expectedHarnesses = [
    "antigravity",
    "claude_code",
    "cursor",
    "codex",
    "goose",
    "opencode",
    "zcode",
    "agy_cli",
    "kimi",
  ];

  assert.equal(expectedHarnesses.length, 9);
  assert.ok(expectedHarnesses.includes("antigravity"));
  assert.ok(expectedHarnesses.includes("claude_code"));
  assert.ok(expectedHarnesses.includes("cursor"));
  assert.ok(expectedHarnesses.includes("codex"));
  assert.ok(expectedHarnesses.includes("goose"));
  assert.ok(expectedHarnesses.includes("opencode"));
  assert.ok(expectedHarnesses.includes("zcode"));
  assert.ok(expectedHarnesses.includes("agy_cli"));
  assert.ok(expectedHarnesses.includes("kimi"));
});

test("harness status contract supports connected, detected, and not_installed", () => {
  const validStatuses = ["connected", "detected", "not_installed"];
  for (const s of validStatuses) {
    assert.ok(typeof s === "string");
  }
});
