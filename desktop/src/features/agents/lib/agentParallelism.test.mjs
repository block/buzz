import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_AGENT_PARALLELISM,
  resolveAgentParallelism,
  parallelismCapHint,
} from "./agentParallelism.ts";

test("parallelism uses the app default only when input and definition omit it", () => {
  assert.equal(
    resolveAgentParallelism(undefined, undefined),
    DEFAULT_AGENT_PARALLELISM,
  );
  assert.equal(
    resolveAgentParallelism(undefined, null),
    DEFAULT_AGENT_PARALLELISM,
  );
  assert.equal(resolveAgentParallelism(undefined, 4), 4);
  assert.equal(resolveAgentParallelism(2, 4), 2);
});

// ── parallelismCapHint: persona/instance hint data path ───────────────────────

test("parallelismCapHint returns null when requested is at or below the cap", () => {
  assert.equal(parallelismCapHint("OpenClaw", 5, 5), null);
  assert.equal(parallelismCapHint("OpenClaw", 5, 3), null);
  assert.equal(parallelismCapHint("OpenClaw", 5, 1), null);
});

test("parallelismCapHint returns hint string when requested exceeds cap", () => {
  const hint = parallelismCapHint("OpenClaw", 5, 10);
  assert.ok(hint !== null, "hint must be non-null when 10 > 5");
  assert.ok(hint.includes("OpenClaw"), "hint must include the harness label");
  assert.ok(hint.includes("5"), "hint must include the cap value");
});

test("parallelismCapHint uses singular form for cap of 1", () => {
  const hint = parallelismCapHint("SomeHarness", 1, 5);
  assert.ok(hint !== null);
  assert.ok(
    hint.includes("conversation") && !hint.includes("conversations"),
    "cap=1 must use singular 'conversation'",
  );
});

// Stored-10 OpenClaw → Goose: hint clears when the harness has no cap.
// The UI derives: if selectedRuntime.maxParallelism is undefined, hint is null.
// This test validates the helper contract that makes that work.
test("parallelismCapHint returns null when cap equals or exceeds any common parallelism value", () => {
  // Simulates an uncapped harness: the caller passes a very large cap
  // OR simply doesn't call parallelismCapHint at all (guarded by maxParallelism check).
  // When stored=10 and harness switches to goose (no cap), no hint is shown.
  assert.equal(parallelismCapHint("Goose", 32, 10), null);
  assert.equal(parallelismCapHint("Goose", 32, 32), null);
});
