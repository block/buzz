import assert from "node:assert/strict";
import test from "node:test";

import { getThreadRailAnchorUpdate } from "./threadRailAnchor.ts";

const PIN = { channelId: "channel-a", rootId: "root-a", pinnedAt: 100 };

test("returns an anchor update only for a pinned nested reply in the open thread", () => {
  assert.deepEqual(
    getThreadRailAnchorUpdate([PIN], "channel-a", "root-a", {
      id: "nested-a",
      rootId: "root-a",
    }),
    { pin: PIN, returnAnchorId: "nested-a" },
  );
});

test("does not anchor roots, other threads, channels, or unpinned threads", () => {
  assert.equal(
    getThreadRailAnchorUpdate([PIN], "channel-a", "root-a", {
      id: "root-a",
      rootId: "root-a",
    }),
    null,
  );
  assert.equal(
    getThreadRailAnchorUpdate([PIN], "channel-a", "root-a", {
      id: "other-thread-reply",
      rootId: "root-b",
    }),
    null,
  );
  assert.equal(
    getThreadRailAnchorUpdate([PIN], "channel-b", "root-a", {
      id: "nested-a",
      rootId: "root-a",
    }),
    null,
  );
  assert.equal(
    getThreadRailAnchorUpdate([], "channel-a", "root-a", {
      id: "nested-a",
      rootId: "root-a",
    }),
    null,
  );
});
