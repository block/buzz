import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

import { useHomeFeedNotificationState } from "./hooks.ts";

const SETTINGS = {
  desktopEnabled: false,
  homeBadgeEnabled: true,
  notifyWhileViewing: false,
  slotAlertsEnabled: {},
  slotAlertsSnapshot: null,
  sounds: {},
};

const emptyFeed = () => ({
  feed: {
    activity: [],
    agentActivity: [],
    mentions: [],
    needsAction: [],
  },
  meta: { generatedAt: 0, since: 0, total: 0 },
});

const feedWithMention = (id) => ({
  ...emptyFeed(),
  feed: {
    ...emptyFeed().feed,
    mentions: [
      {
        category: "mention",
        channelId: "general",
        channelName: "general",
        content: "@self please review",
        createdAt: 1,
        id,
        kind: 9,
        pubkey: "author",
        tags: [["p", "self"]],
      },
    ],
  },
});

function renderAttentionHook(pubkey, initialFeed, initialObserved) {
  return renderHook(
    ({
      channelReadAt = null,
      feed,
      messageReadAt = null,
      observed,
      readStateVersion = 0,
    }) =>
      useHomeFeedNotificationState(
        feed,
        pubkey,
        SETTINGS,
        async () => true,
        false,
        true,
        observed,
        () => channelReadAt,
        readStateVersion,
        new Set(),
        undefined,
        new Set(),
        new Set(),
        [],
        () => null,
        () => messageReadAt,
        [],
        new Set(),
      ),
    { initialProps: { feed: initialFeed, observed: initialObserved } },
  );
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

test("viewing the source message clears and persists mention attention", async () => {
  const pubkey = "source-view-user";
  const storageKey = `buzz-home-mention-attention-seen.v1:${pubkey}`;
  const hook = renderAttentionHook(pubkey, emptyFeed(), true);
  await waitFor(() => assert.equal(localStorage.getItem(storageKey), "[]"));

  await act(async () => {
    hook.rerender({
      feed: feedWithMention("source-view-mention"),
      observed: false,
    });
  });
  assert.equal(hook.result.current.hasHomeMentionAttention, true);

  await act(async () => {
    hook.rerender({
      channelReadAt: 1,
      feed: feedWithMention("source-view-mention"),
      observed: false,
      readStateVersion: 1,
    });
  });
  assert.equal(hook.result.current.hasHomeMentionAttention, false);
  await waitFor(() =>
    assert.equal(
      localStorage.getItem(storageKey),
      JSON.stringify(["source-view-mention"]),
    ),
  );

  hook.unmount();
  const remount = renderAttentionHook(
    pubkey,
    feedWithMention("source-view-mention"),
    false,
  );
  assert.equal(remount.result.current.hasHomeMentionAttention, false);
});

test("a background Inbox does not acknowledge a newly arrived mention", async () => {
  const pubkey = "background-user";
  const storageKey = `buzz-home-mention-attention-seen.v1:${pubkey}`;
  const hook = renderAttentionHook(pubkey, emptyFeed(), true);
  await waitFor(() => assert.equal(localStorage.getItem(storageKey), "[]"));

  await act(async () => {
    hook.rerender({
      feed: feedWithMention("background-mention"),
      observed: false,
    });
  });
  assert.equal(hook.result.current.hasHomeMentionAttention, true);
  assert.equal(localStorage.getItem(storageKey), "[]");

  await act(async () => {
    hook.rerender({
      feed: feedWithMention("background-mention"),
      observed: true,
    });
  });
  await waitFor(() =>
    assert.equal(
      localStorage.getItem(storageKey),
      JSON.stringify(["background-mention"]),
    ),
  );
  assert.equal(hook.result.current.hasHomeMentionAttention, false);

  await act(async () => {
    hook.rerender({
      feed: feedWithMention("background-mention"),
      observed: false,
    });
  });
  assert.equal(hook.result.current.hasHomeMentionAttention, false);
});

test("a huddle companion cannot acknowledge main-Inbox attention", async () => {
  const pubkey = "companion-user";
  const storageKey = `buzz-home-mention-attention-seen.v1:${pubkey}`;
  const baseline = renderAttentionHook(pubkey, emptyFeed(), true);
  await waitFor(() => assert.equal(localStorage.getItem(storageKey), "[]"));
  baseline.unmount();

  const companion = renderAttentionHook(
    pubkey,
    feedWithMention("companion-mention"),
    false,
  );
  assert.equal(companion.result.current.hasHomeMentionAttention, true);
  assert.equal(localStorage.getItem(storageKey), "[]");
  companion.unmount();

  const mainWindow = renderAttentionHook(
    pubkey,
    feedWithMention("companion-mention"),
    false,
  );
  assert.equal(mainWindow.result.current.hasHomeMentionAttention, true);
});
