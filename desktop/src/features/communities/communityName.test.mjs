import assert from "node:assert/strict";
import test from "node:test";
import {
  reconcileCommunityName,
  withLocalCommunityName,
} from "./communityName.ts";
import { deriveCommunityName, initFirstCommunity } from "./communityStorage.ts";
const community = {
  id: "one",
  name: "buzz",
  relayUrl: "wss://buzz.example.com",
  addedAt: "2026-09-16",
};

test("canonical name replaces the display while retaining the original label", () => {
  const updated = reconcileCommunityName(community, { name: "Shared name" });
  assert.equal(updated.name, "Shared name");
  assert.equal(updated.fallbackName, "buzz");
  assert.equal(
    reconcileCommunityName(updated, { name: "Renamed" }).name,
    "Renamed",
  );
  assert.equal(
    reconcileCommunityName(updated, { name: "Shared name" }),
    updated,
  );
  assert.equal(reconcileCommunityName(updated, { name: null }).name, "buzz");
});
test("an explicit local nickname survives relay renames and can be cleared", () => {
  const named = reconcileCommunityName(community, { name: "Shared name" });
  const nicknamed = withLocalCommunityName(named, "  My nickname  ");
  const renamed = reconcileCommunityName(nicknamed, { name: "Renamed" });
  assert.equal(renamed.name, "My nickname");
  assert.equal(renamed.canonicalName, "Renamed");
  assert.equal(withLocalCommunityName(renamed, "").name, "Renamed");
});
test("private IP fallback identifies the whole address", () => {
  assert.equal(
    deriveCommunityName("ws://100.64.1.2:3000"),
    "Community (100.64.1.2)",
  );
});

test("legacy custom labels become explicit local nicknames", () => {
  const updated = reconcileCommunityName(
    { ...community, name: "My old label" },
    { name: "Shared" },
  );
  assert.equal(updated.name, "My old label");
  assert.equal(updated.localName, "My old label");
  assert.equal(updated.canonicalName, "Shared");
  assert.equal(withLocalCommunityName(updated, "").name, "Shared");
  const ip = reconcileCommunityName(
    { ...community, relayUrl: "ws://100.64.1.2:3000", name: "100" },
    { name: "Shared" },
  );
  assert.equal(ip.name, "Shared");
});

test("new invited connections follow canonical names rather than infer aliases", (t) => {
  const previousWindow = globalThis.window;
  const previousStorage = globalThis.localStorage;
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  globalThis.window = { localStorage: storage };
  globalThis.localStorage = storage;
  t.after(() => {
    globalThis.window = previousWindow;
    globalThis.localStorage = previousStorage;
  });
  const first = initFirstCommunity(
    community.relayUrl,
    "identity",
    "Invite label",
  );
  assert.ok(first);
  assert.equal(
    reconcileCommunityName(first, { name: "Shared" }).name,
    "Shared",
  );
});
