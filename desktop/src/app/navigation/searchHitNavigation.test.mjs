import assert from "node:assert/strict";
import test from "node:test";

import {
  activateDesktopNotificationTarget,
  createDesktopNotificationActivationQueue,
} from "../AppShell.helpers.ts";

const { clearSearchHitEventCache, getCachedSearchHitEvent } = await import(
  "./searchHitEventCache.ts"
);
const { openSearchHitWithNavigation } = await import(
  "./searchHitNavigation.ts"
);

const forumComment = {
  eventId: "comment",
  content: "reply",
  kind: 45003,
  pubkey: "author",
  channelId: "old-community-channel",
  channelName: "forum",
  createdAt: 1,
  score: 0,
  threadRootId: null,
};

const plainMessage = {
  ...forumComment,
  eventId: "message",
  kind: 9,
  threadRootId: "thread-root",
};

test("search-hit navigation preserves forced message routing while active", async () => {
  clearSearchHitEventCache();
  const calls = [];
  const result = await openSearchHitWithNavigation(plainMessage, {
    force: true,
    goChannel: async (channelId, options) => {
      calls.push({ channelId, options });
      return true;
    },
    goForumPost: async () => false,
  });

  assert.equal(result, true);
  assert.deepEqual(calls, [
    {
      channelId: "old-community-channel",
      options: {
        force: true,
        messageId: "message",
        searchHighlight: undefined,
        threadRootId: "thread-root",
      },
    },
  ]);
  assert.equal(getCachedSearchHitEvent("message")?.id, "message");
});

test("search-hit navigation carries trimmed highlight state and forces repeated activations", async () => {
  clearSearchHitEventCache();
  const calls = [];

  await openSearchHitWithNavigation(plainMessage, {
    goChannel: async (channelId, options) => {
      calls.push({ channelId, options });
      return true;
    },
    goForumPost: async () => false,
    query: "  Mentions  ",
  });

  assert.equal(calls[0].options.force, true);
  assert.equal(calls[0].options.searchHighlight.messageId, "message");
  assert.equal(calls[0].options.searchHighlight.query, "Mentions");
  assert.match(calls[0].options.searchHighlight.activationId, /.+/);
});

test("forum-post search navigation carries transient same-route activation state", async () => {
  clearSearchHitEventCache();
  const forumPost = { ...forumComment, eventId: "post", kind: 45001 };
  const calls = [];

  await openSearchHitWithNavigation(forumPost, {
    goChannel: async () => false,
    goForumPost: async (channelId, postId, options) => {
      calls.push({ channelId, postId, options });
      return true;
    },
    query: "mentions",
  });

  assert.equal(calls[0].options.force, true);
  assert.equal(calls[0].options.searchHighlight.messageId, "post");
  assert.equal(calls[0].options.searchHighlight.query, "mentions");
  assert.match(calls[0].options.searchHighlight.activationId, /.+/);
});

test("cancelled search-hit navigation cannot repopulate cache or route", async () => {
  clearSearchHitEventCache();
  let resolveLookup;
  const destination = new Promise((resolve) => {
    resolveLookup = resolve;
  });
  const calls = [];
  const controller = new AbortController();
  const navigation = openSearchHitWithNavigation(
    forumComment,
    {
      goChannel: async () => calls.push("channel"),
      goForumPost: async () => calls.push("forum"),
      signal: controller.signal,
    },
    () => destination,
  );

  controller.abort();
  clearSearchHitEventCache();
  resolveLookup({
    kind: "forum-post",
    channelId: "old-community-channel",
    postId: "old-community-post",
    replyId: "comment",
  });
  await navigation;

  assert.deepEqual(calls, []);
  assert.equal(getCachedSearchHitEvent("comment"), null);
});

test("queue cancellation fences an in-flight forum-comment activation", async () => {
  clearSearchHitEventCache();
  let resolveLookup;
  const destination = new Promise((resolve) => {
    resolveLookup = resolve;
  });
  const calls = [];
  const queue = createDesktopNotificationActivationQueue((target, signal) =>
    activateDesktopNotificationTarget(
      target,
      {
        goChannel: async () => calls.push("channel"),
        goHome: async () => calls.push("home"),
        openSearchHit: (hit, behavior) =>
          openSearchHitWithNavigation(
            hit,
            {
              force: behavior?.force,
              goChannel: async () => calls.push("channel"),
              goForumPost: async () => calls.push("forum"),
              signal: behavior?.signal,
            },
            () => destination,
          ),
        revealWindow: async () => {},
      },
      signal,
    ),
  );

  queue.enqueue({
    channelId: "old-community-channel",
    eventId: "comment",
    kind: 45003,
  });
  await new Promise((resolve) => setImmediate(resolve));
  queue.cancel();
  clearSearchHitEventCache();
  resolveLookup({
    kind: "forum-post",
    channelId: "old-community-channel",
    postId: "old-community-post",
    replyId: "comment",
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(calls, []);
  assert.equal(getCachedSearchHitEvent("comment"), null);
});
