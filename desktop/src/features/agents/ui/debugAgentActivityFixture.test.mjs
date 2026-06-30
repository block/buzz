import assert from "node:assert/strict";
import test from "node:test";

import { DEBUG_AGENT_ACTIVITY_FIXTURE } from "./debugAgentActivityFixture.ts";
import {
  DEBUG_AGENT_ACTIVITY_RAW_EVENTS,
  DEBUG_AGENT_ACTIVITY_TRANSCRIPT,
} from "./debugAgentActivityRawFixture.ts";

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

test("debug raw activity fixture aligns with transcript fixture", () => {
  assert.equal(
    DEBUG_AGENT_ACTIVITY_RAW_EVENTS.length,
    DEBUG_AGENT_ACTIVITY_FIXTURE.length,
  );

  for (const [index, event] of DEBUG_AGENT_ACTIVITY_RAW_EVENTS.entries()) {
    const item = DEBUG_AGENT_ACTIVITY_FIXTURE[index];
    assert.equal(event.seq, index + 1);
    assert.equal(event.timestamp, item.timestamp);
    assert.equal(event.channelId, item.channelId ?? null);
    assert.equal(event.sessionId, item.sessionId ?? null);
    assert.equal(event.turnId, item.turnId ?? null);
  }
});

test("debug transcript fixture shows progressive plan upserts", () => {
  const planItems = DEBUG_AGENT_ACTIVITY_TRANSCRIPT.filter(
    (item) => item.type === "plan",
  );
  const plan = planItems.find((item) => !item.isUpdate);
  const updates = planItems.filter((item) => item.isUpdate);

  assert.ok(plan);
  assert.match(plan.text, /5\. \[x\] Report the pushed SHA/);
  assert.deepEqual(
    updates.map((item) => item.text),
    ["2/4 complete", "3/4 complete", "4/5 complete", "5/5 complete"],
  );
});
