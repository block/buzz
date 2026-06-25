import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTranscriptPresentation,
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

test("buildTranscriptPresentation marks running tools as active while working", () => {
  const items = [
    makeMessage({ id: "msg:user", role: "user", text: "Please help" }),
    makeTool({ id: "tool:running", status: "executing" }),
  ];

  const presentation = buildTranscriptPresentation(items, true);

  assert.equal(presentation.state, "tool_running");
  assert.equal(presentation.headline, "Send Message");
  assert.equal(presentation.counts.tools, 1);
  assert.equal(presentation.counts.messages, 1);
  assert.ok(presentation.activeItemIds.has("tool:running"));
});

test("buildTranscriptPresentation highlights assistant streaming while working", () => {
  const items = [
    makeMessage({ id: "msg:assistant", role: "assistant", text: "Drafting" }),
  ];

  const presentation = buildTranscriptPresentation(items, true);

  assert.equal(presentation.state, "responding");
  assert.equal(presentation.headline, "Drafting");
  assert.ok(presentation.activeItemIds.has("msg:assistant"));
});

test("buildTranscriptPresentation surfaces lifecycle errors", () => {
  const items = [
    makeTool({
      id: "tool:done",
      status: "completed",
      completedAt: "2026-06-14T19:00:05.000Z",
    }),
    {
      id: "life:error",
      type: "lifecycle",
      title: "Turn error",
      text: "timeout",
      timestamp: "2026-06-14T19:00:06.000Z",
    },
  ];

  const presentation = buildTranscriptPresentation(items, false);

  assert.equal(presentation.state, "error");
  assert.equal(presentation.hasError, true);
  assert.equal(presentation.headline, "Turn error");
});

test("buildTranscriptPresentation returns idle state when not working", () => {
  const items = [
    makeTool({
      id: "tool:done",
      status: "completed",
      completedAt: "2026-06-14T19:00:05.000Z",
    }),
  ];

  const presentation = buildTranscriptPresentation(items, false);

  assert.equal(presentation.state, "idle");
  assert.equal(presentation.activeItemIds.size, 0);
  assert.equal(presentation.headline, "Send Message");
});
