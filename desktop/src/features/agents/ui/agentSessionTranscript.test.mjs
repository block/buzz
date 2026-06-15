import assert from "node:assert/strict";
import test from "node:test";

import { buildTranscript } from "./agentSessionTranscript.ts";

const turnId = "turn-abc";
const sessionId = "sess-1";
const channelId = "channel-1";
const baseTimestamp = "2026-06-14T22:20:23.000Z";

function makeTurnEvents() {
  return [
    {
      seq: 1,
      timestamp: baseTimestamp,
      kind: "turn_started",
      agentIndex: 0,
      channelId,
      sessionId: null,
      turnId,
      payload: { triggeringEventIds: ["event-1"] },
    },
    {
      seq: 2,
      timestamp: baseTimestamp,
      kind: "session_resolved",
      agentIndex: 0,
      channelId,
      sessionId,
      turnId,
      payload: { sessionId, isNewSession: false },
    },
    {
      seq: 3,
      timestamp: baseTimestamp,
      kind: "acp_write",
      agentIndex: 0,
      channelId,
      sessionId,
      turnId,
      payload: {
        method: "session/prompt",
        params: {
          sessionId,
          prompt: [
            {
              type: "text",
              text: "[Buzz event: message]\nContent: @Ned deliberate, wider pass\nFrom: Tyler hex: abcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd",
            },
          ],
        },
      },
    },
    {
      seq: 4,
      timestamp: "2026-06-14T22:20:47.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId,
      sessionId,
      turnId,
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "agent_message_chunk",
            messageId: "msg-1",
            content: [{ type: "text", text: "On it." }],
          },
        },
      },
    },
  ];
}

test("buildTranscript attaches turnId and sessionId to generated items", () => {
  const items = buildTranscript(makeTurnEvents());

  assert.ok(items.length >= 4);
  for (const item of items) {
    assert.equal(item.turnId, turnId);
    assert.equal(item.channelId, channelId);
  }

  const sessionResolved = items.find(
    (item) =>
      item.type === "lifecycle" && item.acpSource === "session_resolved",
  );
  assert.equal(sessionResolved?.sessionId, sessionId);

  const userPrompt = items.find(
    (item) =>
      item.type === "message" &&
      item.role === "user" &&
      item.acpSource === "session/prompt:user",
  );
  assert.ok(userPrompt);
  assert.equal(userPrompt.sessionId, sessionId);
});

test("buildTranscript tags assistant chunks with agent_message_chunk", () => {
  const items = buildTranscript([
    {
      seq: 1,
      timestamp: "2026-06-14T20:47:14.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "agent_message_chunk",
            messageId: "msg-1",
            content: [{ type: "text", text: "Marge is summoned." }],
          },
        },
      },
    },
  ]);

  assert.equal(items.length, 1);
  assert.equal(items[0]?.type, "message");
  assert.equal(items[0]?.acpSource, "agent_message_chunk");
});

test("buildTranscript tags thought chunks with agent_thought_chunk", () => {
  const items = buildTranscript([
    {
      seq: 2,
      timestamp: "2026-06-14T20:47:15.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "agent_thought_chunk",
            messageId: "thought-1",
            content: [{ type: "text", text: "Considering next step." }],
          },
        },
      },
    },
  ]);

  assert.equal(items.length, 1);
  assert.equal(items[0]?.type, "thought");
  assert.equal(items[0]?.acpSource, "agent_thought_chunk");
});
