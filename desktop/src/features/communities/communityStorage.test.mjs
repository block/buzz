import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalCommunityRelayUrl,
  clearCommunityStorage,
  findCommunityByRelayUrl,
  initFirstCommunity,
  migrateLegacyCommunityStorage,
  shouldAutoConnectDefaultRelay,
} from "./communityStorage.ts";

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  };
}

test("migrateLegacyCommunityStorage promotes current Buzz workspace state", () => {
  const storage = createMemoryStorage({
    "buzz-workspaces": '[{"id":"current"}]',
    "buzz-active-workspace-id": "current",
  });

  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.getItem("buzz-communities"), '[{"id":"current"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), "current");
});

test("migrateLegacyCommunityStorage does not overwrite new community state", () => {
  const storage = createMemoryStorage({
    "buzz-communities": '[{"id":"new"}]',
    "buzz-active-community-id": "new",
    "buzz-workspaces": '[{"id":"old"}]',
    "buzz-active-workspace-id": "old",
  });

  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.getItem("buzz-communities"), '[{"id":"new"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), "new");
});

test("signed-build relay defaults auto-connect during first-run onboarding", () => {
  assert.equal(
    shouldAutoConnectDefaultRelay("wss://buzz.block.builderlab.xyz"),
    true,
  );
  assert.equal(shouldAutoConnectDefaultRelay("ws://localhost:3000"), false);
  assert.equal(shouldAutoConnectDefaultRelay("ws://127.0.0.1:3000"), false);
  assert.equal(shouldAutoConnectDefaultRelay("ws://[::1]:3000"), false);
  assert.equal(shouldAutoConnectDefaultRelay("ws://0.0.0.0:3000"), false);
  assert.equal(shouldAutoConnectDefaultRelay("http://localhost:3000"), false);
  assert.equal(
    shouldAutoConnectDefaultRelay("https://relay.example.com"),
    false,
  );
  assert.equal(shouldAutoConnectDefaultRelay("relay.example.com"), false);
  assert.equal(shouldAutoConnectDefaultRelay("not a valid relay"), false);
});

test("failed first-community write preserves existing community data", () => {
  const storage = createMemoryStorage({
    "buzz-communities": '[{"id":"existing"}]',
    "buzz-workspaces": '[{"id":"legacy"}]',
    "buzz-active-workspace-id": "legacy",
  });
  storage.setItem = (key, value) => {
    if (key === "buzz-communities") {
      throw new Error("QuotaExceededError");
    }
    storage.values.set(key, String(value));
  };
  globalThis.localStorage = storage;
  globalThis.window = { localStorage: storage };

  assert.equal(initFirstCommunity("wss://relay.example.com", "pubkey"), null);
  assert.equal(storage.getItem("buzz-communities"), '[{"id":"existing"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), null);
  assert.equal(storage.getItem("buzz-workspaces"), '[{"id":"legacy"}]');
  assert.equal(storage.getItem("buzz-active-workspace-id"), "legacy");
});

test("clearCommunityStorage removes new and legacy state", () => {
  const storage = createMemoryStorage({
    "buzz-communities": "new",
    "buzz-active-community-id": "new",
    "buzz-workspaces": "old",
    "buzz-active-workspace-id": "old",
  });

  clearCommunityStorage(storage);
  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.length, 0);
});

test("canonicalCommunityRelayUrl normalizes case and trailing slashes", () => {
  assert.equal(
    canonicalCommunityRelayUrl("WSS://Buzz.Block.Builderlab.xyz/"),
    "wss://buzz.block.builderlab.xyz",
  );
  assert.equal(
    canonicalCommunityRelayUrl("wss://buzz.block.builderlab.xyz"),
    "wss://buzz.block.builderlab.xyz",
  );
});

test("canonicalCommunityRelayUrl normalizes loopback host spellings", () => {
  assert.equal(
    canonicalCommunityRelayUrl("ws://localhost:3000"),
    "ws://127.0.0.1:3000",
  );
  assert.equal(
    canonicalCommunityRelayUrl("ws://127.0.0.1:3000"),
    "ws://127.0.0.1:3000",
  );
});

test("findCommunityByRelayUrl matches stored communities canonically", () => {
  const communities = [
    {
      id: "builderlab",
      name: "Builderlab",
      relayUrl: "wss://Buzz.Block.Builderlab.xyz/",
      addedAt: "2026-07-23T00:00:00.000Z",
    },
    {
      id: "other",
      name: "Other",
      relayUrl: "wss://other.example",
      addedAt: "2026-07-23T00:00:00.000Z",
    },
  ];

  assert.equal(
    findCommunityByRelayUrl(communities, "wss://buzz.block.builderlab.xyz")?.id,
    "builderlab",
  );
  assert.equal(
    findCommunityByRelayUrl(communities, "wss://missing.example"),
    undefined,
  );
});

test("findCommunityByRelayUrl matches localhost against 127.0.0.1", () => {
  const communities = [
    {
      id: "local",
      name: "Local Dev",
      relayUrl: "ws://127.0.0.1:3000",
      addedAt: "2026-07-23T00:00:00.000Z",
    },
  ];
  assert.equal(
    findCommunityByRelayUrl(communities, "ws://localhost:3000")?.id,
    "local",
  );
});
