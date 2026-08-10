import assert from "node:assert/strict";
import test from "node:test";

import {
  boundCommunityIconCache,
  MAX_CACHED_COMMUNITY_ICON_LENGTH,
  MAX_CACHED_COMMUNITY_ICONS,
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
