import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_COMMUNITY_SECTIONS_ENABLED,
  communitySectionsEnabledStorageKey,
  readCommunitySectionsEnabled,
  writeCommunitySectionsEnabled,
} from "./featurePreference.ts";

const store = new Map();

test.beforeEach(() => {
  store.clear();
  globalThis.localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
    removeItem: (key) => {
      store.delete(key);
    },
  };
  globalThis.window = { localStorage: globalThis.localStorage };
});

test("read defaults to ON when unset", () => {
  assert.equal(
    readCommunitySectionsEnabled("pk", "wss://relay.example"),
    DEFAULT_COMMUNITY_SECTIONS_ENABLED,
  );
  assert.equal(DEFAULT_COMMUNITY_SECTIONS_ENABLED, true);
});

test("write/read round-trip", () => {
  const pubkey = "aabb";
  const relay = "wss://buzz.huladesk.com";
  writeCommunitySectionsEnabled(pubkey, relay, false);
  assert.equal(readCommunitySectionsEnabled(pubkey, relay), false);
  writeCommunitySectionsEnabled(pubkey, relay, true);
  assert.equal(readCommunitySectionsEnabled(pubkey, relay), true);
  assert.match(
    communitySectionsEnabledStorageKey(pubkey, relay),
    /buzz-community-sections-enabled\.v1:aabb:/,
  );
});
