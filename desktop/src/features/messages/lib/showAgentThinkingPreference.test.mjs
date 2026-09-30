import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_SHOW_AGENT_THINKING,
  parseShowAgentThinking,
} from "./showAgentThinkingPreference.ts";

test("parseShowAgentThinking defaults to on", () => {
  assert.equal(DEFAULT_SHOW_AGENT_THINKING, true);
  assert.equal(parseShowAgentThinking(null), true);
  assert.equal(parseShowAgentThinking(undefined), true);
  assert.equal(parseShowAgentThinking(""), true);
});

test("parseShowAgentThinking reads explicit booleans", () => {
  assert.equal(parseShowAgentThinking("true"), true);
  assert.equal(parseShowAgentThinking("false"), false);
});
