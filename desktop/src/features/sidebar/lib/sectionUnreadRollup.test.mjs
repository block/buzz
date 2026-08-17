import assert from "node:assert/strict";
import { test } from "node:test";

import {
  formatSectionUnreadCount,
  rollUpSectionUnread,
} from "./sectionUnreadRollup.mjs";

/** Build the input bag with empty defaults. */
function input(overrides = {}) {
  return {
    channelIds: [],
    highPriorityUnreadChannelIds: new Set(),
    mutedChannelIds: new Set(),
    topLevelUnreadChannelIds: new Set(),
    unreadChannelCounts: new Map(),
    unreadThreadChannelIds: new Set(),
    ...overrides,
  };
}

test("a quiet section shows nothing", () => {
  assert.deepEqual(rollUpSectionUnread(input({ channelIds: ["a", "b"] })), {
    kind: "none",
  });
});

test("ordinary top-level activity shows a dot", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["a"],
        topLevelUnreadChannelIds: new Set(["a"]),
      }),
    ),
    { kind: "dot" },
  );
});

test("thread-only activity also shows a dot", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["a"],
        unreadThreadChannelIds: new Set(["a"]),
      }),
    ),
    { kind: "dot" },
  );
});

test("a mention or DM shows a count", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["a"],
        highPriorityUnreadChannelIds: new Set(["a"]),
        unreadChannelCounts: new Map([["a", 3]]),
      }),
    ),
    { kind: "count", count: 3 },
  );
});

test("counts sum across the section's channels", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["a", "b"],
        highPriorityUnreadChannelIds: new Set(["a", "b"]),
        unreadChannelCounts: new Map([
          ["a", 3],
          ["b", 4],
        ]),
      }),
    ),
    { kind: "count", count: 7 },
  );
});

test("urgent wins when a section holds both kinds", () => {
  // Reducing a section containing a mention to the same dot as routine
  // chatter would defeat the point of the two tiers.
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["quiet", "urgent"],
        topLevelUnreadChannelIds: new Set(["quiet"]),
        highPriorityUnreadChannelIds: new Set(["urgent"]),
        unreadChannelCounts: new Map([["urgent", 2]]),
      }),
    ),
    { kind: "count", count: 2 },
  );
});

test("a high-priority channel with no count still contributes 1", () => {
  // Being in the unread set is the authority on whether something is waiting;
  // a missing count means unsized, not zero.
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["a"],
        highPriorityUnreadChannelIds: new Set(["a"]),
        unreadChannelCounts: new Map(),
      }),
    ),
    { kind: "count", count: 1 },
  );
});

test("a zero count is treated as unsized rather than empty", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["a"],
        highPriorityUnreadChannelIds: new Set(["a"]),
        unreadChannelCounts: new Map([["a", 0]]),
      }),
    ),
    { kind: "count", count: 1 },
  );
});

test("muted channels contribute nothing, at either tier", () => {
  // Muting means "stop telling me"; a section badge that ignored it would
  // reintroduce the noise one level up, harder to trace.
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["noisy"],
        mutedChannelIds: new Set(["noisy"]),
        topLevelUnreadChannelIds: new Set(["noisy"]),
        highPriorityUnreadChannelIds: new Set(["noisy"]),
        unreadChannelCounts: new Map([["noisy", 9]]),
      }),
    ),
    { kind: "none" },
  );
});

test("muting one channel does not silence its neighbours", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["noisy", "real"],
        mutedChannelIds: new Set(["noisy"]),
        highPriorityUnreadChannelIds: new Set(["noisy", "real"]),
        unreadChannelCounts: new Map([
          ["noisy", 9],
          ["real", 2],
        ]),
      }),
    ),
    { kind: "count", count: 2 },
  );
});

test("channels outside the section are ignored", () => {
  assert.deepEqual(
    rollUpSectionUnread(
      input({
        channelIds: ["mine"],
        topLevelUnreadChannelIds: new Set(["someone-elses"]),
      }),
    ),
    { kind: "none" },
  );
});

test("empty and undefined inputs are quiet rather than throwing", () => {
  assert.deepEqual(rollUpSectionUnread(input()), { kind: "none" });
  assert.deepEqual(
    rollUpSectionUnread({
      channelIds: undefined,
      highPriorityUnreadChannelIds: undefined,
      mutedChannelIds: undefined,
      topLevelUnreadChannelIds: undefined,
      unreadChannelCounts: undefined,
      unreadThreadChannelIds: undefined,
    }),
    { kind: "none" },
  );
});

test("the count caps the same way row badges do", () => {
  assert.equal(formatSectionUnreadCount(1), "1");
  assert.equal(formatSectionUnreadCount(99), "99");
  assert.equal(formatSectionUnreadCount(100), "99+");
  assert.equal(formatSectionUnreadCount(4_000), "99+");
});
