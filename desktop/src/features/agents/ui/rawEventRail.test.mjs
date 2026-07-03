import assert from "node:assert/strict";
import test from "node:test";

import { describeRawEvent } from "./agentSessionTranscriptHelpers.ts";

function rawEvent(overrides = {}) {
  return {
    seq: 1,
    kind: "acp",
    sessionId: "sess-1",
    channelId: "channel-1",
    payload: {},
    ...overrides,
  };
}

test("describeRawEvent surfaces the session/update sessionUpdate label", () => {
  const event = rawEvent({
    payload: {
      method: "session/update",
      params: { update: { sessionUpdate: "agent_message_chunk" } },
    },
  });
  assert.equal(describeRawEvent(event), "agent_message_chunk");
});

test("describeRawEvent falls back to the method when session/update lacks an update label", () => {
  const event = rawEvent({
    payload: { method: "session/update", params: {} },
  });
  assert.equal(describeRawEvent(event), "session/update");
});

test("describeRawEvent uses the method for non-session/update payloads", () => {
  const event = rawEvent({ payload: { method: "session/prompt" } });
  assert.equal(describeRawEvent(event), "session/prompt");
});

test("describeRawEvent falls back to the event kind when no method is present", () => {
  const event = rawEvent({ kind: "acp_parse_error", payload: {} });
  assert.equal(describeRawEvent(event), "acp_parse_error");
});
