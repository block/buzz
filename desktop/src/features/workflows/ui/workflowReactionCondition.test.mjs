import assert from "node:assert/strict";
import test from "node:test";

import { reactionConditionValue } from "./workflowReactionCondition.ts";

test("merges a legacy reaction emoji with a separate structured filter", () => {
  const author = "a".repeat(64);
  assert.equal(
    reactionConditionValue({
      on: "reaction_added",
      emoji: "🔥",
      filter: `trigger_author == "${author}"`,
    }),
    `trigger_emoji == "🔥" && trigger_author == "${author}"`,
  );
});

test("groups a legacy filter that has top-level alternatives", () => {
  const a = "a".repeat(64);
  const b = "b".repeat(64);
  assert.equal(
    reactionConditionValue({
      on: "reaction_added",
      emoji: "✅",
      filter: `trigger_author == "${a}" || trigger_author == "${b}"`,
    }),
    `trigger_emoji == "✅" && (trigger_author == "${a}" || trigger_author == "${b}")`,
  );
});

test("does not add redundant grouping to an already-parenthesized filter", () => {
  const a = "a".repeat(64);
  const b = "b".repeat(64);
  assert.equal(
    reactionConditionValue({
      on: "reaction_added",
      emoji: "✅",
      filter: `(trigger_author == "${a}" || trigger_author == "${b}")`,
    }),
    `trigger_emoji == "✅" && (trigger_author == "${a}" || trigger_author == "${b}")`,
  );
});

test("leaves non-reaction filters unchanged", () => {
  assert.equal(
    reactionConditionValue({ on: "message_posted", filter: "trigger_text" }),
    "trigger_text",
  );
});
