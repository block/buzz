import assert from "node:assert/strict";
import test from "node:test";

import { filterHiddenChannels } from "./hiddenChannelFilter.ts";

const CHANNELS = [{ id: "a" }, { id: "b" }, { id: "c" }];

function ids(channels) {
  return channels.map((channel) => channel.id);
}

test("returns the same array when nothing is hidden", () => {
  assert.equal(filterHiddenChannels(CHANNELS, {}), CHANNELS);
  assert.equal(
    filterHiddenChannels(CHANNELS, { hiddenChannelIds: new Set() }),
    CHANNELS,
  );
});

test("drops hidden channels", () => {
  assert.deepEqual(
    ids(filterHiddenChannels(CHANNELS, { hiddenChannelIds: new Set(["b"]) })),
    ["a", "c"],
  );
});

test("keeps the active channel visible", () => {
  assert.deepEqual(
    ids(
      filterHiddenChannels(CHANNELS, {
        hiddenChannelIds: new Set(["b", "c"]),
        activeChannelId: "b",
      }),
    ),
    ["a", "b"],
  );
});

test("keeps a hidden channel holding a mention-tier unread", () => {
  assert.deepEqual(
    ids(
      filterHiddenChannels(CHANNELS, {
        hiddenChannelIds: new Set(["b", "c"]),
        mentionUnreadChannelIds: new Set(["c"]),
      }),
    ),
    ["a", "c"],
  );
});
