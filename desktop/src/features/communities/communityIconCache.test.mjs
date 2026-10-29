import assert from "node:assert/strict";
import test from "node:test";

import {
  boundCommunityIconCache,
  loadCachedCommunityIcon,
  MAX_CACHED_COMMUNITY_ICON_LENGTH,
  MAX_CACHED_COMMUNITY_ICONS,
  saveCachedCommunityIcon,
} from "./communityIconCache.ts";

test("community icon cache caps entries and rejects oversized icons", () => {
  const cache = Object.fromEntries(
    Array.from({ length: MAX_CACHED_COMMUNITY_ICONS + 1 }, (_, index) => [
      `relay-${index}`,
      `icon-${index}`,
    ]),
  );
  cache.oversized = "x".repeat(MAX_CACHED_COMMUNITY_ICON_LENGTH + 1);

  const bounded = boundCommunityIconCache(cache);

  assert.equal(Object.keys(bounded).length, MAX_CACHED_COMMUNITY_ICONS);
  assert.equal(bounded["relay-0"], undefined);
  assert.equal(bounded.oversized, undefined);
  assert.equal(bounded[`relay-${MAX_CACHED_COMMUNITY_ICONS}`], "icon-32");
});

test("community icon cache accepts relay-sized icons above 64 KiB", () => {
  const values = new Map([
    ["buzz-community-icons", JSON.stringify({ relay: "prior-icon" })],
  ]);
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };
  const acceptedIcon = "x".repeat(80 * 1024);

  saveCachedCommunityIcon("relay", acceptedIcon);

  assert.equal(loadCachedCommunityIcon("relay"), acceptedIcon);
});
