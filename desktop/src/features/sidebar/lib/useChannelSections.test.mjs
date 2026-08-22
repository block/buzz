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

// Fix 2 regression: a live remote arriving while a local edit is pending must
// NOT overwrite the optimistic edit or strand its durable outbox. The pending
// edit's own debounced publish owns convergence (publish-or-adopt). Reverting
// applyRemote's hasPendingEdit guard makes the live event clobber the UI and
// leave the outbox replay-eligible.
test("live remote while a local edit is pending defers to the pending edit", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  const { useChannelSections } = await import("./useChannelSections.ts");

  const origFetch = relayClient.fetchEvents;
  const origLive = relayClient.subscribeLive;
  const origReconnect = relayClient.subscribeToReconnects;
  const origPublish = relayClient.publishEvent;
  const origTauri = window.__TAURI_INTERNALS__;

  let live = null;
  relayClient.fetchEvents = async () => [];
  relayClient.subscribeLive = async (_f, cb) => {
    live = cb;
    return async () => {};
  };
  relayClient.subscribeToReconnects = () => () => {};
  relayClient.publishEvent = async () => {};
  window.__TAURI_INTERNALS__ = {
    invoke: (cmd) => {
      if (cmd === "nip44_decrypt_from_self")
        return Promise.resolve(
          JSON.stringify({
            version: 1,
            sections: [{ id: "remote", name: "Remote", order: 0 }],
            assignments: {},
          }),
        );
      if (cmd === "nip44_encrypt_to_self") return Promise.resolve("ct");
      if (cmd === "sign_event")
        return Promise.resolve(
          JSON.stringify({
            id: "signed",
            pubkey: "pk-live-pending",
            content: "ct",
            created_at: 0,
            kind: 30078,
            tags: [],
            sig: "s",
          }),
        );
      return Promise.reject(new Error(`unmocked ${cmd}`));
    },
  };

  const pubkey = "pk-live-pending";
  const relayUrl = "wss://r.live";
  const outboxKey = `buzz-channel-sections-outbox.v1:${pubkey}:${encodeURIComponent(relayUrl)}`;

  let hook = null;
  try {
    await act(async () => {
      hook = renderHook(() => useChannelSections(pubkey, relayUrl));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    assert.ok(live, "live subscription installed");

    // Make a local edit — it becomes the pending store and persists to outbox.
    await act(async () => {
      hook.result.current.createSection("Local");
    });
    assert.ok(
      window.localStorage.getItem(outboxKey),
      "local edit persisted to outbox",
    );
    const localSectionIds = hook.result.current.sections.map((s) => s.id);

    // A remote live event arrives while the edit is still pending.
    await act(async () => {
      live({
        id: "remote-event",
        pubkey,
        created_at: 500,
        content: "cipher",
        kind: 30078,
        tags: [["d", "channel-sections"]],
        sig: "s",
      });
      await Promise.resolve();
      await Promise.resolve();
    });

    assert.deepEqual(
      hook.result.current.sections.map((s) => s.id),
      localSectionIds,
      "pending local edit must NOT be overwritten by the live remote",
    );
    assert.ok(
      window.localStorage.getItem(outboxKey),
      "outbox for the pending edit must survive the live remote",
    );
    hook.unmount();
  } finally {
    cleanup();
    relayClient.fetchEvents = origFetch;
    relayClient.subscribeLive = origLive;
    relayClient.subscribeToReconnects = origReconnect;
    relayClient.publishEvent = origPublish;
    window.__TAURI_INTERNALS__ = origTauri;
  }
});

// Fix 3 regression: equal-timestamp tie-break must match the relay's canonical
// winner (`created_at DESC, id ASC` → LOWEST id wins). Deliver the larger id
// first, then the lower id at the same timestamp; the lower-id store must win.
// Reverting applyRemote's `>=` back to `<=` converges on the larger id instead.
test("equal-timestamp tie-break applies the lower event id (relay canonical winner)", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  const { useChannelSections } = await import("./useChannelSections.ts");

  const origFetch = relayClient.fetchEvents;
  const origLive = relayClient.subscribeLive;
  const origReconnect = relayClient.subscribeToReconnects;
  const origTauri = window.__TAURI_INTERNALS__;

  let live = null;
  relayClient.fetchEvents = async () => [];
  relayClient.subscribeLive = async (_f, cb) => {
    live = cb;
    return async () => {};
  };
  relayClient.subscribeToReconnects = () => () => {};
  // Decrypt payload keyed off the event id embedded in the ciphertext so each
  // delivered event yields a distinct store we can assert on.
  window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      if (cmd === "nip44_decrypt_from_self") {
        const id = args?.ciphertext ?? "";
        return Promise.resolve(
          JSON.stringify({
            version: 1,
            sections: [{ id, name: id, order: 0 }],
            assignments: {},
          }),
        );
      }
      return Promise.reject(new Error(`unmocked ${cmd}`));
    },
  };

  const pubkey = "pk-tie";
  const relayUrl = "wss://r.tie";
  let hook = null;
  try {
    await act(async () => {
      hook = renderHook(() => useChannelSections(pubkey, relayUrl));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    assert.ok(live, "live subscription installed");

    const deliver = async (id) => {
      await act(async () => {
        live({
          id,
          pubkey,
          created_at: 1000,
          content: id, // decrypt echoes this into the section id
          kind: 30078,
          tags: [["d", "channel-sections"]],
          sig: "s",
        });
        await Promise.resolve();
        await Promise.resolve();
      });
    };

    // Larger id first (would win under the old <= comparator)...
    await deliver("bbbb");
    // ...then the lower id at the same timestamp — the relay's canonical winner.
    await deliver("aaaa");

    assert.deepEqual(
      hook.result.current.sections.map((s) => s.id),
      ["aaaa"],
      "lower event id must win the equal-timestamp tie-break",
    );
    hook.unmount();
  } finally {
    cleanup();
    relayClient.fetchEvents = origFetch;
    relayClient.subscribeLive = origLive;
    relayClient.subscribeToReconnects = origReconnect;
    window.__TAURI_INTERNALS__ = origTauri;
  }
});
