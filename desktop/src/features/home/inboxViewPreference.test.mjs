import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

import {
  DEFAULT_INBOX_VIEW_PREFERENCE,
  inboxViewPreferenceStorageKey,
  parseInboxViewPreference,
} from "./inboxViewPreference.ts";

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

test("parses versioned Inbox view preferences with safe field defaults", () => {
  assert.deepEqual(
    parseInboxViewPreference({
      version: 1,
      filter: "mention",
      unreadOnly: true,
    }),
    { version: 1, filter: "mention", unreadOnly: true },
  );
  assert.deepEqual(
    parseInboxViewPreference({
      version: 1,
      filter: "unknown",
      unreadOnly: "yes",
    }),
    DEFAULT_INBOX_VIEW_PREFERENCE,
  );
  assert.equal(parseInboxViewPreference({ version: 2 }), null);
});

test("filter and unread-only restore from the per-user, per-relay cache", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  const { useInboxViewPreference } = await import(
    "./useInboxViewPreference.ts"
  );

  const originalFetchEvents = relayClient.fetchEvents;
  const originalSubscribeLive = relayClient.subscribeLive;
  const originalSubscribeToReconnects = relayClient.subscribeToReconnects;
  let relayOperations = 0;
  relayClient.fetchEvents = async () => {
    relayOperations += 1;
    return [];
  };
  relayClient.subscribeLive = async () => {
    relayOperations += 1;
    return async () => {};
  };
  relayClient.subscribeToReconnects = () => {
    relayOperations += 1;
    return () => {};
  };

  const pubkey = "pk-inbox-view";
  const relayUrl = "wss://relay.example";
  const key = inboxViewPreferenceStorageKey(pubkey, relayUrl);
  window.localStorage.clear();

  try {
    const first = renderHook(() => useInboxViewPreference(pubkey, relayUrl));
    act(() => first.result.current.setFilter("mention"));
    act(() => first.result.current.setUnreadOnly(true));

    assert.deepEqual(JSON.parse(window.localStorage.getItem(key) ?? "null"), {
      version: 1,
      filter: "mention",
      unreadOnly: true,
    });
    first.unmount();

    const restored = renderHook(() => useInboxViewPreference(pubkey, relayUrl));
    assert.equal(restored.result.current.filter, "mention");
    assert.equal(restored.result.current.unreadOnly, true);
    restored.unmount();
    assert.equal(relayOperations, 0);
  } finally {
    cleanup();
    relayClient.fetchEvents = originalFetchEvents;
    relayClient.subscribeLive = originalSubscribeLive;
    relayClient.subscribeToReconnects = originalSubscribeToReconnects;
  }
});

test("Inbox view cache does not bleed across relays", () => {
  assert.notEqual(
    inboxViewPreferenceStorageKey("pk", "wss://one.example"),
    inboxViewPreferenceStorageKey("pk", "wss://two.example"),
  );
});
