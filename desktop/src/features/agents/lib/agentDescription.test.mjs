import assert from "node:assert/strict";
import test from "node:test";

import {
  agentDescriptionCharacterCount,
  clampAgentDescription,
  effectiveAgentDescription,
} from "./agentDescription.ts";

test("description character count matches Rust Unicode scalar counting", () => {
  assert.equal(agentDescriptionCharacterCount("a🐝é"), 3);
  assert.equal(agentDescriptionCharacterCount("🐝".repeat(280)), 280);
});

test("description clamp preserves a useful prefix for over-cap pastes", () => {
  assert.equal(clampAgentDescription("a".repeat(300)), "a".repeat(280));
  assert.equal(
    clampAgentDescription(`${"a".repeat(279)}🐝extra`),
    `${"a".repeat(279)}🐝`,
  );
});

test("an authored description wins", () => {
  assert.equal(
    effectiveAgentDescription({ description: "Reviews desktop PRs." }),
    "Reviews desktop PRs.",
  );
});

test("an authored description is trimmed", () => {
  assert.equal(
    effectiveAgentDescription({ description: "  Reviews desktop PRs.  " }),
    "Reviews desktop PRs.",
  );
});

test("blank, whitespace-only, and missing descriptions yield null", () => {
  assert.equal(effectiveAgentDescription({ description: "" }), null);
  assert.equal(effectiveAgentDescription({ description: "   " }), null);
  assert.equal(effectiveAgentDescription({ description: null }), null);
  assert.equal(effectiveAgentDescription({}), null);
});
