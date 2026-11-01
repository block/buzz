import assert from "node:assert/strict";
import test from "node:test";

import {
  effectiveChannelFilter,
  filterByChannel,
  threadChannelOptions,
} from "./threadChannelFilter.ts";

const LABELS = { c1: "#general", c2: "#buzz-dev", c3: "Alice" };
const labelFor = (id) => LABELS[id] ?? "Unknown channel";

const entries = [
  { channelId: "c1", rootId: "r1" },
  { channelId: "c2", rootId: "r2" },
  { channelId: "c1", rootId: "r3" },
  { channelId: "c3", rootId: "r4" },
];

test("threadChannelOptions counts threads per channel, sorted by label", () => {
  assert.deepEqual(threadChannelOptions(entries, labelFor), [
    { channelId: "c2", count: 1, label: "#buzz-dev" },
    { channelId: "c1", count: 2, label: "#general" },
    { channelId: "c3", count: 1, label: "Alice" },
  ]);
});

test("threadChannelOptions is empty with no threads", () => {
  assert.deepEqual(threadChannelOptions([], labelFor), []);
});

test("threadChannelOptions breaks label ties by channel id", () => {
  const options = threadChannelOptions(
    [{ channelId: "z" }, { channelId: "a" }],
    () => "Unknown channel",
  );
  assert.deepEqual(
    options.map((option) => option.channelId),
    ["a", "z"],
  );
});

test("filterByChannel keeps order and only the selected channel", () => {
  assert.deepEqual(
    filterByChannel(entries, "c1").map((entry) => entry.rootId),
    ["r1", "r3"],
  );
  assert.deepEqual(filterByChannel(entries, null), entries);
});

test("effectiveChannelFilter drops a channel that no longer has threads", () => {
  const options = threadChannelOptions(entries, labelFor);
  assert.equal(effectiveChannelFilter("c2", options), "c2");
  assert.equal(effectiveChannelFilter("gone", options), null);
  assert.equal(effectiveChannelFilter(null, options), null);
});
