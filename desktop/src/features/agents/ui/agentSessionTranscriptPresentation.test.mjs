import assert from "node:assert/strict";
import test from "node:test";

import {
  getActivityHeadline,
  isMeaningfulItem,
} from "./agentSessionTranscriptPresentation.ts";

const baseTimestamp = "2026-06-14T19:00:00.000Z";

function makeTool(overrides = {}) {
  return {
    id: "tool:1",
    type: "tool",
    title: "Send Message",
    toolName: "send_message",
    buzzToolName: "send_message",
    status: "executing",
    args: { channel_id: "abc" },
    result: "",
    isError: false,
    timestamp: baseTimestamp,
    startedAt: baseTimestamp,
    completedAt: null,
    ...overrides,
  };
}

function makeMessage(overrides = {}) {
  return {
    id: "msg:1",
    type: "message",
    role: "assistant",
    title: "Assistant",
    text: "Looking into that now.",
    timestamp: baseTimestamp,
    ...overrides,
  };
}

test("getActivityHeadline formats tool titles and assistant text", () => {
  assert.equal(getActivityHeadline(makeTool()), "Send Message");
  assert.equal(
    getActivityHeadline(makeMessage({ text: "First line\nSecond line" })),
    "First line",
  );
  assert.equal(getActivityHeadline(makeMessage({ text: "   " })), "Responding");
});

test("isMeaningfulItem ignores lifecycle noise and metadata", () => {
  assert.equal(
    isMeaningfulItem({
      id: "life:1",
      type: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: baseTimestamp,
    }),
    false,
  );
  assert.equal(
    isMeaningfulItem({
      id: "meta:1",
      type: "metadata",
      title: "Prompt context",
      sections: [],
      timestamp: baseTimestamp,
    }),
    false,
  );
  assert.equal(
    isMeaningfulItem({
      id: "life:2",
      type: "lifecycle",
      title: "Turn error",
      text: "boom",
      timestamp: baseTimestamp,
    }),
    true,
  );
});
