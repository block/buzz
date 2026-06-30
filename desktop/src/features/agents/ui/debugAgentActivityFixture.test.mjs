import assert from "node:assert/strict";
import test from "node:test";

import { DEBUG_AGENT_ACTIVITY_FIXTURE } from "./debugAgentActivityFixture.ts";

const expectedRenderClasses = new Set([
  "message",
  "relay-op",
  "file-edit",
  "shell",
  "status",
  "thought",
  "plan",
  "permission",
  "error",
  "generic",
  "raw-rail",
  "suppressed",
]);

const expectedItemTypes = new Set([
  "message",
  "thought",
  "plan",
  "lifecycle",
  "metadata",
  "tool",
]);

test("debug activity fixture covers every render class and item variant", () => {
  const renderClasses = new Set(
    DEBUG_AGENT_ACTIVITY_FIXTURE.map((item) => item.renderClass),
  );
  const itemTypes = new Set(
    DEBUG_AGENT_ACTIVITY_FIXTURE.map((item) => item.type),
  );

  assert.deepEqual(renderClasses, expectedRenderClasses);
  assert.deepEqual(itemTypes, expectedItemTypes);
});
