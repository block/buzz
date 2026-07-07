import assert from "node:assert/strict";
import test from "node:test";

import { resolveSearchHitDestination } from "./resolveSearchHitDestination.ts";

test("resolveSearchHitDestination routes chat hits to chats", async () => {
  const destination = await resolveSearchHitDestination({
    channelId: "chat-123",
    channelType: "chat",
    eventId: "event-456",
    kind: 9,
  });

  assert.deepEqual(destination, {
    kind: "chat",
    chatId: "chat-123",
    messageId: "event-456",
  });
});
