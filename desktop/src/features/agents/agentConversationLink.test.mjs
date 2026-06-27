import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAgentConversationLink,
  parseAgentConversationLink,
} from "./agentConversationLink.ts";

test("buildAgentConversationLink -> parseAgentConversationLink round-trips", () => {
  const href = buildAgentConversationLink({
    agentReplyId: "reply-1",
    channelId: "channel-1",
  });

  assert.deepEqual(parseAgentConversationLink(href), {
    ok: true,
    value: {
      agentReplyId: "reply-1",
      channelId: "channel-1",
    },
  });
});

test("parseAgentConversationLink rejects missing required params", () => {
  assert.deepEqual(parseAgentConversationLink("buzz://task?channel=c1"), {
    ok: false,
    reason: "missing-reply",
  });
  assert.deepEqual(parseAgentConversationLink("buzz://task?reply=m1"), {
    ok: false,
    reason: "missing-channel",
  });
});
