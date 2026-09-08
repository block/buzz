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

test("assignChannel refreshes an existing assignment before the next eviction", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  const { MAX_CHANNEL_SECTION_ASSIGNMENTS, storageKey } = await import(
    "./channelSectionsStorage.ts"
  );
  const { useChannelSections } = await import("./useChannelSections.ts");

  const originalFetchEvents = relayClient.fetchEvents;
  const originalSubscribeLive = relayClient.subscribeLive;
  const originalSubscribeToReconnects = relayClient.subscribeToReconnects;
  relayClient.fetchEvents = async () => [];
  relayClient.subscribeLive = async () => async () => {};
  relayClient.subscribeToReconnects = () => () => {};

  const pubkey = "pk-at-capacity";
  const relayUrl = "wss://relay.example";
  const assignments = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_SECTION_ASSIGNMENTS }, (_, index) => [
      `chan-${String(index).padStart(4, "0")}`,
      "section-1",
    ]),
  );
  window.localStorage.setItem(
    storageKey(pubkey, relayUrl),
    JSON.stringify({
      version: 1,
      sections: [
        { id: "section-1", name: "One", order: 0 },
        { id: "section-2", name: "Two", order: 1 },
      ],
      assignments,
    }),
  );

  try {
    const { result, unmount } = renderHook(() =>
      useChannelSections(pubkey, relayUrl),
    );

    act(() => result.current.assignChannel("chan-0000", "section-2"));
    act(() => result.current.assignChannel("chan-new", "section-1"));

    assert.equal(result.current.assignments["chan-0000"], "section-2");
    assert.equal(result.current.assignments["chan-new"], "section-1");
    assert.equal(result.current.assignments["chan-0001"], undefined);
    assert.equal(
      Object.keys(result.current.assignments).length,
      MAX_CHANNEL_SECTION_ASSIGNMENTS,
    );
    unmount();
  } finally {
    cleanup();
    relayClient.fetchEvents = originalFetchEvents;
    relayClient.subscribeLive = originalSubscribeLive;
    relayClient.subscribeToReconnects = originalSubscribeToReconnects;
  }
});
