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
  assert.equal(getActivityHeadline(makeTool()), "Send Message · abc");
  assert.equal(
    getActivityHeadline(makeMessage({ text: "First line\nSecond line" })),
    "First line",
  );
  assert.equal(getActivityHeadline(makeMessage({ text: "   " })), "Responding");
});

test("isMeaningfulItem ignores lifecycle noise and raw JSON-RPC metadata", () => {
  assert.equal(
    isMeaningfulItem({
      id: "life:1",
      type: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: baseTimestamp,
    }),
    false,
    "turn started is lifecycle noise → not meaningful",
  );
  assert.equal(
    isMeaningfulItem({
      id: "meta:raw",
      type: "metadata",
      renderClass: "raw-rail",
      title: "Raw ACP payload",
      sections: [],
      timestamp: baseTimestamp,
      acpSource: "raw_json_rpc",
    }),
    false,
    "raw_json_rpc metadata is infrastructure noise → not meaningful",
  );
  assert.equal(
    isMeaningfulItem({
      id: "meta:ctx",
      type: "metadata",
      renderClass: "raw-rail",
      title: "Prompt context",
      sections: [],
      timestamp: baseTimestamp,
      acpSource: "session/prompt:context",
    }),
    true,
    "prompt context metadata is semantic → meaningful",
  );
  assert.equal(
    isMeaningfulItem({
      id: "meta:sys",
      type: "metadata",
      renderClass: "raw-rail",
      title: "System prompt",
      sections: [],
      timestamp: baseTimestamp,
    }),
    true,
    "system prompt metadata (no acpSource) is semantic → meaningful",
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

test("getActivityHeadline uses semantic tool descriptors", () => {
  assert.equal(
    getActivityHeadline(
      makeTool({
        title: "Shell",
        toolName: "dev__shell",
        buzzToolName: null,
        args: { command: "buzz messages send --content hi" },
        descriptor: {
          renderClass: "message",
          label: "Send Message",
          preview: "hi",
          source: "shell",
          groupKey: "buzz-cli:messages.send",
        },
      }),
    ),
    "Send Message · hi",
  );
});

test("isMeaningfulItem ignores suppressed tools", () => {
  assert.equal(
    isMeaningfulItem(
      makeTool({
        renderClass: "suppressed",
        descriptor: {
          renderClass: "suppressed",
          label: "Checked todos",
          preview: null,
          source: "harness",
          groupKey: "suppressed:stop-hook",
        },
      }),
    ),
    false,
  );
});
