import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

after(() => dom.window.close());

test("same-second mute and unmute mutations survive at capacity", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  const { MAX_CHANNEL_MUTE_ENTRIES, readChannelMutesStore, storageKey } =
    await import("./channelMutesStorage.ts");
  const { useChannelMutes } = await import("./useChannelMutes.ts");

  const originalFetchEvents = relayClient.fetchEvents;
  const originalSubscribeLive = relayClient.subscribeLive;
  const originalSubscribeToReconnects = relayClient.subscribeToReconnects;
  const originalDateNow = Date.now;
  const updatedAt = 1_234_567;
  Date.now = () => updatedAt * 1_000;
  relayClient.fetchEvents = async () => [];
  relayClient.subscribeLive = async () => async () => {};
  relayClient.subscribeToReconnects = () => () => {};

  const relayUrl = "wss://relay.example";
  const channels = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_MUTE_ENTRIES }, (_, index) => [
      `z-channel-${String(index).padStart(3, "0")}`,
      { muted: true, updatedAt },
    ]),
  );

  try {
    for (const [pubkey, action, expectedMuted] of [
      ["pk-mute", "muteChannel", true],
      ["pk-unmute", "unmuteChannel", false],
    ]) {
      window.localStorage.setItem(
        storageKey(pubkey),
        JSON.stringify({ version: 1, channels }),
      );
      const { result, unmount } = renderHook(() =>
        useChannelMutes(pubkey, relayUrl),
      );

      act(() => result.current[action]("a-target"));

      const persisted = readChannelMutesStore(pubkey);
      assert.equal(
        Object.keys(persisted.channels).length,
        MAX_CHANNEL_MUTE_ENTRIES,
      );
      assert.deepEqual(persisted.channels["a-target"], {
        muted: expectedMuted,
        updatedAt,
      });
      unmount();
    }
  } finally {
    cleanup();
    Date.now = originalDateNow;
    relayClient.fetchEvents = originalFetchEvents;
    relayClient.subscribeLive = originalSubscribeLive;
    relayClient.subscribeToReconnects = originalSubscribeToReconnects;
  }
});
