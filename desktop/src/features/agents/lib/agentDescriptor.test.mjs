import assert from "node:assert/strict";
import test from "node:test";

import { resolveAgentDescriptor } from "./agentDescriptor.ts";

test("prefers and normalizes an explicit one-line description", () => {
  assert.equal(
    resolveAgentDescriptor("  Turns ideas\ninto finished work.  ", "ignored"),
    "Turns ideas into finished work.",
  );
});

test("falls back to the first instruction sentence for legacy agents", () => {
  assert.equal(
    resolveAgentDescriptor(
      null,
      "You are a careful researcher. Compare sources and cite evidence.",
    ),
    "You are a careful researcher.",
  );
});

test("keeps an instruction without sentence punctuation", () => {
  assert.equal(
    resolveAgentDescriptor(undefined, "Help the team ship"),
    "Help the team ship",
  );
});

test("keeps every card populated when both fields are blank", () => {
  assert.equal(resolveAgentDescriptor(null, "   "), "No description yet");
});
