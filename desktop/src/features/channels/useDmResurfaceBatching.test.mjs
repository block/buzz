/**
 * Batching lifecycle for useDmResurfaceFromMessages.
 *
 * The relay rejects a REQ whose aggregate explicit `#h` values exceed
 * MAX_EXPLICIT_CHANNEL_VALUES, so a hidden set larger than the cap must be
 * split into multiple subscriptions or every hidden DM loses its resurface
 * trigger. These tests exercise exactly that split and its teardown, which a
 * pure helper test cannot reach because the batching only happens inside the
 * mounted effect.
 */

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
    localStorage: dom.window.localStorage,
  });
  globalThis.__TAURI_INTERNALS__ = {
    invoke: (cmd) => {
      if (cmd === "get_channel_members") {
        return Promise.resolve({
          members: [
            { pubkey: VIEWER, role: "member", is_agent: false },
            { pubkey: "b".repeat(64), role: "member", is_agent: false },
          ],
        });
      }
      return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

const RELAY_URL = "wss://relay.example";
const VIEWER = "a".repeat(64);

function seedCommunity() {
  window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-a",
        name: "Community A",
        relayUrl: RELAY_URL,
        addedAt: "2026-01-01T00:00:00Z",
      },
    ]),
  );
  window.localStorage.setItem("buzz-active-community-id", "community-a");
}

function hiddenDmSnapshot(count) {
  return {
    id: "snapshot-1",
    kind: 30622,
    pubkey: VIEWER,
    content: "",
    created_at: 1,
    tags: Array.from({ length: count }, (_, index) => [
      "h",
      `dm-${String(index).padStart(4, "0")}`,
    ]),
    sig: "",
  };
}

async function mount(hiddenCount, subscribeImpl, reopen) {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const React = (await import("react")).default;
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  );
  const { relayClient } = await import("@/shared/api/relayClient");
  const { useDmResurfaceFromMessages } = await import(
    "./useDmResurfaceFromMessages.ts"
  );

  const originalFetchEvents = relayClient.fetchEvents;
  const originalSubscribeLive = relayClient.subscribeLive;

  relayClient.fetchEvents = async () =>
    hiddenCount > 0 ? [hiddenDmSnapshot(hiddenCount)] : [];
  relayClient.subscribeLive = subscribeImpl;

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });

  const wrapper = ({ children }) =>
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(CommunitiesProvider, null, children),
    );

  const hook = renderHook(
    () =>
      useDmResurfaceFromMessages({
        pubkey: VIEWER,
        relayUrl: RELAY_URL,
        reopen: reopen ?? (async () => ({ id: "x" })),
      }),
    { wrapper },
  );

  return {
    act,
    async settle() {
      for (let i = 0; i < 6; i++) {
        await act(async () => {
          await new Promise((r) => setTimeout(r, 5));
        });
      }
    },
    unmount: hook.unmount,
    restore() {
      hook.unmount();
      queryClient.clear();
      queryClient.unmount();
      cleanup();
      relayClient.fetchEvents = originalFetchEvents;
      relayClient.subscribeLive = originalSubscribeLive;
    },
  };
}

test("128 hidden DMs use a single subscription within the cap", async () => {
  seedCommunity();
  const batches = [];
  const harness = await mount(128, async (filter) => {
    batches.push(filter["#h"]);
    return async () => {};
  });
  try {
    await harness.settle();
    assert.equal(batches.length, 1);
    assert.equal(batches[0].length, 128);
  } finally {
    harness.restore();
  }
});

test("129 hidden DMs split into two subscriptions, both within the cap", async () => {
  seedCommunity();
  const batches = [];
  const harness = await mount(129, async (filter) => {
    batches.push(filter["#h"]);
    return async () => {};
  });
  try {
    await harness.settle();
    assert.equal(batches.length, 2);
    assert.deepEqual(
      batches.map((batch) => batch.length),
      [128, 1],
    );
    // Every hidden id appears exactly once across the batches.
    const all = batches.flat();
    assert.equal(new Set(all).size, 129);
    for (const batch of batches) {
      assert.ok(batch.length <= 128);
    }
  } finally {
    harness.restore();
  }
});

test("activity delivered on the final batch resurfaces its DM", async () => {
  seedCommunity();
  const handlers = [];
  const reopened = [];
  const targetIdRef = { id: null };
  const harness = await mount(
    129,
    async (filter, onEvent) => {
      handlers.push({ ids: filter["#h"], onEvent });
      return async () => {};
    },
    async ({ pubkeys }) => {
      reopened.push(pubkeys);
      // The reopen contract returns the resurfaced channel id; the action
      // rejects a mismatch, so echo the target the event carried.
      return { id: targetIdRef.id };
    },
  );
  try {
    await harness.settle();
    const finalBatch = handlers[1];
    const targetId = finalBatch.ids[0];
    targetIdRef.id = targetId;
    // Deliver a peer message on the FINAL batch's subscription. If batching
    // dropped that batch's handler, this event would never reach the
    // coordinator and reopen would never fire.
    await harness.act(async () => {
      finalBatch.onEvent({
        id: "evt-1",
        kind: 9,
        pubkey: "b".repeat(64),
        content: "hi",
        created_at: 2,
        tags: [["h", targetId]],
        sig: "",
      });
      await new Promise((r) => setTimeout(r, 5));
    });
    assert.equal(reopened.length, 1);
    assert.deepEqual(reopened[0], ["b".repeat(64)]);
  } finally {
    harness.restore();
  }
});

test("a batch subscription failure does not abort the other batches", async () => {
  seedCommunity();
  const batches = [];
  let call = 0;
  const harness = await mount(129, async (filter) => {
    call += 1;
    if (call === 1) throw new Error("relay rejected batch 1");
    batches.push(filter["#h"]);
    return async () => {};
  });
  try {
    await harness.settle();
    // The second batch still subscribed even though the first threw.
    assert.equal(batches.length, 1);
    assert.equal(batches[0].length, 1);
  } finally {
    harness.restore();
  }
});

test("teardown while batch setup is pending disposes every settled batch", async () => {
  seedCommunity();
  let disposeCount = 0;
  const releases = [];
  const harness = await mount(129, async () => {
    // Each subscribe parks until the test releases it, so the effect can be
    // torn down while batch setup is still in flight.
    await new Promise((resolve) => releases.push(resolve));
    return async () => {
      disposeCount += 1;
    };
  });
  try {
    await harness.settle();
    // Unmount before any subscribe resolves.
    await harness.act(async () => {
      harness.unmount();
    });
    // Now let both subscribes resolve; each must be disposed since its owning
    // generation is gone.
    await harness.act(async () => {
      for (const release of releases) release();
      await new Promise((r) => setTimeout(r, 5));
    });
    assert.equal(disposeCount, 2);
  } finally {
    harness.restore();
  }
});
