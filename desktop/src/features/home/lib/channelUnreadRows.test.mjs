import assert from "node:assert/strict";
import { test } from "node:test";

import {
  buildChannelUnreadRows,
  withoutDuplicatedChannels,
} from "./channelUnreadRows.mjs";

/** Minimal Channel stand-in. */
function channel(id, name = `#${id}`) {
  return { id, name };
}

/** Build the input bag with sensible empty defaults. */
function input(overrides = {}) {
  return {
    channels: [],
    latestUnreadActivityByChannelId: new Map(),
    mutedChannelIds: new Set(),
    topLevelUnreadChannelIds: new Set(),
    unreadChannelCounts: new Map(),
    unreadThreadChannelIds: new Set(),
    ...overrides,
  };
}

test("a channel with top-level unread produces a row", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general")],
      topLevelUnreadChannelIds: new Set(["general"]),
      unreadChannelCounts: new Map([["general", 4]]),
      latestUnreadActivityByChannelId: new Map([["general", 1000]]),
    }),
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].channelId, "general");
  assert.equal(rows[0].unreadCount, 4);
  assert.equal(rows[0].sortAt, 1000);
  assert.equal(rows[0].kind, "channel");
});

test("a channel with only unread threads still produces a row", () => {
  // Thread activity is real activity — the user asked for everything new,
  // not only what landed in the main timeline.
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("design")],
      unreadThreadChannelIds: new Set(["design"]),
    }),
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].channelId, "design");
});

test("a channel unread at both levels produces exactly one row", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general")],
      topLevelUnreadChannelIds: new Set(["general"]),
      unreadThreadChannelIds: new Set(["general"]),
    }),
  );

  assert.equal(rows.length, 1);
});

test("muted channels are excluded even when unread", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("noisy"), channel("general")],
      topLevelUnreadChannelIds: new Set(["noisy", "general"]),
      mutedChannelIds: new Set(["noisy"]),
    }),
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].channelId, "general");
});

test("muting also suppresses a thread-only row", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("noisy")],
      unreadThreadChannelIds: new Set(["noisy"]),
      mutedChannelIds: new Set(["noisy"]),
    }),
  );

  assert.deepEqual(rows, []);
});

test("a missing count falls back to 1, never 0", () => {
  // The row exists because the channel is unread; reporting "0 new" on it
  // would contradict the reason it is on screen.
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general")],
      topLevelUnreadChannelIds: new Set(["general"]),
      unreadChannelCounts: new Map(),
    }),
  );

  assert.equal(rows[0].unreadCount, 1);
});

test("a zero count is treated as missing", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general")],
      topLevelUnreadChannelIds: new Set(["general"]),
      unreadChannelCounts: new Map([["general", 0]]),
    }),
  );

  assert.equal(rows[0].unreadCount, 1);
});

test("rows are ordered newest activity first", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("old"), channel("new"), channel("mid")],
      topLevelUnreadChannelIds: new Set(["old", "new", "mid"]),
      latestUnreadActivityByChannelId: new Map([
        ["old", 100],
        ["new", 300],
        ["mid", 200],
      ]),
    }),
  );

  assert.deepEqual(
    rows.map((row) => row.channelId),
    ["new", "mid", "old"],
  );
});

test("equal timestamps order stably rather than jittering", () => {
  const build = () =>
    buildChannelUnreadRows(
      input({
        channels: [channel("beta"), channel("alpha")],
        topLevelUnreadChannelIds: new Set(["beta", "alpha"]),
        latestUnreadActivityByChannelId: new Map([
          ["beta", 500],
          ["alpha", 500],
        ]),
      }),
    );

  assert.deepEqual(
    build().map((row) => row.channelId),
    ["alpha", "beta"],
  );
  assert.deepEqual(
    build().map((row) => row.channelId),
    build().map((row) => row.channelId),
  );
});

test("a channel with no activity timestamp sorts last rather than dropping", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("timed"), channel("untimed")],
      topLevelUnreadChannelIds: new Set(["timed", "untimed"]),
      latestUnreadActivityByChannelId: new Map([["timed", 100]]),
    }),
  );

  assert.equal(rows.length, 2);
  assert.equal(rows[1].channelId, "untimed");
  assert.equal(rows[1].sortAt, 0);
});

test("unread ids with no matching channel record are skipped", () => {
  // Happens transiently while the channel list is still loading; a nameless
  // row is worse than one that appears a moment later.
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general")],
      topLevelUnreadChannelIds: new Set(["general", "not-loaded-yet"]),
    }),
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].channelId, "general");
});

test("empty and undefined inputs produce no rows rather than throwing", () => {
  assert.deepEqual(buildChannelUnreadRows(input()), []);
  assert.deepEqual(
    buildChannelUnreadRows({
      channels: undefined,
      latestUnreadActivityByChannelId: undefined,
      mutedChannelIds: undefined,
      topLevelUnreadChannelIds: undefined,
      unreadChannelCounts: undefined,
      unreadThreadChannelIds: undefined,
    }),
    [],
  );
});

test("channels already carrying a feed row are dropped", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general"), channel("design")],
      topLevelUnreadChannelIds: new Set(["general", "design"]),
    }),
  );

  const deduped = withoutDuplicatedChannels(rows, new Set(["general"]));
  assert.equal(deduped.length, 1);
  assert.equal(deduped[0].channelId, "design");
});

test("dedupe is a no-op when nothing is occupied", () => {
  const rows = buildChannelUnreadRows(
    input({
      channels: [channel("general")],
      topLevelUnreadChannelIds: new Set(["general"]),
    }),
  );

  assert.equal(withoutDuplicatedChannels(rows, new Set()).length, 1);
  assert.equal(withoutDuplicatedChannels(rows, undefined).length, 1);
});
