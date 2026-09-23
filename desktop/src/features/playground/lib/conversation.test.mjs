import assert from "node:assert/strict";
import test from "node:test";

import {
  playgroundConversationFromRoute,
  playgroundConversationHasOpenThread,
  playgroundScreenshotAvailable,
} from "./conversation.ts";

test("screenshot is absent without a channel and present with one", () => {
  assert.equal(
    playgroundScreenshotAvailable(
      playgroundConversationFromRoute({
        selectedView: "home",
        selectedChannelId: null,
      }),
    ),
    false,
  );
  assert.equal(
    playgroundScreenshotAvailable(
      playgroundConversationFromRoute({
        selectedView: "agents",
        selectedChannelId: "hula-id",
      }),
    ),
    false,
  );
  assert.equal(
    playgroundScreenshotAvailable(
      playgroundConversationFromRoute({
        selectedView: "pin",
        selectedChannelId: null,
      }),
    ),
    false,
  );
  const channel = playgroundConversationFromRoute({
    selectedView: "channel",
    selectedChannelId: "hula-id",
  });
  assert.deepEqual(channel, { channelId: "hula-id", draftKey: "hula-id" });
  assert.equal(playgroundScreenshotAvailable(channel), true);
  const thread = playgroundConversationFromRoute({
    selectedView: "channel",
    selectedChannelId: "hula-id",
    threadId: "root-1",
  });
  assert.deepEqual(thread, {
    channelId: "hula-id",
    draftKey: "thread:root-1",
  });
  assert.equal(playgroundConversationHasOpenThread(channel), false);
  assert.equal(playgroundConversationHasOpenThread(thread), true);
  assert.equal(playgroundConversationHasOpenThread(null), false);
});

test("playgroundPinScopeKey distinguishes channel vs thread", async () => {
  const { playgroundPinScopeKey } = await import("./conversation.ts");
  assert.equal(
    playgroundPinScopeKey({ channelId: "chan-a", draftKey: "chan-a" }),
    "channel:chan-a",
  );
  assert.equal(
    playgroundPinScopeKey({
      channelId: "chan-a",
      draftKey: "thread:root-1",
    }),
    "thread:root-1",
  );
});

test("playgroundConversationFromPopout mirrors channel vs thread draft keys", async () => {
  const { playgroundConversationFromPopout } = await import(
    "./conversation.ts"
  );
  assert.equal(playgroundConversationFromPopout({}), null);
  assert.equal(playgroundConversationFromPopout({ channelId: "  " }), null);
  assert.deepEqual(playgroundConversationFromPopout({ channelId: "chan-a" }), {
    channelId: "chan-a",
    draftKey: "chan-a",
  });
  assert.deepEqual(
    playgroundConversationFromPopout({
      channelId: "chan-a",
      threadId: "root-1",
    }),
    { channelId: "chan-a", draftKey: "thread:root-1" },
  );
});
