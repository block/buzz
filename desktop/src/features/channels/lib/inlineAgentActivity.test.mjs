import assert from "node:assert/strict";
import test from "node:test";

import { buildInlineAgentActivityPlacement } from "./inlineAgentActivity.ts";

function toolItem({
  channelId = "channel-1",
  id,
  result = "",
  turnId = "turn-1",
  toolName = "check_messages",
}) {
  return {
    id,
    type: "tool",
    renderClass: toolName === "send_message" ? "message" : "relay-op",
    descriptor: {
      label: toolName,
      preview: null,
      renderClass: toolName === "send_message" ? "message" : "relay-op",
    },
    title: toolName,
    toolName,
    buzzToolName: toolName,
    status: "completed",
    args: { channel_id: channelId },
    result,
    isError: false,
    timestamp: "2026-08-04T18:00:00.000Z",
    startedAt: "2026-08-04T18:00:00.000Z",
    completedAt: "2026-08-04T18:00:01.000Z",
    channelId,
    sessionId: "session-1",
    turnId,
  };
}

test("keeps an active turn at the live timeline tail", () => {
  const placement = buildInlineAgentActivityPlacement({
    channelId: "channel-1",
    isWorking: true,
    renderedMessageIds: new Set(),
    transcript: [toolItem({ id: "read-1" })],
  });

  assert.equal(placement?.anchorMessageId, null);
  assert.deepEqual(
    placement?.items.map((item) => item.id),
    ["read-1"],
  );
});

test("anchors the trace before the exact message returned by send_message", () => {
  const placement = buildInlineAgentActivityPlacement({
    channelId: "channel-1",
    isWorking: false,
    renderedMessageIds: new Set(["message-42"]),
    transcript: [
      toolItem({ id: "read-1" }),
      toolItem({
        id: "send-1",
        result: JSON.stringify({ accepted: true, event_id: "message-42" }),
        toolName: "send_message",
      }),
    ],
  });

  assert.equal(placement?.anchorMessageId, "message-42");
  assert.deepEqual(
    placement?.items.map((item) => item.id),
    ["read-1", "send-1"],
  );
});

test("does not leave an unanchored trace behind after a turn completes", () => {
  const placement = buildInlineAgentActivityPlacement({
    channelId: "channel-1",
    isWorking: false,
    renderedMessageIds: new Set(),
    transcript: [
      toolItem({
        id: "send-1",
        result: JSON.stringify({ accepted: true, event_id: "message-42" }),
        toolName: "send_message",
      }),
    ],
  });

  assert.equal(placement, null);
});
