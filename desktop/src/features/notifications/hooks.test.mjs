import assert from "node:assert/strict";
import test from "node:test";

import { shouldCountTowardHomeBadgeSubtotal } from "./lib/homeBadge.ts";

test("home badge subtotal excludes high-priority channel items regardless of thread status", () => {
  const highPriorityChannelIds = new Set(["dm-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "dm-channel" },
      highPriorityChannelIds,
    ),
    false,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "main-channel" },
      highPriorityChannelIds,
    ),
    true,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: null },
      highPriorityChannelIds,
    ),
    true,
  );
});
