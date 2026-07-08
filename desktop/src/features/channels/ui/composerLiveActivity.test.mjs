import assert from "node:assert/strict";
import test from "node:test";

import {
  groupLiveActivity,
  mergeLiveActivity,
} from "./composerLiveActivity.ts";

const CHANNEL = "channel-1";
const OTHER_CHANNEL = "channel-2";

const alice = { pubkey: "alice-pubkey", name: "Alice" };
const bob = { pubkey: "bob-pubkey", name: "Bob" };

function makeItem(overrides = {}) {
  return {
    id: "item:1",
    type: "tool",
    title: "Read file",
    toolName: "read_file",
    buzzToolName: "read_file",
    status: "completed",
    args: {},
    result: "",
    isError: false,
    renderClass: "tool-run",
    channelId: CHANNEL,
    timestamp: "2026-07-07T00:00:00.000Z",
    startedAt: "2026-07-07T00:00:00.000Z",
    completedAt: null,
    ...overrides,
  };
}

test("mergeLiveActivity interleaves per-agent transcripts chronologically", () => {
  const merged = mergeLiveActivity(
    [alice, bob],
    [
      [
        makeItem({ id: "a1", timestamp: "2026-07-07T00:00:01.000Z" }),
        makeItem({ id: "a2", timestamp: "2026-07-07T00:00:04.000Z" }),
      ],
      [
        makeItem({ id: "b1", timestamp: "2026-07-07T00:00:02.000Z" }),
        makeItem({ id: "b2", timestamp: "2026-07-07T00:00:03.000Z" }),
      ],
    ],
    CHANNEL,
  );

  assert.deepEqual(
    merged.map((entry) => entry.key),
    ["alice-pubkey:a1", "bob-pubkey:b1", "bob-pubkey:b2", "alice-pubkey:a2"],
  );
});

test("mergeLiveActivity scopes to the requested channel", () => {
  const merged = mergeLiveActivity(
    [alice],
    [
      [
        makeItem({ id: "in-scope" }),
        makeItem({ id: "out-of-scope", channelId: OTHER_CHANNEL }),
      ],
    ],
    CHANNEL,
  );

  assert.deepEqual(
    merged.map((entry) => entry.item.id),
    ["in-scope"],
  );
});

test("mergeLiveActivity drops suppressed tool items", () => {
  const merged = mergeLiveActivity(
    [alice],
    [
      [
        makeItem({ id: "visible" }),
        makeItem({ id: "hidden", renderClass: "suppressed" }),
      ],
    ],
    CHANNEL,
  );

  assert.deepEqual(
    merged.map((entry) => entry.item.id),
    ["visible"],
  );
});

test("mergeLiveActivity keeps ties in agent order (stable merge)", () => {
  const timestamp = "2026-07-07T00:00:05.000Z";
  const merged = mergeLiveActivity(
    [alice, bob],
    [[makeItem({ id: "a1", timestamp })], [makeItem({ id: "b1", timestamp })]],
    CHANNEL,
  );

  assert.deepEqual(
    merged.map((entry) => entry.agent.pubkey),
    ["alice-pubkey", "bob-pubkey"],
  );
});

test("mergeLiveActivity caps the merged stream at the preview limit", () => {
  const items = Array.from({ length: 120 }, (_, index) =>
    makeItem({
      id: `a${index}`,
      timestamp: new Date(Date.UTC(2026, 6, 7, 0, 0, index)).toISOString(),
    }),
  );
  const merged = mergeLiveActivity([alice], [items], CHANNEL);

  assert.equal(merged.length, 80);
  assert.equal(merged[0].item.id, "a40");
  assert.equal(merged.at(-1).item.id, "a119");
});

test("groupLiveActivity groups consecutive same-agent runs", () => {
  const merged = mergeLiveActivity(
    [alice, bob],
    [
      [
        makeItem({ id: "a1", timestamp: "2026-07-07T00:00:01.000Z" }),
        makeItem({ id: "a2", timestamp: "2026-07-07T00:00:02.000Z" }),
        makeItem({ id: "a3", timestamp: "2026-07-07T00:00:09.000Z" }),
      ],
      [makeItem({ id: "b1", timestamp: "2026-07-07T00:00:05.000Z" })],
    ],
    CHANNEL,
  );

  const groups = groupLiveActivity(merged);
  assert.deepEqual(
    groups.map((group) => ({
      agent: group.agent.pubkey,
      ids: group.entries.map((entry) => entry.item.id),
    })),
    [
      { agent: "alice-pubkey", ids: ["a1", "a2"] },
      { agent: "bob-pubkey", ids: ["b1"] },
      { agent: "alice-pubkey", ids: ["a3"] },
    ],
  );
});
