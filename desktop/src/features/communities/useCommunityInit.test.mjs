import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
let renderHook, waitFor, cleanup, act, useCommunityInit, relayClient;
const calls = [];
const pendingTrust = [];
let holdTrust = false;
const a = { id: "a", relayUrl: "wss://a.example", name: "A" };
const b = { id: "b", relayUrl: "wss://b.example", name: "B" };

before(async () => {
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "set_agent_avatar_communities" && holdTrust) {
        await new Promise((resolve, reject) => {
          pendingTrust.push({ resolve, reject });
        });
      }
      if (command === "get_identity") return { pubkey: "a".repeat(64) };
      if (command === "get_relay_url") return b.relayUrl;
      return undefined;
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
  ({ renderHook, waitFor, cleanup, act } = await import(
    "@testing-library/react"
  ));
  ({ useCommunityInit } = await import("./useCommunityInit.ts"));
  ({ relayClient } = await import("@/shared/api/relayClient"));
});
afterEach(async () => {
  cleanup();
  holdTrust = false;
  await act(async () => {
    for (const deferred of pendingTrust) deferred.resolve();
  });
  pendingTrust.length = 0;
  calls.length = 0;
  localStorage.clear();
});
after(() => dom.window.close());

function mount(communities) {
  return renderHook((list) => useCommunityInit(b, "b", false, false, list), {
    initialProps: communities,
  });
}

test("inactive relay removal/edit/add refreshes trust without reapplying or disconnecting active community", async () => {
  let disconnects = 0;
  const original = relayClient.disconnect;
  relayClient.disconnect = () => {
    disconnects += 1;
  };
  try {
    const { result, rerender } = mount([a, b]);
    await waitFor(() => assert.equal(result.current.isReady, true));
    assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 1);
    for (const list of [
      [b],
      [{ ...a, relayUrl: "wss://new-a.example" }, b],
      [a, b],
    ]) {
      const before = calls.filter(
        ([cmd]) => cmd === "set_agent_avatar_communities",
      ).length;
      rerender(list);
      await waitFor(() =>
        assert.equal(
          calls.filter(([cmd]) => cmd === "set_agent_avatar_communities")
            .length,
          before + 1,
        ),
      );
      assert.equal(result.current.isReady, true);
      assert.equal(
        calls.filter(([cmd]) => cmd === "apply_workspace").length,
        1,
      );
      assert.equal(disconnects, 0);
    }
    const count = calls.length;
    rerender([{ ...b, name: "New label" }, a]);
    assert.equal(calls.length, count);
    const updates = calls.filter(
      ([cmd]) => cmd === "set_agent_avatar_communities",
    );
    assert.deepEqual(updates[1][1].relayUrls, ["https://b.example"]);
  } finally {
    relayClient.disconnect = original;
  }
});

test("initial workspace restore waits for avatar trust IPC", async () => {
  holdTrust = true;
  const { result } = mount([a, b]);
  await waitFor(() => assert.equal(pendingTrust.length, 1));
  // Let identity resolution and any unguarded apply settle while trust is held.
  await act(async () => {});
  assert.equal(
    calls.some(([cmd]) => cmd === "apply_workspace"),
    false,
  );
  await act(async () => pendingTrust[0].resolve());
  await waitFor(() => assert.equal(result.current.isReady, true));
  assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 1);
});

test("source removal during pending trust serializes IPC and blocks restore until the latest update", async (t) => {
  holdTrust = true;
  const disconnect = t.mock.method(relayClient, "disconnect", () => {});
  const { result, rerender } = mount([a, b]);
  await waitFor(() => assert.equal(pendingTrust.length, 1));
  await act(async () => {});

  rerender([b]);
  await act(async () => {});
  assert.equal(
    pendingTrust.length,
    1,
    "P2 must not dispatch before P1 settles",
  );
  assert.equal(result.current.isReady, false);

  await act(async () => pendingTrust[0].resolve());
  await waitFor(() => assert.equal(pendingTrust.length, 2));
  assert.deepEqual(
    calls
      .filter(([cmd]) => cmd === "set_agent_avatar_communities")
      .map(([, args]) => args.relayUrls),
    [["https://a.example", "https://b.example"], ["https://b.example"]],
  );
  assert.equal(
    calls.filter(([cmd]) => cmd === "apply_workspace").length,
    0,
    "P1 alone must not release restoration while source removal is pending",
  );
  assert.equal(result.current.isReady, false);

  await act(async () => pendingTrust[1].resolve());
  await waitFor(() => assert.equal(result.current.isReady, true));
  assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 1);
  assert.equal(disconnect.mock.callCount(), 0);
});

for (const failingUpdate of [0, 1]) {
  test(`trust update ${failingUpdate + 1} rejection does not release queued restoration`, async (t) => {
    holdTrust = true;
    t.mock.method(console, "error", () => {});
    const { result, rerender } = mount([a, b]);
    await waitFor(() => assert.equal(pendingTrust.length, 1));
    rerender([b]);
    await act(async () => {});
    if (failingUpdate === 1) {
      await act(async () => pendingTrust[0].resolve());
      await waitFor(() => assert.equal(pendingTrust.length, 2));
    }
    await act(async () =>
      pendingTrust[failingUpdate].reject(new Error("IPC failed")),
    );
    assert.equal(result.current.isReady, false);
    assert.ok(result.current.error);
    assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 0);
    assert.equal(pendingTrust.length, failingUpdate + 1);
  });
}
