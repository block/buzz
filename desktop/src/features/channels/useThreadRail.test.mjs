import assert from "node:assert/strict";
import test from "node:test";

import {
  installDOMShim,
  installFreshStorage,
} from "./observedUnreadTestHarness.mjs";

installDOMShim();
installFreshStorage();

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import {
  readThreadRailStore,
  threadRailStorageKey,
  writeThreadRailStore,
} from "./threadRailStorage.ts";
import { useThreadRail } from "./useThreadRail.ts";

const RELAY_A = "wss://relay-a.example.com";
const RELAY_B = "wss://relay-b.example.com";
const SCOPE_A = { pubkey: "pubkey-a", relayUrl: RELAY_A };
const SCOPE_B = { pubkey: "pubkey-b", relayUrl: RELAY_A };
const SCOPE_C = { pubkey: "pubkey-a", relayUrl: RELAY_B };
const PIN_A = { channelId: "channel-a", rootId: "root-a", pinnedAt: 100 };
const PIN_B = { channelId: "channel-b", rootId: "root-b", pinnedAt: 200 };

async function mountHook(props) {
  const apiRef = { current: null };

  function Harness({ pubkey, relayUrl }) {
    apiRef.current = useThreadRail(pubkey, relayUrl);
    return null;
  }

  const root = createRoot(document.createElement("div"));
  const render = async (nextProps) => {
    await act(async () => {
      root.render(React.createElement(Harness, nextProps));
    });
  };
  await render(props);

  return {
    get api() {
      return apiRef.current;
    },
    render,
    unmount: async () => {
      await act(async () => root.unmount());
    },
  };
}

test("no scope exposes empty uncollapsed defaults", async () => {
  installFreshStorage();
  const harness = await mountHook({ pubkey: null, relayUrl: RELAY_A });

  assert.deepEqual(harness.api.pins, []);
  assert.equal(harness.api.collapsed, false);
  assert.equal(harness.api.isScoped, false);

  await harness.unmount();
});

test("hydrates pins and collapse preference from the active scope", async () => {
  installFreshStorage();
  writeThreadRailStore(SCOPE_A, { version: 1, pins: [PIN_A], collapsed: true });

  const harness = await mountHook({
    pubkey: SCOPE_A.pubkey,
    relayUrl: SCOPE_A.relayUrl,
  });

  assert.deepEqual(harness.api.pins, [PIN_A]);
  assert.equal(harness.api.collapsed, true);
  assert.equal(harness.api.isScoped, true);

  await harness.unmount();
});

test("updates the existing canonical pin return anchor without changing its order", async () => {
  installFreshStorage();
  const harness = await mountHook({
    pubkey: SCOPE_A.pubkey,
    relayUrl: SCOPE_A.relayUrl,
  });

  await act(async () => harness.api.pin(PIN_A));
  await act(async () => harness.api.pin(PIN_B));
  await act(async () => harness.api.updateAnchor(PIN_A, "nested-reply-a"));

  assert.deepEqual(harness.api.pins, [
    { ...PIN_A, returnAnchorId: "nested-reply-a" },
    PIN_B,
  ]);
  assert.deepEqual(readThreadRailStore(SCOPE_A).pins, harness.api.pins);

  await harness.unmount();
});

test("pin, unpin, and toggle persist while retaining in-memory state after a write failure", async () => {
  const storage = installFreshStorage();
  const harness = await mountHook({
    pubkey: SCOPE_A.pubkey,
    relayUrl: SCOPE_A.relayUrl,
  });

  await act(async () => harness.api.pin(PIN_A));
  assert.deepEqual(readThreadRailStore(SCOPE_A).pins, [PIN_A]);

  await act(async () => harness.api.toggleCollapsed());
  assert.equal(readThreadRailStore(SCOPE_A).collapsed, true);

  await act(async () => harness.api.unpin(PIN_A));
  assert.deepEqual(readThreadRailStore(SCOPE_A).pins, []);

  const originalSetItem = storage.setItem;
  const originalConsoleWarn = console.warn;
  storage.setItem = () => {
    throw new Error("storage unavailable");
  };
  console.warn = () => {};
  try {
    await act(async () => harness.api.pin(PIN_B));
    assert.deepEqual(harness.api.pins, [PIN_B]);
  } finally {
    storage.setItem = originalSetItem;
    console.warn = originalConsoleWarn;
  }

  await harness.unmount();
});

test("switching pubkey or relay reloads only the new scope without exposing prior pins", async () => {
  installFreshStorage();
  writeThreadRailStore(SCOPE_A, { version: 1, pins: [PIN_A], collapsed: true });
  writeThreadRailStore(SCOPE_B, {
    version: 1,
    pins: [PIN_B],
    collapsed: false,
  });
  writeThreadRailStore(SCOPE_C, { version: 1, pins: [], collapsed: false });

  const harness = await mountHook({
    pubkey: SCOPE_A.pubkey,
    relayUrl: SCOPE_A.relayUrl,
  });
  assert.deepEqual(harness.api.pins, [PIN_A]);

  await harness.render({ pubkey: SCOPE_B.pubkey, relayUrl: SCOPE_B.relayUrl });
  assert.deepEqual(harness.api.pins, [PIN_B]);
  assert.equal(harness.api.collapsed, false);

  await harness.render({ pubkey: SCOPE_C.pubkey, relayUrl: SCOPE_C.relayUrl });
  assert.deepEqual(harness.api.pins, []);
  assert.equal(harness.api.collapsed, false);
  assert.deepEqual(readThreadRailStore(SCOPE_A).pins, [PIN_A]);
  assert.deepEqual(readThreadRailStore(SCOPE_B).pins, [PIN_B]);
  assert.equal(
    globalThis.localStorage.getItem(threadRailStorageKey(SCOPE_C)) !== null,
    true,
  );

  await harness.unmount();
});
