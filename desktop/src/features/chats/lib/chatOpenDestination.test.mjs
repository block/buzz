import assert from "node:assert/strict";
import test from "node:test";

import { resolveChatOpenDestination } from "./chatOpenDestination.ts";

const CHANNEL_ID = "f570339f-8f8a-4e08-a779-8d954aa44109";

test("resolveChatOpenDestination opens chat when metadata exists", async () => {
  assert.deepEqual(
    await resolveChatOpenDestination(CHANNEL_ID, async () => ({
      channelId: CHANNEL_ID,
      authorPubkey: null,
      title: "Chat",
      defaultAgentPubkey: null,
      templateId: null,
      projectId: null,
      projectName: null,
      projectPath: null,
      projectTemplateId: null,
      sourceChannelId: null,
      sourceEventId: null,
      sourceThreadRootId: null,
      updatedAt: 1,
    })),
    { kind: "chat", chatId: CHANNEL_ID },
  );
});

test("resolveChatOpenDestination falls back to channel without metadata", async () => {
  assert.deepEqual(
    await resolveChatOpenDestination(CHANNEL_ID, async () => null),
    { kind: "channel", channelId: CHANNEL_ID },
  );
});

test("resolveChatOpenDestination falls back to channel on lookup errors", async () => {
  assert.deepEqual(
    await resolveChatOpenDestination(CHANNEL_ID, async () => {
      throw new Error("not a member");
    }),
    { kind: "channel", channelId: CHANNEL_ID },
  );
});
