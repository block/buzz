import assert from "node:assert/strict";
import test from "node:test";

import {
  claimUnhydratedRenderScopedReactionIds,
  collectRenderScopedReactionMessageIds,
  hydrateRenderScopedReactions,
  releaseRenderScopedReactionIds,
  resetRenderScopedReactionHydration,
} from "./renderScopedReactions.ts";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { channelMessagesKey } from "./messageQueryKeys.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";

function hex(char) {
  return char.repeat(64);
}

function event(id, kind, overrides = {}) {
  return {
    id,
    pubkey: hex("a"),
    kind,
    created_at: 1_700_000_000,
    content: "",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function entry(message) {
  return { message, summary: null };
}

function makeQueryClientStub(initialEvents = []) {
  const store = new Map([
    [JSON.stringify(channelMessagesKey(CHANNEL_ID)), initialEvents],
  ]);
  return {
    getQueryData(key) {
      return store.get(JSON.stringify(key));
    },
    setQueryData(key, updater) {
      const k = JSON.stringify(key);
      const next =
        typeof updater === "function" ? updater(store.get(k) ?? []) : updater;
      store.set(k, next);
      return next;
    },
  };
}

test.afterEach(() => {
  resetRenderScopedReactionHydration();
});

test("collects main timeline and open thread messages without hidden replies", () => {
  const main = event(hex("1"), 9);
  const hiddenCollapsedReply = event(hex("2"), 9, {
    tags: [["e", main.id]],
  });
  const threadHead = event(hex("3"), 9);
  const visibleThreadReply = event(hex("4"), 9, {
    tags: [["e", threadHead.id]],
  });

  assert.deepEqual(
    collectRenderScopedReactionMessageIds({
      mainEntries: [entry(main), entry(threadHead)],
      threadHeadMessage: threadHead,
      threadEntries: [entry(visibleThreadReply)],
    }),
    [main.id, threadHead.id, visibleThreadReply.id],
  );

  assert.ok(
    !collectRenderScopedReactionMessageIds({
      mainEntries: [entry(main), entry(threadHead)],
      threadHeadMessage: threadHead,
      threadEntries: [entry(visibleThreadReply)],
    }).includes(hiddenCollapsedReply.id),
  );
});

test("claims each rendered message id once per channel and can retry released ids", () => {
  assert.deepEqual(
    claimUnhydratedRenderScopedReactionIds(CHANNEL_ID, [
      hex("1"),
      hex("2"),
      hex("1"),
    ]),
    [hex("1"), hex("2")],
  );
  assert.deepEqual(
    claimUnhydratedRenderScopedReactionIds(CHANNEL_ID, [hex("1"), hex("2")]),
    [],
  );
  assert.deepEqual(
    claimUnhydratedRenderScopedReactionIds("other-channel", [hex("1")]),
    [hex("1")],
  );

  releaseRenderScopedReactionIds(CHANNEL_ID, [hex("2")]);
  assert.deepEqual(
    claimUnhydratedRenderScopedReactionIds(CHANNEL_ID, [hex("1"), hex("2")]),
    [hex("2")],
  );
});

test("hydrates visible reactions into the channel timeline cache", async () => {
  const messageId = hex("1");
  const reactionId = hex("2");
  const currentUser = hex("c");
  const message = event(messageId, 9, {
    pubkey: hex("a"),
    content: "ship it?",
  });
  const reaction = event(reactionId, 7, {
    pubkey: currentUser,
    content: "✅",
    tags: [["e", messageId]],
  });
  const queryClient = makeQueryClientStub([message]);
  const calls = [];

  await hydrateRenderScopedReactions({
    channelId: CHANNEL_ID,
    messageIds: [messageId],
    queryClient,
    deps: {
      fetchReactionEventsForMessages: async (channelId, messageIds) => {
        calls.push({ channelId, messageIds });
        return [reaction];
      },
    },
  });

  assert.deepEqual(calls, [{ channelId: CHANNEL_ID, messageIds: [messageId] }]);
  const cached = queryClient.getQueryData(channelMessagesKey(CHANNEL_ID));
  assert.ok(cached.some((e) => e.id === reactionId));

  const timeline = formatTimelineMessages(cached, null, currentUser, null);
  assert.deepEqual(
    timeline
      .find((m) => m.id === messageId)
      ?.reactions?.map((r) => ({
        count: r.count,
        emoji: r.emoji,
        mine: r.reactedByCurrentUser,
      })),
    [{ count: 1, emoji: "✅", mine: true }],
  );
});

test("failed hydration releases ids so the next render can retry", async () => {
  const messageId = hex("1");
  const queryClient = makeQueryClientStub([event(messageId, 9)]);

  await hydrateRenderScopedReactions({
    channelId: CHANNEL_ID,
    messageIds: [messageId],
    queryClient,
    deps: {
      fetchReactionEventsForMessages: async () => {
        throw new Error("relay timeout");
      },
    },
  });

  assert.deepEqual(
    claimUnhydratedRenderScopedReactionIds(CHANNEL_ID, [messageId]),
    [messageId],
  );
});
