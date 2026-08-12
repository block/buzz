import assert from "node:assert/strict";
import test from "node:test";

import { mergeFullChannelContext } from "./useInboxThreadContext.ts";

const channelId = "11111111-1111-1111-1111-111111111111";

function message(id, created_at, tags = [["h", channelId]]) {
  return {
    content: id,
    created_at,
    id,
    kind: 9,
    pubkey: "agent-or-member",
    sig: "",
    tags,
  };
}

test("DM Inbox context retains a fetched NIP-10 reply omitted from the channel window", () => {
  const incoming = message("incoming", 10);
  // The channel-window projection excludes this ordinary, non-broadcast
  // reply. The DM context fetch must put it back into the flat conversation.
  const agentReply = message("agent-reply", 11, [
    ["h", channelId],
    ["e", "incoming", "", "reply"],
  ]);
  const newerTopLevelMessage = message("newer-message", 12);

  const context = mergeFullChannelContext(
    incoming,
    [incoming, newerTopLevelMessage],
    [agentReply],
  );

  assert.deepEqual(
    context.map((event) => event.id),
    ["incoming", "agent-reply", "newer-message"],
  );
});
